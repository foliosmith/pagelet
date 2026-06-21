//! Core identifiers, diagnostics, limits, anchors, and fixed-point units.

use std::{
    fmt,
    hash::Hasher,
    io,
    ops::{Add, AddAssign, Neg, Sub, SubAssign},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

macro_rules! id_type {
    ($name:ident, $inner:ty) => {
        #[doc = concat!("Strongly typed ", stringify!($name), " identifier.")]
        #[repr(transparent)]
        #[derive(Debug, Default, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
        pub struct $name(pub $inner);

        impl $name {
            /// Create an identifier from its raw storage value.
            #[must_use]
            pub const fn new(value: $inner) -> Self {
                Self(value)
            }

            /// Return the raw storage value.
            #[must_use]
            pub const fn get(self) -> $inner {
                self.0
            }
        }

        impl From<$inner> for $name {
            fn from(value: $inner) -> Self {
                Self(value)
            }
        }

        impl From<$name> for $inner {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

id_type!(BookId, u64);
id_type!(DocumentId, u32);
id_type!(NodeId, u32);
id_type!(ResourceId, u32);
id_type!(StyleId, u32);
id_type!(FontId, u32);

/// Fixed-point layout unit with 1/64 logical pixel precision.
#[repr(transparent)]
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct LayoutUnit(i64);

impl LayoutUnit {
    /// Number of layout units in one logical pixel.
    pub const SCALE: i64 = 64;
    /// Zero layout units.
    pub const ZERO: Self = Self(0);

    /// Create a value from raw fixed-point units.
    #[must_use]
    pub const fn from_raw(raw: i64) -> Self {
        Self(raw)
    }

    /// Return raw fixed-point units.
    #[must_use]
    pub const fn raw(self) -> i64 {
        self.0
    }

    /// Create a value from whole logical pixels.
    #[must_use]
    pub const fn from_px(px: i64) -> Self {
        Self(px * Self::SCALE)
    }

    /// Quantize logical pixels into layout units.
    #[must_use]
    pub fn from_f64_px(px: f64) -> Self {
        Self((px * Self::SCALE as f64).round() as i64)
    }

    /// Convert to logical pixels.
    #[must_use]
    pub fn to_f64_px(self) -> f64 {
        self.0 as f64 / Self::SCALE as f64
    }
}

impl Add for LayoutUnit {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl AddAssign for LayoutUnit {
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

impl Sub for LayoutUnit {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

impl SubAssign for LayoutUnit {
    fn sub_assign(&mut self, rhs: Self) {
        self.0 -= rhs.0;
    }
}

impl Neg for LayoutUnit {
    type Output = Self;

    fn neg(self) -> Self::Output {
        Self(-self.0)
    }
}

/// Inclusive start, exclusive end byte range inside a source resource.
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq, Hash)]
pub struct SourceRange {
    /// UTF-8 byte offset at the start of the source range.
    pub start: u32,
    /// UTF-8 byte offset immediately after the source range.
    pub end: u32,
}

impl SourceRange {
    /// Create a range when `start <= end`.
    #[must_use]
    pub const fn new(start: u32, end: u32) -> Option<Self> {
        if start <= end {
            Some(Self { start, end })
        } else {
            None
        }
    }

    /// Create a range from a start offset and length.
    #[must_use]
    pub const fn from_start_len(start: u32, len: u32) -> Option<Self> {
        match start.checked_add(len) {
            Some(end) => Some(Self { start, end }),
            None => None,
        }
    }

    /// Return the byte length of the range.
    #[must_use]
    pub const fn len(self) -> u32 {
        self.end - self.start
    }

    /// Return true when the range is empty.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }
}

impl TryFrom<std::ops::Range<u32>> for SourceRange {
    type Error = InvalidSourceRange;

    fn try_from(value: std::ops::Range<u32>) -> Result<Self, Self::Error> {
        Self::new(value.start, value.end).ok_or(InvalidSourceRange)
    }
}

impl From<SourceRange> for std::ops::Range<u32> {
    fn from(value: SourceRange) -> Self {
        value.start..value.end
    }
}

/// Error returned when a source range has an invalid ordering.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct InvalidSourceRange;

impl fmt::Display for InvalidSourceRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("source range start must be less than or equal to end")
    }
}

impl std::error::Error for InvalidSourceRange {}

/// Cursor affinity used when a text offset lies on a boundary.
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq, Hash)]
pub enum TextAffinity {
    /// Prefer the upstream visual/logical position.
    Upstream,
    /// Prefer the downstream visual/logical position.
    #[default]
    Downstream,
}

/// Stable text position inside a document node.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub struct TextAnchor {
    /// Document containing the node.
    pub document_id: DocumentId,
    /// Node containing the offset.
    pub node_id: NodeId,
    /// UTF-8 byte offset inside the node text.
    pub utf8_byte_offset: u32,
    /// Boundary affinity.
    pub affinity: TextAffinity,
}

