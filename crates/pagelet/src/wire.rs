//! Versioned cross-language DTOs and the `pageletScene` binary protocol.
//!
//! Wire schema versions are intentionally independent from the pagelet crate's
//! semantic version. All multi-byte integers use little-endian byte order.

use std::{fmt, sync::Arc};

use crate::{
    core::{
        ContentHash, Diagnostic, DiagnosticCode, DocumentId, FontId, LayoutUnit, NodeId,
        ResourceId, Severity, SourceRange, TextAffinity, TextAnchor,
    },
    document::LinkKind,
    layout::{
        AnchorRegion, BreakToken, LinkRegion, PageFingerprint, PageScene, PageSize, Rect,
        SceneFragment, SceneFragmentKind, SelectionMap, SemanticNode, TextAnchorRange,
    },
    text::{
        FontDescriptor, FontFallbackChain, FontSetFingerprint, FontStyle, HeightBehavior,
        LineMetrics, MeasureBatch as TextMeasureBatch, MeasureRequest,
        MeasuredBatch as TextMeasuredBatch, MeasuredText, StrutStyle, TextBackendId, TextCluster,
        TextDirection, TextStyleRun,
    },
};

const MAGIC: [u8; 8] = *b"PGLTSCN\0";
const HEADER_LEN: usize = 20;
const MAX_PAYLOAD_BYTES: usize = 64 * 1024 * 1024;
const MAX_COLLECTION_ITEMS: usize = 1_000_000;
const MAX_STRING_BYTES: usize = 8 * 1024 * 1024;

/// Current `pageletScene` schema version.
///
/// This value is advanced only when the binary schema changes and is not
/// derived from `CARGO_PKG_VERSION`.
pub const CURRENT_SCHEMA_VERSION: SchemaVersion = SchemaVersion::new(1);

/// Version of the cross-language binary schema.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct SchemaVersion(u16);

impl SchemaVersion {
    /// Create a schema version from its wire value.
    #[must_use]
    pub const fn new(value: u16) -> Self {
        Self(value)
    }

    /// Return the wire value.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

/// Versioned batch of laid-out pages.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PageBatch {
    /// Binary schema version used by this DTO.
    pub schema_version: SchemaVersion,
    /// Pages in host delivery order.
    pub pages: Vec<PageScene>,
}

impl PageBatch {
    /// Create a batch using the current schema version.
    #[must_use]
    pub fn new(pages: Vec<PageScene>) -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            pages,
        }
    }

    /// Encode the batch as a canonical little-endian `pageletScene` payload.
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        require_current_version(self.schema_version)?;
        let mut writer = Writer::default();
        writer.write_collection_len("pages", self.pages.len())?;
        for page in &self.pages {
            write_page_scene(&mut writer, page)?;
        }
        encode_envelope(PayloadKind::Page, self.schema_version, writer.finish())
    }

    /// Decode and validate a `pageletScene` page batch.
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let (schema_version, payload) = decode_envelope(bytes, PayloadKind::Page)?;
        let mut reader = Reader::new(payload);
        let page_count = reader.read_collection_len("pages")?;
        let mut pages = Vec::new();
        for _ in 0..page_count {
            pages.push(read_page_scene(&mut reader)?);
        }
        reader.finish()?;
        Ok(Self {
            schema_version,
            pages,
        })
    }
}

/// Versioned host text-measurement request batch.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct MeasureBatch {
    /// Binary schema version used by this DTO.
    pub schema_version: SchemaVersion,
    /// Requests in stable layout order.
    pub requests: Vec<MeasureRequest>,
}

impl MeasureBatch {
    /// Create a batch using the current schema version.
    #[must_use]
    pub fn new(requests: Vec<MeasureRequest>) -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            requests,
        }
    }

    /// Encode the batch as a canonical little-endian payload.
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        require_current_version(self.schema_version)?;
        let mut writer = Writer::default();
        writer.write_collection_len("measure requests", self.requests.len())?;
        for request in &self.requests {
            write_measure_request(&mut writer, request)?;
        }
        encode_envelope(PayloadKind::Measure, self.schema_version, writer.finish())
    }

    /// Decode and validate a host text-measurement request batch.
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let (schema_version, payload) = decode_envelope(bytes, PayloadKind::Measure)?;
        let mut reader = Reader::new(payload);
        let request_count = reader.read_collection_len("measure requests")?;
        let mut requests = Vec::new();
        for _ in 0..request_count {
            requests.push(read_measure_request(&mut reader)?);
        }
        reader.finish()?;
        Ok(Self {
            schema_version,
            requests,
        })
    }

    /// Convert to the layout text backend batch.
    #[must_use]
    pub fn into_text_batch(self) -> TextMeasureBatch {
        TextMeasureBatch::new(self.requests)
    }
}

impl From<TextMeasureBatch> for MeasureBatch {
    fn from(value: TextMeasureBatch) -> Self {
        Self::new(value.requests)
    }
}

/// Versioned host text-measurement response batch.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct MeasuredBatch {
    /// Binary schema version used by this DTO.
    pub schema_version: SchemaVersion,
    /// Stable identity of the host backend that produced the results.
    pub backend_id: TextBackendId,
    /// Stable fingerprint of the font set used for measurement.
    pub font_fingerprint: FontSetFingerprint,
    /// Results in request order.
    pub results: Vec<MeasuredText>,
}

impl MeasuredBatch {
    /// Create a batch using the current schema version.
    #[must_use]
    pub fn new(
        backend_id: TextBackendId,
        font_fingerprint: FontSetFingerprint,
        results: Vec<MeasuredText>,
    ) -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            backend_id,
            font_fingerprint,
            results,
        }
    }

    /// Encode the batch as a canonical little-endian payload.
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        require_current_version(self.schema_version)?;
        let mut writer = Writer::default();
        writer.write_u64(self.backend_id.0);
        writer.write_u64(self.font_fingerprint.0);
        writer.write_collection_len("measured results", self.results.len())?;
        for result in &self.results {
            write_measured_text(&mut writer, result)?;
        }
        encode_envelope(PayloadKind::Measured, self.schema_version, writer.finish())
    }

    /// Decode and validate a host text-measurement response batch.
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let (schema_version, payload) = decode_envelope(bytes, PayloadKind::Measured)?;
        let mut reader = Reader::new(payload);
        let backend_id = TextBackendId(reader.read_u64("text backend id")?);
        let font_fingerprint = FontSetFingerprint(reader.read_u64("text font fingerprint")?);
        let result_count = reader.read_collection_len("measured results")?;
        let mut results = Vec::new();
        for _ in 0..result_count {
            results.push(read_measured_text(&mut reader)?);
        }
        reader.finish()?;
        Ok(Self {
            schema_version,
            backend_id,
            font_fingerprint,
            results,
        })
    }

    /// Convert to the layout text backend response batch.
    #[must_use]
    pub fn into_text_batch(self) -> TextMeasuredBatch {
        TextMeasuredBatch::new(self.backend_id, self.font_fingerprint, self.results)
    }
}