impl TextAnchor {
    /// Create a stable text anchor.
    #[must_use]
    pub const fn new(
        document_id: DocumentId,
        node_id: NodeId,
        utf8_byte_offset: u32,
        affinity: TextAffinity,
    ) -> Self {
        Self {
            document_id,
            node_id,
            utf8_byte_offset,
            affinity,
        }
    }
}

/// Severity of a diagnostic.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

/// Stable diagnostic code for UI, tests, and telemetry aggregation.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum DiagnosticCode {
    Io,
    InvalidContainer,
    InvalidPackage,
    UnsupportedFeature,
    ResourceLimitExceeded,
    Parse,
    Layout,
    Cancelled,
    Protocol,
    Internal,
}

/// Warning or error emitted while parsing, laying out, or adapting data.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Diagnostic {
    pub code: DiagnosticCode,
    pub severity: Severity,
    pub message: Arc<str>,
    pub resource: Option<ResourceId>,
    pub source_range: Option<SourceRange>,
}

impl Diagnostic {
    /// Create a diagnostic with a stable code, severity, and message.
    #[must_use]
    pub fn new(code: DiagnosticCode, severity: Severity, message: impl Into<Arc<str>>) -> Self {
        Self {
            code,
            severity,
            message: message.into(),
            resource: None,
            source_range: None,
        }
    }

    /// Attach a resource identifier.
    #[must_use]
    pub const fn with_resource(mut self, resource: ResourceId) -> Self {
        self.resource = Some(resource);
        self
    }

    /// Attach a source byte range.
    #[must_use]
    pub const fn with_source_range(mut self, source_range: SourceRange) -> Self {
        self.source_range = Some(source_range);
        self
    }
}

/// Typed error returned by pagelet APIs.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum PageletError {
    Io(IoError),
    InvalidContainer(ContainerError),
    InvalidPackage(PackageError),
    UnsupportedFeature(UnsupportedFeature),
    ResourceLimitExceeded(ResourceLimitError),
    Parse(ParseError),
    Layout(LayoutError),
    Cancelled,
    Protocol(ProtocolError),
    Internal(InternalErrorId),
}

impl PageletError {
    /// Return the stable diagnostic code matching the error variant.
    #[must_use]
    pub const fn code(&self) -> DiagnosticCode {
        match self {
            Self::Io(_) => DiagnosticCode::Io,
            Self::InvalidContainer(_) => DiagnosticCode::InvalidContainer,
            Self::InvalidPackage(_) => DiagnosticCode::InvalidPackage,
            Self::UnsupportedFeature(_) => DiagnosticCode::UnsupportedFeature,
            Self::ResourceLimitExceeded(_) => DiagnosticCode::ResourceLimitExceeded,
            Self::Parse(_) => DiagnosticCode::Parse,
            Self::Layout(_) => DiagnosticCode::Layout,
            Self::Cancelled => DiagnosticCode::Cancelled,
            Self::Protocol(_) => DiagnosticCode::Protocol,
            Self::Internal(_) => DiagnosticCode::Internal,
        }
    }
}

impl fmt::Display for PageletError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "I/O error: {error}"),
            Self::InvalidContainer(error) => write!(f, "invalid container: {error}"),
            Self::InvalidPackage(error) => write!(f, "invalid package: {error}"),
            Self::UnsupportedFeature(error) => write!(f, "unsupported feature: {error}"),
            Self::ResourceLimitExceeded(error) => write!(f, "resource limit exceeded: {error}"),
            Self::Parse(error) => write!(f, "parse error: {error}"),
            Self::Layout(error) => write!(f, "layout error: {error}"),
            Self::Cancelled => f.write_str("operation cancelled"),
            Self::Protocol(error) => write!(f, "protocol error: {error}"),
            Self::Internal(error) => write!(f, "internal error: {error}"),
        }
    }
}

impl std::error::Error for PageletError {}

impl From<io::Error> for PageletError {
    fn from(value: io::Error) -> Self {
        Self::Io(IoError::from(value))
    }
}

macro_rules! message_error {
    ($name:ident) => {
        #[derive(Debug, Clone, Eq, PartialEq)]
        pub struct $name {
            pub message: Arc<str>,
        }

        impl $name {
            #[must_use]
            pub fn new(message: impl Into<Arc<str>>) -> Self {
                Self {
                    message: message.into(),
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.message)
            }
        }

        impl std::error::Error for $name {}
    };
}

/// I/O error payload with cloneable storage.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct IoError {
    pub kind: io::ErrorKind,
    pub message: Arc<str>,
}

impl IoError {
    #[must_use]
    pub fn new(kind: io::ErrorKind, message: impl Into<Arc<str>>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl From<io::Error> for IoError {
    fn from(value: io::Error) -> Self {
        Self::new(value.kind(), value.to_string())
    }
}

impl fmt::Display for IoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)
    }
}

impl std::error::Error for IoError {}

message_error!(ContainerError);
message_error!(PackageError);
message_error!(UnsupportedFeature);
message_error!(ParseError);
message_error!(LayoutError);
message_error!(ProtocolError);

/// Internal error identifier used when the detailed cause is not public API.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub struct InternalErrorId(pub u64);

impl fmt::Display for InternalErrorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "internal error {}", self.0)
    }
}

impl std::error::Error for InternalErrorId {}

/// Resource limit that was exceeded.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub enum ResourceLimitKind {
    ZipEntries,
    TotalDecompressedBytes,
    SingleResourceBytes,
    CompressionRatio,
    XmlDepth,
    CssImportDepth,
    CssSelectors,
    DataUriBytes,
    DomNodes,
    ParagraphChars,
    LayoutFragments,
    PageBacktrackCandidates,
    Diagnostics,
}

/// Resource limit error payload.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ResourceLimitError {
    pub kind: ResourceLimitKind,
    pub limit: u64,
    pub observed: u64,
}

impl ResourceLimitError {
    #[must_use]
    pub const fn new(kind: ResourceLimitKind, limit: u64, observed: u64) -> Self {
        Self {
            kind,
            limit,
            observed,
        }
    }
}

impl fmt::Display for ResourceLimitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:?} limit {} exceeded by observed {}",
            self.kind, self.limit, self.observed
        )
    }
}

impl std::error::Error for ResourceLimitError {}

/// Configurable safety and resource limits.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ResourceLimits {
    pub max_zip_entries: u32,
    pub max_total_decompressed_bytes: u64,
    pub max_single_resource_bytes: u64,
    pub max_compression_ratio: u32,
    pub max_xml_depth: u32,
    pub max_css_import_depth: u32,
    pub max_css_selectors: u32,
    pub max_data_uri_bytes: u64,
    pub max_dom_nodes: u32,
    pub max_paragraph_chars: u32,
    pub max_layout_fragments: u32,
    pub max_page_backtrack_candidates: u32,
    pub max_diagnostics: u32,
}

impl ResourceLimits {
    /// Conservative defaults intended for mobile hosts.
    #[must_use]
    pub const fn mobile_defaults() -> Self {
        Self {
            max_zip_entries: 16_384,
            max_total_decompressed_bytes: 512 * 1024 * 1024,
            max_single_resource_bytes: 64 * 1024 * 1024,
            max_compression_ratio: 100,
            max_xml_depth: 256,
            max_css_import_depth: 8,
            max_css_selectors: 65_536,
            max_data_uri_bytes: 2 * 1024 * 1024,
            max_dom_nodes: 1_000_000,
            max_paragraph_chars: 1_000_000,
            max_layout_fragments: 2_000_000,
            max_page_backtrack_candidates: 256,
            max_diagnostics: 10_000,
        }
    }
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self::mobile_defaults()
    }
}

/// Cloneable cancellation token shared by long-running operations.
#[derive(Debug, Clone, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    /// Create a token in the non-cancelled state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Request cancellation.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    /// Return true when cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

/// Deterministic, non-cryptographic content hash used for cache keys.
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq, Hash)]
pub struct ContentHash([u8; 32]);