impl From<TextMeasuredBatch> for MeasuredBatch {
    fn from(value: TextMeasuredBatch) -> Self {
        Self::new(value.backend_id, value.font_fingerprint, value.results)
    }
}

/// Stable validation failure returned by wire encoders and decoders.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum WireError {
    /// Payload is shorter than the fixed envelope header.
    PayloadTooShort { minimum: usize, actual: usize },
    /// Envelope magic does not identify a pageletScene payload.
    InvalidMagic,
    /// Schema version is not supported by this decoder.
    UnsupportedVersion { expected: u16, actual: u16 },
    /// Envelope contains a different payload kind than requested.
    UnexpectedPayloadKind { expected: u16, actual: u16 },
    /// Payload exceeds the protocol allocation limit.
    PayloadTooLarge { limit: usize, actual: usize },
    /// A length cannot be represented by the wire schema.
    LengthOverflow { field: &'static str, actual: usize },
    /// Declared payload length does not match the provided buffer.
    LengthMismatch { declared: usize, actual: usize },
    /// Payload checksum does not match the envelope.
    ChecksumMismatch { expected: u32, actual: u32 },
    /// A collection count exceeds the decoder allocation limit.
    CollectionTooLarge {
        field: &'static str,
        limit: usize,
        actual: usize,
    },
    /// A string exceeds the decoder allocation limit.
    StringTooLarge {
        field: &'static str,
        limit: usize,
        actual: usize,
    },
    /// Payload ended before a field could be decoded.
    UnexpectedEnd { field: &'static str },
    /// String bytes are not valid UTF-8.
    InvalidUtf8 { field: &'static str },
    /// A one-byte boolean contained a value other than zero or one.
    InvalidBoolean { field: &'static str, actual: u8 },
    /// An enum discriminant is not defined by this schema version.
    InvalidEnum { field: &'static str, actual: u8 },
    /// A byte range is reversed, out of bounds, or not on UTF-8 boundaries.
    InvalidRange { field: &'static str },
    /// Valid fields were followed by unexpected bytes.
    TrailingBytes { remaining: usize },
}

impl fmt::Display for WireError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PayloadTooShort { minimum, actual } => {
                write!(
                    formatter,
                    "wire payload needs at least {minimum} bytes, got {actual}"
                )
            }
            Self::InvalidMagic => formatter.write_str("wire payload has invalid magic"),
            Self::UnsupportedVersion { expected, actual } => write!(
                formatter,
                "unsupported wire schema version {actual}; expected {expected}"
            ),
            Self::UnexpectedPayloadKind { expected, actual } => write!(
                formatter,
                "unexpected wire payload kind {actual}; expected {expected}"
            ),
            Self::PayloadTooLarge { limit, actual } => {
                write!(
                    formatter,
                    "wire payload size {actual} exceeds limit {limit}"
                )
            }
            Self::LengthOverflow { field, actual } => {
                write!(
                    formatter,
                    "wire field {field} length {actual} cannot be encoded"
                )
            }
            Self::LengthMismatch { declared, actual } => write!(
                formatter,
                "wire payload declares {declared} bytes but contains {actual}"
            ),
            Self::ChecksumMismatch { expected, actual } => write!(
                formatter,
                "wire checksum mismatch: expected {expected:08x}, actual {actual:08x}"
            ),
            Self::CollectionTooLarge {
                field,
                limit,
                actual,
            } => write!(
                formatter,
                "wire collection {field} count {actual} exceeds limit {limit}"
            ),
            Self::StringTooLarge {
                field,
                limit,
                actual,
            } => write!(
                formatter,
                "wire string {field} length {actual} exceeds limit {limit}"
            ),
            Self::UnexpectedEnd { field } => {
                write!(formatter, "wire payload ended while reading {field}")
            }
            Self::InvalidUtf8 { field } => write!(formatter, "wire field {field} is not UTF-8"),
            Self::InvalidBoolean { field, actual } => {
                write!(formatter, "wire field {field} has invalid boolean {actual}")
            }
            Self::InvalidEnum { field, actual } => {
                write!(
                    formatter,
                    "wire field {field} has unknown discriminant {actual}"
                )
            }
            Self::InvalidRange { field } => {
                write!(formatter, "wire field {field} contains an invalid range")
            }
            Self::TrailingBytes { remaining } => {
                write!(formatter, "wire payload has {remaining} trailing bytes")
            }
        }
    }
}

impl std::error::Error for WireError {}

#[repr(u16)]
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum PayloadKind {
    Page = 1,
    Measure = 2,
    Measured = 3,
}

impl PayloadKind {
    const fn get(self) -> u16 {
        self as u16
    }
}

fn require_current_version(version: SchemaVersion) -> Result<(), WireError> {
    if version == CURRENT_SCHEMA_VERSION {
        Ok(())
    } else {
        Err(WireError::UnsupportedVersion {
            expected: CURRENT_SCHEMA_VERSION.get(),
            actual: version.get(),
        })
    }
}

fn encode_envelope(
    kind: PayloadKind,
    schema_version: SchemaVersion,
    payload: Vec<u8>,
) -> Result<Vec<u8>, WireError> {
    require_current_version(schema_version)?;
    if payload.len() > MAX_PAYLOAD_BYTES {
        return Err(WireError::PayloadTooLarge {
            limit: MAX_PAYLOAD_BYTES,
            actual: payload.len(),
        });
    }
    let payload_len = u32::try_from(payload.len()).map_err(|_| WireError::LengthOverflow {
        field: "payload",
        actual: payload.len(),
    })?;
    let checksum = crc32(&payload);
    let mut bytes = Vec::with_capacity(HEADER_LEN + payload.len());
    bytes.extend_from_slice(&MAGIC);
    bytes.extend_from_slice(&schema_version.get().to_le_bytes());
    bytes.extend_from_slice(&kind.get().to_le_bytes());
    bytes.extend_from_slice(&payload_len.to_le_bytes());
    bytes.extend_from_slice(&checksum.to_le_bytes());
    bytes.extend_from_slice(&payload);
    Ok(bytes)
}

fn decode_envelope(
    bytes: &[u8],
    expected_kind: PayloadKind,
) -> Result<(SchemaVersion, &[u8]), WireError> {
    if bytes.len() < HEADER_LEN {
        return Err(WireError::PayloadTooShort {
            minimum: HEADER_LEN,
            actual: bytes.len(),
        });
    }
    if bytes[..MAGIC.len()] != MAGIC {
        return Err(WireError::InvalidMagic);
    }
    let schema_version = SchemaVersion::new(u16::from_le_bytes([bytes[8], bytes[9]]));
    require_current_version(schema_version)?;
    let actual_kind = u16::from_le_bytes([bytes[10], bytes[11]]);
    if actual_kind != expected_kind.get() {
        return Err(WireError::UnexpectedPayloadKind {
            expected: expected_kind.get(),
            actual: actual_kind,
        });
    }
    let declared = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;
    if declared > MAX_PAYLOAD_BYTES {
        return Err(WireError::PayloadTooLarge {
            limit: MAX_PAYLOAD_BYTES,
            actual: declared,
        });
    }
    let payload = &bytes[HEADER_LEN..];
    if declared != payload.len() {
        return Err(WireError::LengthMismatch {
            declared,
            actual: payload.len(),
        });
    }
    let expected_checksum = u32::from_le_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let actual_checksum = crc32(payload);
    if expected_checksum != actual_checksum {
        return Err(WireError::ChecksumMismatch {
            expected: expected_checksum,
            actual: actual_checksum,
        });
    }
    Ok((schema_version, payload))
}

#[derive(Default)]
struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    fn finish(self) -> Vec<u8> {
        self.bytes
    }

    fn write_u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    fn write_u16(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn write_u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn write_u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn write_i64(&mut self, value: i64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn write_bool(&mut self, value: bool) {
        self.write_u8(u8::from(value));
    }

    fn write_layout_unit(&mut self, value: LayoutUnit) {
        self.write_i64(value.raw());
    }

    fn write_collection_len(
        &mut self,
        field: &'static str,
        actual: usize,
    ) -> Result<(), WireError> {
        if actual > MAX_COLLECTION_ITEMS {
            return Err(WireError::CollectionTooLarge {
                field,
                limit: MAX_COLLECTION_ITEMS,
                actual,
            });
        }
        let value =
            u32::try_from(actual).map_err(|_| WireError::LengthOverflow { field, actual })?;
        self.write_u32(value);
        Ok(())
    }

    fn write_string(&mut self, field: &'static str, value: &str) -> Result<(), WireError> {
        if value.len() > MAX_STRING_BYTES {
            return Err(WireError::StringTooLarge {
                field,
                limit: MAX_STRING_BYTES,
                actual: value.len(),
            });
        }
        let length = u32::try_from(value.len()).map_err(|_| WireError::LengthOverflow {
            field,
            actual: value.len(),
        })?;
        self.write_u32(length);
        self.bytes.extend_from_slice(value.as_bytes());
        Ok(())
    }

    fn write_hash(&mut self, value: ContentHash) {
        self.bytes.extend_from_slice(value.as_bytes());
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn finish(self) -> Result<(), WireError> {
        let remaining = self.bytes.len().saturating_sub(self.offset);
        if remaining == 0 {
            Ok(())
        } else {
            Err(WireError::TrailingBytes { remaining })
        }
    }

    fn read_exact<const N: usize>(&mut self, field: &'static str) -> Result<[u8; N], WireError> {
        let end = self
            .offset
            .checked_add(N)
            .ok_or(WireError::UnexpectedEnd { field })?;
        let source = self
            .bytes
            .get(self.offset..end)
            .ok_or(WireError::UnexpectedEnd { field })?;
        let mut value = [0_u8; N];
        value.copy_from_slice(source);
        self.offset = end;
        Ok(value)
    }

    fn read_u8(&mut self, field: &'static str) -> Result<u8, WireError> {
        Ok(self.read_exact::<1>(field)?[0])
    }

    fn read_u16(&mut self, field: &'static str) -> Result<u16, WireError> {
        Ok(u16::from_le_bytes(self.read_exact(field)?))
    }

    fn read_u32(&mut self, field: &'static str) -> Result<u32, WireError> {
        Ok(u32::from_le_bytes(self.read_exact(field)?))
    }

    fn read_u64(&mut self, field: &'static str) -> Result<u64, WireError> {
        Ok(u64::from_le_bytes(self.read_exact(field)?))
    }

    fn read_i64(&mut self, field: &'static str) -> Result<i64, WireError> {
        Ok(i64::from_le_bytes(self.read_exact(field)?))
    }

    fn read_bool(&mut self, field: &'static str) -> Result<bool, WireError> {
        match self.read_u8(field)? {
            0 => Ok(false),
            1 => Ok(true),
            actual => Err(WireError::InvalidBoolean { field, actual }),
        }
    }

    fn read_layout_unit(&mut self, field: &'static str) -> Result<LayoutUnit, WireError> {
        Ok(LayoutUnit::from_raw(self.read_i64(field)?))
    }

    fn read_collection_len(&mut self, field: &'static str) -> Result<usize, WireError> {
        let actual = self.read_u32(field)? as usize;
        if actual > MAX_COLLECTION_ITEMS {
            Err(WireError::CollectionTooLarge {
                field,
                limit: MAX_COLLECTION_ITEMS,
                actual,
            })
        } else {
            Ok(actual)
        }
    }

    fn read_string(&mut self, field: &'static str) -> Result<Arc<str>, WireError> {
        let length = self.read_u32(field)? as usize;
        if length > MAX_STRING_BYTES {
            return Err(WireError::StringTooLarge {
                field,
                limit: MAX_STRING_BYTES,
                actual: length,
            });
        }
        let end = self
            .offset
            .checked_add(length)
            .ok_or(WireError::UnexpectedEnd { field })?;
        let bytes = self
            .bytes
            .get(self.offset..end)
            .ok_or(WireError::UnexpectedEnd { field })?;
        let value = std::str::from_utf8(bytes).map_err(|_| WireError::InvalidUtf8 { field })?;
        self.offset = end;
        Ok(Arc::from(value))
    }

    fn read_hash(&mut self, field: &'static str) -> Result<ContentHash, WireError> {
        Ok(ContentHash::from_array(self.read_exact(field)?))
    }
}

fn write_measure_request(writer: &mut Writer, request: &MeasureRequest) -> Result<(), WireError> {
    validate_request_ranges(request)?;
    writer.write_u32(request.id);
    writer.write_u32(request.paragraph_id);
    writer.write_string("measure text", &request.text)?;
    writer.write_u32(request.text_range.start);
    writer.write_u32(request.text_range.end);
    writer.write_collection_len("text style runs", request.style_runs.len())?;
    for style_run in &request.style_runs {
        write_text_style_run(writer, style_run)?;
    }
    writer.write_layout_unit(request.font_size);
    writer.write_layout_unit(request.max_width);
    writer.write_layout_unit(request.available_width);
    writer.write_string("measure locale", &request.locale)?;
    writer.write_u8(text_direction_to_wire(request.direction));
    writer.write_layout_unit(request.text_scale);
    write_font_fallback_chain(writer, &request.font_candidates)?;
    writer.write_layout_unit(request.strut.ascent);
    writer.write_layout_unit(request.strut.descent);
    writer.write_layout_unit(request.strut.leading);
    writer.write_u8(height_behavior_to_wire(request.height_behavior));
    writer.write_u64(request.request_fingerprint);
    Ok(())
}

fn read_measure_request(reader: &mut Reader<'_>) -> Result<MeasureRequest, WireError> {
    let id = reader.read_u32("measure request id")?;
    let paragraph_id = reader.read_u32("paragraph id")?;
    let text = reader.read_string("measure text")?;
    let text_start = reader.read_u32("measure text range start")?;
    let text_end = reader.read_u32("measure text range end")?;
    validate_utf8_range("measure text range", &text, text_start, text_end)?;
    let style_run_count = reader.read_collection_len("text style runs")?;
    let mut style_runs = Vec::new();
    for _ in 0..style_run_count {
        let style_run = read_text_style_run(reader)?;
        validate_utf8_range("text style run", &text, style_run.start, style_run.end)?;
        if style_run.start < text_start || style_run.end > text_end {
            return Err(WireError::InvalidRange {
                field: "text style run",
            });
        }
        style_runs.push(style_run);
    }
    Ok(MeasureRequest {
        id,
        paragraph_id,
        text,
        text_range: text_start..text_end,
        style_runs,
        font_size: reader.read_layout_unit("font size")?,
        max_width: reader.read_layout_unit("maximum width")?,
        available_width: reader.read_layout_unit("available width")?,
        locale: reader.read_string("measure locale")?,
        direction: text_direction_from_wire(reader.read_u8("text direction")?)?,
        text_scale: reader.read_layout_unit("text scale")?,
        font_candidates: read_font_fallback_chain(reader)?,
        strut: StrutStyle {
            ascent: reader.read_layout_unit("strut ascent")?,
            descent: reader.read_layout_unit("strut descent")?,
            leading: reader.read_layout_unit("strut leading")?,
        },
        height_behavior: height_behavior_from_wire(reader.read_u8("height behavior")?)?,
        request_fingerprint: reader.read_u64("request fingerprint")?,
    })
}

fn write_measured_text(writer: &mut Writer, measured: &MeasuredText) -> Result<(), WireError> {
    if measured.line_count != u32::try_from(measured.lines.len()).unwrap_or(u32::MAX) {
        return Err(WireError::LengthMismatch {
            declared: usize::try_from(measured.line_count).unwrap_or(usize::MAX),
            actual: measured.lines.len(),
        });
    }
    writer.write_u32(measured.request_id);
    writer.write_u64(measured.request_fingerprint);
    writer.write_layout_unit(measured.width);
    writer.write_layout_unit(measured.height);
    writer.write_u32(measured.line_count);
    writer.write_u32(measured.utf8_len);
    writer.write_collection_len("measured lines", measured.lines.len())?;
    for line in &measured.lines {
        writer.write_u32(line.text_start);
        writer.write_u32(line.text_end);
        writer.write_layout_unit(line.baseline);
        writer.write_layout_unit(line.ascent);
        writer.write_layout_unit(line.descent);
        writer.write_layout_unit(line.line_height);
        writer.write_layout_unit(line.width);
        writer.write_bool(line.hard_break);
    }
    writer.write_collection_len("measured clusters", measured.clusters.len())?;
    for cluster in &measured.clusters {
        writer.write_u32(cluster.text_start);
        writer.write_u32(cluster.text_end);
        writer.write_u32(cluster.line_index);
        writer.write_layout_unit(cluster.x_start);
        writer.write_layout_unit(cluster.x_end);
    }
    writer.write_u64(measured.measurement_fingerprint);
    Ok(())
}

fn read_measured_text(reader: &mut Reader<'_>) -> Result<MeasuredText, WireError> {
    let request_id = reader.read_u32("measured request id")?;
    let request_fingerprint = reader.read_u64("measured request fingerprint")?;
    let width = reader.read_layout_unit("measured width")?;
    let height = reader.read_layout_unit("measured height")?;
    let line_count = reader.read_u32("measured line count")?;
    let utf8_len = reader.read_u32("measured utf8 length")?;
    let encoded_line_count = reader.read_collection_len("measured lines")?;
    if usize::try_from(line_count).unwrap_or(usize::MAX) != encoded_line_count {
        return Err(WireError::LengthMismatch {
            declared: usize::try_from(line_count).unwrap_or(usize::MAX),
            actual: encoded_line_count,
        });
    }
    let mut lines = Vec::new();
    for _ in 0..encoded_line_count {
        lines.push(LineMetrics {
            text_start: reader.read_u32("measured line start")?,
            text_end: reader.read_u32("measured line end")?,
            baseline: reader.read_layout_unit("measured line baseline")?,
            ascent: reader.read_layout_unit("measured line ascent")?,
            descent: reader.read_layout_unit("measured line descent")?,
            line_height: reader.read_layout_unit("measured line height")?,
            width: reader.read_layout_unit("measured line width")?,
            hard_break: reader.read_bool("measured line hard break")?,
        });
    }
    let cluster_count = reader.read_collection_len("measured clusters")?;
    let mut clusters = Vec::new();
    for _ in 0..cluster_count {
        clusters.push(TextCluster {
            text_start: reader.read_u32("measured cluster start")?,
            text_end: reader.read_u32("measured cluster end")?,
            line_index: reader.read_u32("measured cluster line")?,
            x_start: reader.read_layout_unit("measured cluster x start")?,
            x_end: reader.read_layout_unit("measured cluster x end")?,
        });
    }
    Ok(MeasuredText {
        request_id,
        request_fingerprint,
        width,
        height,
        line_count,
        utf8_len,
        lines,
        clusters,
        measurement_fingerprint: reader.read_u64("measurement fingerprint")?,
    })
}

fn write_text_style_run(writer: &mut Writer, style_run: &TextStyleRun) -> Result<(), WireError> {
    if style_run.start > style_run.end {
        return Err(WireError::InvalidRange {
            field: "text style run",
        });
    }
    writer.write_u32(style_run.start);
    writer.write_u32(style_run.end);
    writer.write_layout_unit(style_run.font_size);
    write_font_fallback_chain(writer, &style_run.fonts)
}

fn read_text_style_run(reader: &mut Reader<'_>) -> Result<TextStyleRun, WireError> {
    let start = reader.read_u32("text style run start")?;
    let end = reader.read_u32("text style run end")?;
    if start > end {
        return Err(WireError::InvalidRange {
            field: "text style run",
        });
    }
    Ok(TextStyleRun {
        start,
        end,
        font_size: reader.read_layout_unit("text style font size")?,
        fonts: read_font_fallback_chain(reader)?,
    })
}

fn write_font_fallback_chain(
    writer: &mut Writer,
    chain: &FontFallbackChain,
) -> Result<(), WireError> {
    write_font_descriptor(writer, &chain.primary)?;
    writer.write_collection_len("font fallbacks", chain.fallbacks.len())?;
    for fallback in &chain.fallbacks {
        write_font_descriptor(writer, fallback)?;
    }
    Ok(())
}

fn read_font_fallback_chain(reader: &mut Reader<'_>) -> Result<FontFallbackChain, WireError> {
    let primary = read_font_descriptor(reader)?;
    let fallback_count = reader.read_collection_len("font fallbacks")?;
    let mut fallbacks = Vec::new();
    for _ in 0..fallback_count {
        fallbacks.push(read_font_descriptor(reader)?);
    }
    Ok(FontFallbackChain { primary, fallbacks })
}

fn write_font_descriptor(
    writer: &mut Writer,
    descriptor: &FontDescriptor,
) -> Result<(), WireError> {
    write_option(writer, descriptor.font_id, |writer, font_id| {
        writer.write_u32(font_id.get());
        Ok(())
    })?;
    writer.write_string("font family", &descriptor.family)?;
    writer.write_u16(descriptor.weight);
    writer.write_u8(font_style_to_wire(descriptor.style));
    writer.write_u16(descriptor.stretch);
    writer.write_u64(descriptor.fingerprint.0);
    Ok(())
}

fn read_font_descriptor(reader: &mut Reader<'_>) -> Result<FontDescriptor, WireError> {
    Ok(FontDescriptor {
        font_id: read_option(reader, "font id", |reader| {
            Ok(FontId::new(reader.read_u32("font id")?))
        })?,
        family: reader.read_string("font family")?,
        weight: reader.read_u16("font weight")?,
        style: font_style_from_wire(reader.read_u8("font style")?)?,
        stretch: reader.read_u16("font stretch")?,
        fingerprint: FontSetFingerprint(reader.read_u64("font fingerprint")?),
    })
}

fn validate_request_ranges(request: &MeasureRequest) -> Result<(), WireError> {
    validate_utf8_range(
        "measure text range",
        &request.text,
        request.text_range.start,
        request.text_range.end,
    )?;
    for style_run in &request.style_runs {
        validate_utf8_range(
            "text style run",
            &request.text,
            style_run.start,
            style_run.end,
        )?;
        if style_run.start < request.text_range.start || style_run.end > request.text_range.end {
            return Err(WireError::InvalidRange {
                field: "text style run",
            });
        }
    }
    Ok(())
}

fn validate_utf8_range(
    field: &'static str,
    text: &str,
    start: u32,
    end: u32,
) -> Result<(), WireError> {
    let start = start as usize;
    let end = end as usize;
    if start <= end
        && end <= text.len()
        && text.is_char_boundary(start)
        && text.is_char_boundary(end)
    {
        Ok(())
    } else {
        Err(WireError::InvalidRange { field })
    }
}

fn write_page_scene(writer: &mut Writer, page: &PageScene) -> Result<(), WireError> {
    writer.write_u32(page.page_index);
    write_page_size(writer, page.size);
    write_option(writer, page.start_anchor, |writer, anchor| {
        write_text_anchor(writer, anchor);
        Ok(())
    })?;
    write_option(writer, page.end_anchor, |writer, anchor| {
        write_text_anchor(writer, anchor);
        Ok(())
    })?;

    writer.write_collection_len("scene fragments", page.fragments.len())?;
    for fragment in &page.fragments {
        write_scene_fragment(writer, fragment)?;
    }

    writer.write_collection_len("link regions", page.links.len())?;
    for link in &page.links {
        write_link_region(writer, link)?;
    }

    writer.write_collection_len("anchor regions", page.anchors.len())?;
    for anchor in &page.anchors {
        write_anchor_region(writer, anchor)?;
    }

    writer.write_collection_len("selection maps", page.selections.len())?;
    for selection in &page.selections {
        write_selection_map(writer, selection)?;
    }

    writer.write_collection_len("semantic nodes", page.semantics.len())?;
    for semantic in &page.semantics {
        write_semantic_node(writer, semantic)?;
    }

    writer.write_hash(page.fingerprint.0);
    write_option(writer, page.next_break_token.as_ref(), |writer, token| {
        write_break_token(writer, token);
        Ok(())
    })?;

    writer.write_collection_len("diagnostics", page.diagnostics.len())?;
    for diagnostic in &page.diagnostics {
        write_diagnostic(writer, diagnostic)?;
    }
    Ok(())
}

fn read_page_scene(reader: &mut Reader<'_>) -> Result<PageScene, WireError> {
    let page_index = reader.read_u32("page index")?;
    let size = read_page_size(reader)?;
    let start_anchor = read_option(reader, "start anchor", read_text_anchor)?;
    let end_anchor = read_option(reader, "end anchor", read_text_anchor)?;

    let fragment_count = reader.read_collection_len("scene fragments")?;
    let mut fragments = Vec::new();
    for _ in 0..fragment_count {
        fragments.push(read_scene_fragment(reader)?);
    }

    let link_count = reader.read_collection_len("link regions")?;
    let mut links = Vec::new();
    for _ in 0..link_count {
        links.push(read_link_region(reader)?);
    }

    let anchor_count = reader.read_collection_len("anchor regions")?;
    let mut anchors = Vec::new();
    for _ in 0..anchor_count {
        anchors.push(read_anchor_region(reader)?);
    }

    let selection_count = reader.read_collection_len("selection maps")?;
    let mut selections = Vec::new();
    for _ in 0..selection_count {
        selections.push(read_selection_map(reader)?);
    }

    let semantic_count = reader.read_collection_len("semantic nodes")?;
    let mut semantics = Vec::new();
    for _ in 0..semantic_count {
        semantics.push(read_semantic_node(reader)?);
    }

    let fingerprint = PageFingerprint(reader.read_hash("page fingerprint")?);
    let next_break_token = read_option(reader, "next break token", read_break_token)?;

    let diagnostic_count = reader.read_collection_len("diagnostics")?;
    let mut diagnostics = Vec::new();
    for _ in 0..diagnostic_count {
        diagnostics.push(read_diagnostic(reader)?);
    }

    Ok(PageScene {
        page_index,
        size,
        start_anchor,
        end_anchor,
        fragments,
        links,
        anchors,
        selections,
        semantics,
        fingerprint,
        next_break_token,
        diagnostics,
    })
}

fn write_scene_fragment(writer: &mut Writer, fragment: &SceneFragment) -> Result<(), WireError> {
    writer.write_u32(fragment.id);
    writer.write_u8(scene_fragment_kind_to_wire(&fragment.kind));
    writer.write_u32(fragment.node_id.get());
    write_rect(writer, fragment.rect);
    write_option(writer, fragment.text.as_deref(), |writer, text| {
        writer.write_string("fragment text", text)
    })?;
    write_option(writer, fragment.source_range, |writer, source_range| {
        write_source_range(writer, source_range)
    })?;
    write_option(writer, fragment.anchor_range, |writer, anchor_range| {
        write_text_anchor_range(writer, anchor_range);
        Ok(())
    })?;
    write_option(writer, fragment.line_index, |writer, line_index| {
        writer.write_u32(line_index);
        Ok(())
    })?;
    writer.write_bool(fragment.overflow);
    Ok(())
}

fn read_scene_fragment(reader: &mut Reader<'_>) -> Result<SceneFragment, WireError> {
    Ok(SceneFragment {
        id: reader.read_u32("fragment id")?,
        kind: scene_fragment_kind_from_wire(reader.read_u8("fragment kind")?)?,
        node_id: NodeId::new(reader.read_u32("fragment node id")?),
        rect: read_rect(reader)?,
        text: read_option(reader, "fragment text", |reader| {
            reader.read_string("fragment text")
        })?,
        source_range: read_option(reader, "fragment source range", read_source_range)?,
        anchor_range: read_option(reader, "fragment anchor range", read_text_anchor_range)?,
        line_index: read_option(reader, "fragment line index", |reader| {
            reader.read_u32("fragment line index")
        })?,
        overflow: reader.read_bool("fragment overflow")?,
    })
}

fn write_link_region(writer: &mut Writer, link: &LinkRegion) -> Result<(), WireError> {
    write_rect(writer, link.rect);
    writer.write_u32(link.node_id.get());
    writer.write_string("link href", &link.href)?;
    write_option(
        writer,
        link.resolved_document.as_deref(),
        |writer, document| writer.write_string("resolved link document", document),
    )?;
    write_option(writer, link.fragment.as_deref(), |writer, fragment| {
        writer.write_string("link fragment", fragment)
    })?;
    writer.write_u8(link_kind_to_wire(&link.kind));
    Ok(())
}

fn read_link_region(reader: &mut Reader<'_>) -> Result<LinkRegion, WireError> {
    Ok(LinkRegion {
        rect: read_rect(reader)?,
        node_id: NodeId::new(reader.read_u32("link node id")?),
        href: reader.read_string("link href")?,
        resolved_document: read_option(reader, "resolved link document", |reader| {
            reader.read_string("resolved link document")
        })?,
        fragment: read_option(reader, "link fragment", |reader| {
            reader.read_string("link fragment")
        })?,
        kind: link_kind_from_wire(reader.read_u8("link kind")?)?,
    })
}

fn write_anchor_region(writer: &mut Writer, anchor: &AnchorRegion) -> Result<(), WireError> {
    write_rect(writer, anchor.rect);
    writer.write_string("anchor key", &anchor.key)?;
    writer.write_u32(anchor.node_id.get());
    Ok(())
}

fn read_anchor_region(reader: &mut Reader<'_>) -> Result<AnchorRegion, WireError> {
    Ok(AnchorRegion {
        rect: read_rect(reader)?,
        key: reader.read_string("anchor key")?,
        node_id: NodeId::new(reader.read_u32("anchor node id")?),
    })
}

fn write_selection_map(writer: &mut Writer, selection: &SelectionMap) -> Result<(), WireError> {
    if selection.start > selection.end {
        return Err(WireError::InvalidRange { field: "selection" });
    }
    writer.write_u32(selection.node_id.get());
    writer.write_u32(selection.start);
    writer.write_u32(selection.end);
    writer.write_collection_len("selection rectangles", selection.rects.len())?;
    for rect in &selection.rects {
        write_rect(writer, *rect);
    }
    Ok(())
}

fn read_selection_map(reader: &mut Reader<'_>) -> Result<SelectionMap, WireError> {
    let node_id = NodeId::new(reader.read_u32("selection node id")?);
    let start = reader.read_u32("selection start")?;
    let end = reader.read_u32("selection end")?;
    if start > end {
        return Err(WireError::InvalidRange { field: "selection" });
    }
    let rect_count = reader.read_collection_len("selection rectangles")?;
    let mut rects = Vec::new();
    for _ in 0..rect_count {
        rects.push(read_rect(reader)?);
    }
    Ok(SelectionMap {
        node_id,
        start,
        end,
        rects,
    })
}

fn write_semantic_node(writer: &mut Writer, semantic: &SemanticNode) -> Result<(), WireError> {
    writer.write_u32(semantic.node_id.get());
    write_rect(writer, semantic.rect);
    writer.write_string("semantic role", &semantic.role)?;
    writer.write_string("semantic label", &semantic.label)?;
    Ok(())
}

fn read_semantic_node(reader: &mut Reader<'_>) -> Result<SemanticNode, WireError> {
    Ok(SemanticNode {
        node_id: NodeId::new(reader.read_u32("semantic node id")?),
        rect: read_rect(reader)?,
        role: reader.read_string("semantic role")?,
        label: reader.read_string("semantic label")?,
    })
}

fn write_page_size(writer: &mut Writer, size: PageSize) {
    writer.write_layout_unit(size.width);
    writer.write_layout_unit(size.height);
}

fn read_page_size(reader: &mut Reader<'_>) -> Result<PageSize, WireError> {
    Ok(PageSize {
        width: reader.read_layout_unit("page width")?,
        height: reader.read_layout_unit("page height")?,
    })
}

fn write_rect(writer: &mut Writer, rect: Rect) {
    writer.write_layout_unit(rect.x);
    writer.write_layout_unit(rect.y);
    writer.write_layout_unit(rect.width);
    writer.write_layout_unit(rect.height);
}

fn read_rect(reader: &mut Reader<'_>) -> Result<Rect, WireError> {
    Ok(Rect {
        x: reader.read_layout_unit("rectangle x")?,
        y: reader.read_layout_unit("rectangle y")?,
        width: reader.read_layout_unit("rectangle width")?,
        height: reader.read_layout_unit("rectangle height")?,
    })
}

fn write_source_range(writer: &mut Writer, source_range: SourceRange) -> Result<(), WireError> {
    if source_range.start > source_range.end {
        return Err(WireError::InvalidRange {
            field: "source range",
        });
    }
    writer.write_u32(source_range.start);
    writer.write_u32(source_range.end);
    Ok(())
}

fn read_source_range(reader: &mut Reader<'_>) -> Result<SourceRange, WireError> {
    let start = reader.read_u32("source range start")?;
    let end = reader.read_u32("source range end")?;
    SourceRange::new(start, end).ok_or(WireError::InvalidRange {
        field: "source range",
    })
}

fn write_text_anchor(writer: &mut Writer, anchor: TextAnchor) {
    writer.write_u32(anchor.document_id.get());
    writer.write_u32(anchor.node_id.get());
    writer.write_u32(anchor.utf8_byte_offset);
    writer.write_u8(text_affinity_to_wire(anchor.affinity));
}

fn read_text_anchor(reader: &mut Reader<'_>) -> Result<TextAnchor, WireError> {
    Ok(TextAnchor {
        document_id: DocumentId::new(reader.read_u32("anchor document id")?),
        node_id: NodeId::new(reader.read_u32("anchor node id")?),
        utf8_byte_offset: reader.read_u32("anchor UTF-8 offset")?,
        affinity: text_affinity_from_wire(reader.read_u8("anchor affinity")?)?,
    })
}

fn write_text_anchor_range(writer: &mut Writer, range: TextAnchorRange) {
    write_text_anchor(writer, range.start);
    write_text_anchor(writer, range.end);
}

fn read_text_anchor_range(reader: &mut Reader<'_>) -> Result<TextAnchorRange, WireError> {
    Ok(TextAnchorRange {
        start: read_text_anchor(reader)?,
        end: read_text_anchor(reader)?,
    })
}

fn write_break_token(writer: &mut Writer, token: &BreakToken) {
    writer.write_u32(token.node_id.get());
    writer.write_u32(token.child_index);
    writer.write_u32(token.text_offset);
    writer.write_bool(token.continuation);
    writer.write_u32(token.page_index);
    writer.write_hash(token.content_fingerprint);
    writer.write_u64(token.config_fingerprint);
    writer.write_u64(token.text_backend_id.0);
    writer.write_u64(token.font_fingerprint.0);
}

fn read_break_token(reader: &mut Reader<'_>) -> Result<BreakToken, WireError> {
    Ok(BreakToken {
        node_id: NodeId::new(reader.read_u32("break token node id")?),
        child_index: reader.read_u32("break token child index")?,
        text_offset: reader.read_u32("break token text offset")?,
        continuation: reader.read_bool("break token continuation")?,
        page_index: reader.read_u32("break token page index")?,
        content_fingerprint: reader.read_hash("break token content fingerprint")?,
        config_fingerprint: reader.read_u64("break token config fingerprint")?,
        text_backend_id: TextBackendId(reader.read_u64("break token text backend id")?),
        font_fingerprint: FontSetFingerprint(reader.read_u64("break token font fingerprint")?),
    })
}

fn write_diagnostic(writer: &mut Writer, diagnostic: &Diagnostic) -> Result<(), WireError> {
    writer.write_u8(diagnostic_code_to_wire(diagnostic.code));
    writer.write_u8(severity_to_wire(diagnostic.severity));
    writer.write_string("diagnostic message", &diagnostic.message)?;
    write_option(writer, diagnostic.resource, |writer, resource| {
        writer.write_u32(resource.get());
        Ok(())
    })?;
    write_option(writer, diagnostic.source_range, |writer, source_range| {
        write_source_range(writer, source_range)
    })
}

fn read_diagnostic(reader: &mut Reader<'_>) -> Result<Diagnostic, WireError> {
    Ok(Diagnostic {
        code: diagnostic_code_from_wire(reader.read_u8("diagnostic code")?)?,
        severity: severity_from_wire(reader.read_u8("diagnostic severity")?)?,
        message: reader.read_string("diagnostic message")?,
        resource: read_option(reader, "diagnostic resource", |reader| {
            Ok(ResourceId::new(reader.read_u32("diagnostic resource")?))
        })?,
        source_range: read_option(reader, "diagnostic source range", read_source_range)?,
    })
}

fn write_option<T>(
    writer: &mut Writer,
    value: Option<T>,
    write_value: impl FnOnce(&mut Writer, T) -> Result<(), WireError>,
) -> Result<(), WireError> {
    writer.write_bool(value.is_some());
    if let Some(value) = value {
        write_value(writer, value)?;
    }
    Ok(())
}

fn read_option<T>(
    reader: &mut Reader<'_>,
    field: &'static str,
    read_value: impl FnOnce(&mut Reader<'_>) -> Result<T, WireError>,
) -> Result<Option<T>, WireError> {
    if reader.read_bool(field)? {
        Ok(Some(read_value(reader)?))
    } else {
        Ok(None)
    }
}

const fn text_direction_to_wire(value: TextDirection) -> u8 {
    match value {
        TextDirection::Auto => 0,
        TextDirection::Ltr => 1,
        TextDirection::Rtl => 2,
    }
}

fn text_direction_from_wire(value: u8) -> Result<TextDirection, WireError> {
    match value {
        0 => Ok(TextDirection::Auto),
        1 => Ok(TextDirection::Ltr),
        2 => Ok(TextDirection::Rtl),
        actual => Err(WireError::InvalidEnum {
            field: "text direction",
            actual,
        }),
    }
}

const fn font_style_to_wire(value: FontStyle) -> u8 {
    match value {
        FontStyle::Normal => 0,
        FontStyle::Italic => 1,
        FontStyle::Oblique => 2,
    }
}

fn font_style_from_wire(value: u8) -> Result<FontStyle, WireError> {
    match value {
        0 => Ok(FontStyle::Normal),
        1 => Ok(FontStyle::Italic),
        2 => Ok(FontStyle::Oblique),
        actual => Err(WireError::InvalidEnum {
            field: "font style",
            actual,
        }),
    }
}

const fn height_behavior_to_wire(value: HeightBehavior) -> u8 {
    match value {
        HeightBehavior::Natural => 0,
        HeightBehavior::IncludeStrut => 1,
        HeightBehavior::Tight => 2,
    }
}

fn height_behavior_from_wire(value: u8) -> Result<HeightBehavior, WireError> {
    match value {
        0 => Ok(HeightBehavior::Natural),
        1 => Ok(HeightBehavior::IncludeStrut),
        2 => Ok(HeightBehavior::Tight),
        actual => Err(WireError::InvalidEnum {
            field: "height behavior",
            actual,
        }),
    }
}

const fn text_affinity_to_wire(value: TextAffinity) -> u8 {
    match value {
        TextAffinity::Upstream => 0,
        TextAffinity::Downstream => 1,
    }
}

fn text_affinity_from_wire(value: u8) -> Result<TextAffinity, WireError> {
    match value {
        0 => Ok(TextAffinity::Upstream),
        1 => Ok(TextAffinity::Downstream),
        actual => Err(WireError::InvalidEnum {
            field: "text affinity",
            actual,
        }),
    }
}

const fn scene_fragment_kind_to_wire(value: &SceneFragmentKind) -> u8 {
    match value {
        SceneFragmentKind::TextLine => 0,
        SceneFragmentKind::Marker => 1,
        SceneFragmentKind::Image => 2,
        SceneFragmentKind::Divider => 3,
        SceneFragmentKind::BackgroundBorder => 4,
        SceneFragmentKind::DebugOverlay => 5,
        SceneFragmentKind::UnsupportedPlaceholder => 6,
    }
}

fn scene_fragment_kind_from_wire(value: u8) -> Result<SceneFragmentKind, WireError> {
    match value {
        0 => Ok(SceneFragmentKind::TextLine),
        1 => Ok(SceneFragmentKind::Marker),
        2 => Ok(SceneFragmentKind::Image),
        3 => Ok(SceneFragmentKind::Divider),
        4 => Ok(SceneFragmentKind::BackgroundBorder),
        5 => Ok(SceneFragmentKind::DebugOverlay),
        6 => Ok(SceneFragmentKind::UnsupportedPlaceholder),
        actual => Err(WireError::InvalidEnum {
            field: "fragment kind",
            actual,
        }),
    }
}

const fn link_kind_to_wire(value: &LinkKind) -> u8 {
    match value {
        LinkKind::Internal => 0,
        LinkKind::External => 1,
        LinkKind::Resource => 2,
        LinkKind::Footnote => 3,
        LinkKind::Unknown => 4,
    }
}

fn link_kind_from_wire(value: u8) -> Result<LinkKind, WireError> {
    match value {
        0 => Ok(LinkKind::Internal),
        1 => Ok(LinkKind::External),
        2 => Ok(LinkKind::Resource),
        3 => Ok(LinkKind::Footnote),
        4 => Ok(LinkKind::Unknown),
        actual => Err(WireError::InvalidEnum {
            field: "link kind",
            actual,
        }),
    }
}

const fn diagnostic_code_to_wire(value: DiagnosticCode) -> u8 {
    match value {
        DiagnosticCode::Io => 0,
        DiagnosticCode::InvalidContainer => 1,
        DiagnosticCode::InvalidPackage => 2,
        DiagnosticCode::UnsupportedFeature => 3,
        DiagnosticCode::ResourceLimitExceeded => 4,
        DiagnosticCode::Parse => 5,
        DiagnosticCode::Layout => 6,
        DiagnosticCode::Cancelled => 7,
        DiagnosticCode::Protocol => 8,
        DiagnosticCode::Internal => 9,
    }
}

fn diagnostic_code_from_wire(value: u8) -> Result<DiagnosticCode, WireError> {
    match value {
        0 => Ok(DiagnosticCode::Io),
        1 => Ok(DiagnosticCode::InvalidContainer),
        2 => Ok(DiagnosticCode::InvalidPackage),
        3 => Ok(DiagnosticCode::UnsupportedFeature),
        4 => Ok(DiagnosticCode::ResourceLimitExceeded),
        5 => Ok(DiagnosticCode::Parse),
        6 => Ok(DiagnosticCode::Layout),
        7 => Ok(DiagnosticCode::Cancelled),
        8 => Ok(DiagnosticCode::Protocol),
        9 => Ok(DiagnosticCode::Internal),
        actual => Err(WireError::InvalidEnum {
            field: "diagnostic code",
            actual,
        }),
    }
}

const fn severity_to_wire(value: Severity) -> u8 {
    match value {
        Severity::Info => 0,
        Severity::Warning => 1,
        Severity::Error => 2,
    }
}

fn severity_from_wire(value: u8) -> Result<Severity, WireError> {
    match value {
        0 => Ok(Severity::Info),
        1 => Ok(Severity::Warning),
        2 => Ok(Severity::Error),
        actual => Err(WireError::InvalidEnum {
            field: "diagnostic severity",
            actual,
        }),
    }
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    }

    #[test]
    fn unknown_enum_discriminants_are_rejected() {
        assert_eq!(
            scene_fragment_kind_from_wire(u8::MAX),
            Err(WireError::InvalidEnum {
                field: "fragment kind",
                actual: u8::MAX,
            })
        );
        assert_eq!(
            text_direction_from_wire(u8::MAX),
            Err(WireError::InvalidEnum {
                field: "text direction",
                actual: u8::MAX,
            })
        );
    }

    #[test]
    fn invalid_boolean_is_rejected() {
        let mut reader = Reader::new(&[2]);
        assert_eq!(
            reader.read_bool("fixture"),
            Err(WireError::InvalidBoolean {
                field: "fixture",
                actual: 2,
            })
        );
    }
}