impl ContentHash {
    /// Hash bytes into a stable 32-byte value.
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Self {
        const SEEDS: [u64; 4] = [
            0xcbf2_9ce4_8422_2325,
            0x8422_2325_cbf2_9ce4,
            0x9e37_79b9_7f4a_7c15,
            0x517c_c1b7_2722_0a95,
        ];

        let mut out = [0_u8; 32];
        for (index, seed) in SEEDS.into_iter().enumerate() {
            let mut hasher = StableHasher::new(seed);
            hasher.write(bytes);
            hasher.write_usize(bytes.len());
            let hash = hasher.finish();
            out[index * 8..(index + 1) * 8].copy_from_slice(&hash.to_le_bytes());
        }

        Self(out)
    }

    /// Create a hash from raw bytes.
    #[must_use]
    pub const fn from_array(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Return raw hash bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[derive(Debug, Clone, Copy)]
struct StableHasher(u64);

impl StableHasher {
    const fn new(seed: u64) -> Self {
        Self(seed)
    }
}

impl Hasher for StableHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        const PRIME: u64 = 0x0000_0100_0000_01b3;
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(PRIME);
        }
    }
}

/// Independent versions used by cache and wire compatibility keys.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub struct EngineVersions {
    pub parser_schema: u32,
    pub style_schema: u32,
    pub text_schema: u32,
    pub pagination_algorithm: u32,
    pub scene_wire: u32,
    pub disk_cache: u32,
}

impl EngineVersions {
    /// Current pre-alpha version set.
    pub const CURRENT: Self = Self {
        parser_schema: 1,
        style_schema: 1,
        text_schema: 1,
        pagination_algorithm: 1,
        scene_wire: 1,
        disk_cache: 1,
    };
}

impl Default for EngineVersions {
    fn default() -> Self {
        Self::CURRENT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_ids_do_not_mix_raw_types() {
        let book = BookId::new(7);
        let document = DocumentId::new(7);
        let node = NodeId::new(8);
        let resource = ResourceId::new(9);
        let style = StyleId::new(10);
        let font = FontId::new(11);

        assert_eq!(book.get(), 7);
        assert_eq!(document.get(), 7);
        assert_eq!(node.get(), 8);
        assert_eq!(resource.get(), 9);
        assert_eq!(style.get(), 10);
        assert_eq!(font.get(), 11);
    }

    #[test]
    fn layout_unit_quantizes_logical_pixels() {
        let one = LayoutUnit::from_px(1);
        let half = LayoutUnit::from_f64_px(0.5);
        let quarter = LayoutUnit::from_raw(16);

        assert_eq!(one.raw(), 64);
        assert_eq!(half.raw(), 32);
        assert_eq!((half + quarter).to_f64_px(), 0.75);
        assert_eq!((one - half).raw(), 32);
        assert_eq!((-quarter).raw(), -16);
    }

    #[test]
    fn source_range_converts_to_and_from_std_range() {
        let source = SourceRange::try_from(3..9).expect("valid range");
        let std_range: std::ops::Range<u32> = source.into();

        assert_eq!(source.start, 3);
        assert_eq!(source.end, 9);
        assert_eq!(source.len(), 6);
        assert_eq!(std_range, 3..9);
        assert_eq!(
            SourceRange::from_start_len(u32::MAX, 1),
            None,
            "overflow is rejected"
        );
    }

    #[test]
    fn text_anchor_stores_stable_utf8_position() {
        let anchor = TextAnchor::new(
            DocumentId::new(1),
            NodeId::new(2),
            42,
            TextAffinity::Upstream,
        );

        assert_eq!(anchor.document_id, DocumentId(1));
        assert_eq!(anchor.node_id, NodeId(2));
        assert_eq!(anchor.utf8_byte_offset, 42);
        assert_eq!(anchor.affinity, TextAffinity::Upstream);
    }

    #[test]
    fn diagnostic_can_attach_resource_and_source_range() {
        let range = SourceRange::new(4, 12).expect("valid range");
        let diagnostic = Diagnostic::new(DiagnosticCode::Parse, Severity::Warning, "bad token")
            .with_resource(ResourceId::new(3))
            .with_source_range(range);

        assert_eq!(diagnostic.code, DiagnosticCode::Parse);
        assert_eq!(diagnostic.severity, Severity::Warning);
        assert_eq!(&*diagnostic.message, "bad token");
        assert_eq!(diagnostic.resource, Some(ResourceId(3)));
        assert_eq!(diagnostic.source_range, Some(range));
    }

    #[test]
    fn pagelet_error_io_variant_has_stable_code() {
        let error = PageletError::Io(IoError::new(io::ErrorKind::NotFound, "missing"));
        assert_eq!(error.code(), DiagnosticCode::Io);
        assert!(error.to_string().contains("missing"));
    }

    #[test]
    fn pagelet_error_invalid_container_variant_has_stable_code() {
        let error = PageletError::InvalidContainer(ContainerError::new("bad zip"));
        assert_eq!(error.code(), DiagnosticCode::InvalidContainer);
        assert!(error.to_string().contains("bad zip"));
    }

    #[test]
    fn pagelet_error_invalid_package_variant_has_stable_code() {
        let error = PageletError::InvalidPackage(PackageError::new("bad opf"));
        assert_eq!(error.code(), DiagnosticCode::InvalidPackage);
        assert!(error.to_string().contains("bad opf"));
    }

    #[test]
    fn pagelet_error_unsupported_feature_variant_has_stable_code() {
        let error = PageletError::UnsupportedFeature(UnsupportedFeature::new("script"));
        assert_eq!(error.code(), DiagnosticCode::UnsupportedFeature);
        assert!(error.to_string().contains("script"));
    }

    #[test]
    fn pagelet_error_resource_limit_variant_has_stable_code() {
        let error = PageletError::ResourceLimitExceeded(ResourceLimitError::new(
            ResourceLimitKind::ZipEntries,
            10,
            11,
        ));
        assert_eq!(error.code(), DiagnosticCode::ResourceLimitExceeded);
        assert!(error.to_string().contains("ZipEntries"));
    }

    #[test]
    fn pagelet_error_parse_variant_has_stable_code() {
        let error = PageletError::Parse(ParseError::new("xml"));
        assert_eq!(error.code(), DiagnosticCode::Parse);
        assert!(error.to_string().contains("xml"));
    }

    #[test]
    fn pagelet_error_layout_variant_has_stable_code() {
        let error = PageletError::Layout(LayoutError::new("overflow"));
        assert_eq!(error.code(), DiagnosticCode::Layout);
        assert!(error.to_string().contains("overflow"));
    }

    #[test]
    fn pagelet_error_cancelled_variant_has_stable_code() {
        let error = PageletError::Cancelled;
        assert_eq!(error.code(), DiagnosticCode::Cancelled);
        assert_eq!(error.to_string(), "operation cancelled");
    }

    #[test]
    fn pagelet_error_protocol_variant_has_stable_code() {
        let error = PageletError::Protocol(ProtocolError::new("bad wire"));
        assert_eq!(error.code(), DiagnosticCode::Protocol);
        assert!(error.to_string().contains("bad wire"));
    }

    #[test]
    fn pagelet_error_internal_variant_has_stable_code() {
        let error = PageletError::Internal(InternalErrorId(99));
        assert_eq!(error.code(), DiagnosticCode::Internal);
        assert!(error.to_string().contains("99"));
    }

    #[test]
    fn resource_limits_have_mobile_defaults() {
        let limits = ResourceLimits::mobile_defaults();

        assert!(limits.max_zip_entries > 0);
        assert!(limits.max_total_decompressed_bytes >= limits.max_single_resource_bytes);
        assert!(limits.max_compression_ratio > 1);
        assert_eq!(ResourceLimits::default(), limits);
    }

    #[test]
    fn cancellation_token_is_shared_by_clones() {
        let token = CancellationToken::new();
        let clone = token.clone();

        assert!(!token.is_cancelled());
        clone.cancel();
        assert!(token.is_cancelled());
    }

    #[test]
    fn content_hash_is_deterministic_and_input_sensitive() {
        let one = ContentHash::from_bytes(b"chapter");
        let two = ContentHash::from_bytes(b"chapter");
        let three = ContentHash::from_bytes(b"other");

        assert_eq!(one, two);
        assert_ne!(one, three);
        assert_eq!(one.as_bytes().len(), 32);
    }

    #[test]
    fn engine_versions_default_to_current() {
        let versions = EngineVersions::default();

        assert_eq!(versions, EngineVersions::CURRENT);
        assert!(versions.parser_schema > 0);
        assert!(versions.disk_cache > 0);
    }
}
