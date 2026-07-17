//! Text measurement and shaping contracts.

use std::{collections::BTreeMap, sync::Arc};

use crate::core::{CancellationToken, FontId, LayoutUnit, PageletError};

/// Stable text backend identifier.
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq, Hash)]
pub struct TextBackendId(pub u64);

/// Stable fingerprint of the font set used by a backend.
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq, Hash)]
pub struct FontSetFingerprint(pub u64);

/// Host or engine text direction used for measurement.
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq, Hash)]
pub enum TextDirection {
    /// Let the backend infer paragraph direction from content and locale.
    #[default]
    Auto,
    /// Left-to-right paragraph direction.
    Ltr,
    /// Right-to-left paragraph direction.
    Rtl,
}

/// Font slant requested by a style run.
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq, Hash)]
pub enum FontStyle {
    /// Upright roman style.
    #[default]
    Normal,
    /// Italic style.
    Italic,
    /// Oblique style.
    Oblique,
}

/// Physical or host-resolved font descriptor.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct FontDescriptor {
    /// Stable pagelet font id, when known.
    pub font_id: Option<FontId>,
    /// Host-visible family name or fallback family.
    pub family: Arc<str>,
    /// CSS-compatible numeric weight.
    pub weight: u16,
    /// Font slant.
    pub style: FontStyle,
    /// CSS-compatible stretch percentage.
    pub stretch: u16,
    /// Stable fingerprint for the underlying font bytes or host font entry.
    pub fingerprint: FontSetFingerprint,
}

impl FontDescriptor {
    /// Create a descriptor for a named family.
    #[must_use]
    pub fn new(family: impl Into<Arc<str>>, fingerprint: FontSetFingerprint) -> Self {
        Self {
            font_id: None,
            family: family.into(),
            weight: 400,
            style: FontStyle::Normal,
            stretch: 100,
            fingerprint,
        }
    }
}

impl Default for FontDescriptor {
    fn default() -> Self {
        Self::new("serif", FontSetFingerprint::default())
    }
}

/// Ordered fallback chain supplied to a text backend.
#[derive(Debug, Default, Clone, Eq, PartialEq, Hash)]
pub struct FontFallbackChain {
    /// Primary requested font.
    pub primary: FontDescriptor,
    /// Ordered fallback candidates.
    pub fallbacks: Vec<FontDescriptor>,
}

/// Per-run style inside a measurement request.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct TextStyleRun {
    /// UTF-8 byte offset into [`MeasureRequest::text`].
    pub start: u32,
    /// Exclusive UTF-8 byte offset into [`MeasureRequest::text`].
    pub end: u32,
    /// Font size in logical pixels.
    pub font_size: LayoutUnit,
    /// Additional advance between text clusters.
    pub letter_spacing: LayoutUnit,
    /// Requested font fallback chain.
    pub fonts: FontFallbackChain,
}

impl TextStyleRun {
    /// Create a single style run.
    #[must_use]
    pub fn new(start: u32, end: u32, font_size: LayoutUnit, fonts: FontFallbackChain) -> Self {
        Self {
            start,
            end,
            font_size,
            letter_spacing: LayoutUnit::ZERO,
            fonts,
        }
    }
}

/// Paragraph-local ink bounds reported by the shaping adapter.
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq, Hash)]
pub struct TextBounds {
    /// Left edge relative to the line origin.
    pub x: LayoutUnit,
    /// Top edge relative to the line top.
    pub y: LayoutUnit,
    /// Ink width, which may exceed the line advance.
    pub width: LayoutUnit,
    /// Ink height.
    pub height: LayoutUnit,
}

/// Height and strut handling requested by layout.
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq, Hash)]
pub enum HeightBehavior {
    /// Use backend natural line metrics.
    #[default]
    Natural,
    /// Force at least the requested strut height.
    IncludeStrut,
    /// Use tight line boxes where the backend supports them.
    Tight,
}

/// Optional strut metrics for paragraph measurement.
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq, Hash)]
pub struct StrutStyle {
    /// Minimum ascent.
    pub ascent: LayoutUnit,
    /// Minimum descent.
    pub descent: LayoutUnit,
    /// Additional leading.
    pub leading: LayoutUnit,
}

/// One text measurement request.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct MeasureRequest {
    /// Host-defined request id echoed back in the measurement result.
    pub id: u32,
    /// Stable paragraph id inside the current layout request.
    pub paragraph_id: u32,
    /// Text to measure.
    pub text: Arc<str>,
    /// UTF-8 byte range inside `text` to shape.
    pub text_range: std::ops::Range<u32>,
    /// Style runs inside `text_range`.
    pub style_runs: Vec<TextStyleRun>,
    /// Font size in logical pixels.
    pub font_size: LayoutUnit,
    /// Maximum line width in logical pixels.
    pub max_width: LayoutUnit,
    /// Alias used by the host-measured protocol.
    pub available_width: LayoutUnit,
    /// BCP-47 locale hint.
    pub locale: Arc<str>,
    /// Paragraph direction hint.
    pub direction: TextDirection,
    /// Text scale as fixed-point where 64 means 1.0.
    pub text_scale: LayoutUnit,
    /// Requested fallback chain.
    pub font_candidates: FontFallbackChain,
    /// Optional paragraph strut.
    pub strut: StrutStyle,
    /// Line-height behavior.
    pub height_behavior: HeightBehavior,
    /// Stable request fingerprint supplied by the layout engine.
    pub request_fingerprint: u64,
}

impl MeasureRequest {
    /// Create a measurement request.
    #[must_use]
    pub fn new(
        id: u32,
        text: impl Into<Arc<str>>,
        font_size: LayoutUnit,
        max_width: LayoutUnit,
    ) -> Self {
        let text = text.into();
        let end = u32::try_from(text.len()).unwrap_or(u32::MAX);
        let fonts = FontFallbackChain::default();
        Self {
            id,
            paragraph_id: id,
            text,
            text_range: 0..end,
            style_runs: vec![TextStyleRun::new(0, end, font_size, fonts.clone())],
            font_size,
            max_width,
            available_width: max_width,
            locale: Arc::from("und"),
            direction: TextDirection::Auto,
            text_scale: LayoutUnit::from_raw(LayoutUnit::SCALE),
            font_candidates: fonts,
            strut: StrutStyle::default(),
            height_behavior: HeightBehavior::Natural,
            request_fingerprint: u64::from(id),
        }
    }
}

/// Batch of measurement requests.
#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct MeasureBatch {
    /// Requests to measure in a single backend call.
    pub requests: Vec<MeasureRequest>,
}

impl MeasureBatch {
    /// Create a batch from requests.
    #[must_use]
    pub fn new(requests: Vec<MeasureRequest>) -> Self {
        Self { requests }
    }
}

/// Measurement result for one request.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct MeasuredText {
    /// Request id copied from [`MeasureRequest::id`].
    pub request_id: u32,
    /// Request fingerprint copied from [`MeasureRequest::request_fingerprint`].
    pub request_fingerprint: u64,
    /// Measured width.
    pub width: LayoutUnit,
    /// Measured height.
    pub height: LayoutUnit,
    /// Number of lines after wrapping.
    pub line_count: u32,
    /// Count of UTF-8 bytes measured.
    pub utf8_len: u32,
    /// Line boxes and corresponding UTF-8 ranges.
    pub lines: Vec<LineMetrics>,
    /// Cluster-to-offset mapping for hit testing and selections.
    pub clusters: Vec<TextCluster>,
    /// Stable fingerprint for this measured output.
    pub measurement_fingerprint: u64,
}

impl MeasuredText {
    /// Create measured text from line and cluster details.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        request_id: u32,
        request_fingerprint: u64,
        width: LayoutUnit,
        height: LayoutUnit,
        utf8_len: u32,
        lines: Vec<LineMetrics>,
        clusters: Vec<TextCluster>,
        measurement_fingerprint: u64,
    ) -> Self {
        Self {
            request_id,
            request_fingerprint,
            width,
            height,
            line_count: u32::try_from(lines.len()).unwrap_or(u32::MAX),
            utf8_len,
            lines,
            clusters,
            measurement_fingerprint,
        }
    }
}

/// One measured line range and metrics.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct LineMetrics {
    /// UTF-8 byte start inside the measured text.
    pub text_start: u32,
    /// UTF-8 byte end inside the measured text.
    pub text_end: u32,
    /// Baseline position from the line top.
    pub baseline: LayoutUnit,
    /// Ascent above baseline.
    pub ascent: LayoutUnit,
    /// Descent below baseline.
    pub descent: LayoutUnit,
    /// Full line box height.
    pub line_height: LayoutUnit,
    /// Measured line width.
    pub width: LayoutUnit,
    /// Actual glyph ink relative to the line origin and line top.
    pub ink_bounds: TextBounds,
    /// True when the range ends at a hard line break.
    pub hard_break: bool,
}

/// One text cluster range and horizontal bounds inside its line.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct TextCluster {
    /// UTF-8 byte start inside the measured text.
    pub text_start: u32,
    /// UTF-8 byte end inside the measured text.
    pub text_end: u32,
    /// Line index containing the cluster.
    pub line_index: u32,
    /// Cluster start x in logical pixels.
    pub x_start: LayoutUnit,
    /// Cluster end x in logical pixels.
    pub x_end: LayoutUnit,
}

/// Batch of measured text results.
#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct MeasuredBatch {
    /// Stable identity of the host backend that produced the results.
    pub backend_id: TextBackendId,
    /// Stable fingerprint of the font set used for measurement.
    pub font_fingerprint: FontSetFingerprint,
    /// Measurement results in request order.
    pub results: Vec<MeasuredText>,
}

impl MeasuredBatch {
    /// Create a batch from results.
    #[must_use]
    pub fn new(
        backend_id: TextBackendId,
        font_fingerprint: FontSetFingerprint,
        results: Vec<MeasuredText>,
    ) -> Self {
        Self {
            backend_id,
            font_fingerprint,
            results,
        }
    }

    /// Find a result by request id.
    #[must_use]
    pub fn get(&self, request_id: u32) -> Option<&MeasuredText> {
        self.results
            .iter()
            .find(|result| result.request_id == request_id)
    }
}

/// Host-provided text backend after one complete measurement batch is submitted.
///
/// Construction validates backend identity, request identity, and all UTF-8 line
/// and cluster ranges before the results can reach layout.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct HostMeasuredTextBackend {
    backend_id: TextBackendId,
    font_fingerprint: FontSetFingerprint,
    measurements: BTreeMap<u32, (MeasureRequest, MeasuredText)>,
}

impl HostMeasuredTextBackend {
    /// Validate a host response and create a backend that serves those metrics.
    pub fn new(requested: &MeasureBatch, measured: MeasuredBatch) -> Result<Self, PageletError> {
        let mut requested_by_id = BTreeMap::new();
        for request in &requested.requests {
            if requested_by_id.insert(request.id, request).is_some() {
                return Err(protocol_error(
                    "measurement batch contains duplicate request ids",
                ));
            }
        }

        let mut measurements = BTreeMap::new();
        for result in measured.results {
            let Some(request) = requested_by_id.get(&result.request_id) else {
                return Err(protocol_error(
                    "measured batch contains an unknown request id",
                ));
            };
            validate_measured_text(request, &result)?;
            if measurements
                .insert(result.request_id, ((*request).clone(), result))
                .is_some()
            {
                return Err(protocol_error(
                    "measured batch contains duplicate request ids",
                ));
            }
        }
        if measurements.len() != requested_by_id.len() {
            return Err(protocol_error(
                "measured batch does not contain every requested measurement",
            ));
        }

        Ok(Self {
            backend_id: measured.backend_id,
            font_fingerprint: measured.font_fingerprint,
            measurements,
        })
    }
}

impl TextBackend for HostMeasuredTextBackend {
    fn backend_id(&self) -> TextBackendId {
        self.backend_id
    }

    fn font_fingerprint(&self) -> FontSetFingerprint {
        self.font_fingerprint
    }

    fn measure_batch(
        &self,
        request: &MeasureBatch,
        cancel: &CancellationToken,
    ) -> Result<MeasuredBatch, PageletError> {
        let mut results = Vec::with_capacity(request.requests.len());
        for item in &request.requests {
            if cancel.is_cancelled() {
                return Err(PageletError::Cancelled);
            }
            let Some((expected, measured)) = self.measurements.get(&item.id) else {
                return Err(protocol_error(
                    "layout requested text that was not present in the host batch",
                ));
            };
            if expected != item {
                return Err(protocol_error(
                    "layout measurement request differs from the prepared host batch",
                ));
            }
            results.push(measured.clone());
        }
        Ok(MeasuredBatch::new(
            self.backend_id,
            self.font_fingerprint,
            results,
        ))
    }
}

fn validate_measured_text(
    request: &MeasureRequest,
    measured: &MeasuredText,
) -> Result<(), PageletError> {
    if measured.request_fingerprint != request.request_fingerprint {
        return Err(protocol_error(
            "measured text request fingerprint does not match",
        ));
    }
    let text_start = usize::try_from(request.text_range.start).unwrap_or(usize::MAX);
    let text_end = usize::try_from(request.text_range.end).unwrap_or(usize::MAX);
    let Some(text) = request.text.get(text_start..text_end) else {
        return Err(protocol_error(
            "measurement request has an invalid text range",
        ));
    };
    if measured.utf8_len != u32::try_from(text.len()).unwrap_or(u32::MAX) {
        return Err(protocol_error(
            "measured text length does not match request",
        ));
    }
    if measured.line_count != u32::try_from(measured.lines.len()).unwrap_or(u32::MAX) {
        return Err(protocol_error(
            "measured text line count does not match lines",
        ));
    }

    let mut previous_line_end = 0_u32;
    for line in &measured.lines {
        validate_relative_range(text, line.text_start, line.text_end, "line")?;
        if line.text_start < previous_line_end {
            return Err(protocol_error("measured text lines are not ordered"));
        }
        previous_line_end = line.text_end;
        if line.width.raw() < 0
            || line.ascent.raw() < 0
            || line.descent.raw() < 0
            || line.line_height.raw() < 0
            || line.baseline.raw() < 0
            || line.ink_bounds.width.raw() < 0
            || line.ink_bounds.height.raw() < 0
        {
            return Err(protocol_error("measured line metrics must not be negative"));
        }
    }

    let mut previous_cluster: Option<(u32, LayoutUnit)> = None;
    for cluster in &measured.clusters {
        validate_relative_range(text, cluster.text_start, cluster.text_end, "cluster")?;
        if usize::try_from(cluster.line_index).unwrap_or(usize::MAX) >= measured.lines.len() {
            return Err(protocol_error("measured cluster refers to an unknown line"));
        }
        if cluster.x_start.raw() < 0 || cluster.x_end < cluster.x_start {
            return Err(protocol_error("measured cluster bounds are invalid"));
        }
        if let Some((previous_line, previous_x_start)) = previous_cluster {
            if cluster.line_index < previous_line
                || (cluster.line_index == previous_line && cluster.x_start < previous_x_start)
            {
                return Err(protocol_error("measured clusters are not visually ordered"));
            }
        }
        previous_cluster = Some((cluster.line_index, cluster.x_start));
    }
    Ok(())
}

fn validate_relative_range(
    text: &str,
    start: u32,
    end: u32,
    kind: &'static str,
) -> Result<(), PageletError> {
    let start = usize::try_from(start).unwrap_or(usize::MAX);
    let end = usize::try_from(end).unwrap_or(usize::MAX);
    if start <= end
        && end <= text.len()
        && text.is_char_boundary(start)
        && text.is_char_boundary(end)
    {
        Ok(())
    } else {
        Err(protocol_error(match kind {
            "line" => "measured line range is not a valid UTF-8 range",
            _ => "measured cluster range is not a valid UTF-8 range",
        }))
    }
}

fn protocol_error(message: &'static str) -> PageletError {
    PageletError::Protocol(crate::core::ProtocolError::new(message))
}

/// Text measurement backend used by the layout engine.
pub trait TextBackend: Send + Sync {
    /// Stable backend id.
    fn backend_id(&self) -> TextBackendId;

    /// Stable font fingerprint for cache keys.
    fn font_fingerprint(&self) -> FontSetFingerprint;

    /// Measure all requests in one batch.
    fn measure_batch(
        &self,
        request: &MeasureBatch,
        cancel: &CancellationToken,
    ) -> Result<MeasuredBatch, PageletError>;
}

/// Deterministic fallback backend used by CLI pagination and unit tests.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct DefaultTextBackend {
    backend_id: TextBackendId,
    font_fingerprint: FontSetFingerprint,
}

impl DefaultTextBackend {
    /// Create the deterministic default backend.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            backend_id: TextBackendId(0x7061_6765_6c65_7403),
            font_fingerprint: FontSetFingerprint(0x666f_6e74_0000_0003),
        }
    }
}

impl Default for DefaultTextBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl TextBackend for DefaultTextBackend {
    fn backend_id(&self) -> TextBackendId {
        self.backend_id
    }

    fn font_fingerprint(&self) -> FontSetFingerprint {
        self.font_fingerprint
    }

    fn measure_batch(
        &self,
        request: &MeasureBatch,
        cancel: &CancellationToken,
    ) -> Result<MeasuredBatch, PageletError> {
        let mut results = Vec::with_capacity(request.requests.len());
        for item in &request.requests {
            if cancel.is_cancelled() {
                return Err(PageletError::Cancelled);
            }
            results.push(measure_deterministic(item, self.font_fingerprint));
        }
        Ok(MeasuredBatch::new(
            self.backend_id,
            self.font_fingerprint,
            results,
        ))
    }
}

fn measure_deterministic(
    item: &MeasureRequest,
    font_fingerprint: FontSetFingerprint,
) -> MeasuredText {
    let primary_run = item.style_runs.first();
    let font_raw = primary_run
        .map_or(item.font_size, |run| run.font_size)
        .raw()
        .max(LayoutUnit::SCALE);
    let mut ascent = LayoutUnit::from_raw((font_raw * 4) / 5);
    let mut descent = LayoutUnit::from_raw((font_raw / 5).max(1));
    let mut leading = LayoutUnit::from_raw((font_raw / 8).max(1));
    if item.height_behavior == HeightBehavior::IncludeStrut {
        ascent = ascent.max(item.strut.ascent);
        descent = descent.max(item.strut.descent);
        leading = leading.max(item.strut.leading);
    }
    let line_height = LayoutUnit::from_raw(ascent.raw() + descent.raw() + leading.raw());
    let letter_spacing = primary_run.map_or(LayoutUnit::ZERO, |run| run.letter_spacing);
    let advance =
        (LayoutUnit::from_raw((font_raw / 2).max(1)) + letter_spacing).max(LayoutUnit::from_raw(1));
    let width_limit = item.max_width.raw().max(advance.raw());
    let mut lines = Vec::new();
    let mut clusters = Vec::new();

    let text_start = usize::try_from(item.text_range.start).unwrap_or(0);
    let text_end = usize::try_from(item.text_range.end)
        .unwrap_or(item.text.len())
        .min(item.text.len());
    let text = item.text.get(text_start..text_end).unwrap_or(&item.text);
    let mut line_start = 0_usize;
    let mut line_width = LayoutUnit::ZERO;
    let mut line_index = 0_u32;
    let mut cluster_x = LayoutUnit::ZERO;
    let mut max_line_width = LayoutUnit::ZERO;

    for (relative_offset, ch) in text.char_indices() {
        let ch_end = relative_offset + ch.len_utf8();
        let hard_break = ch == '\n';
        let next_width = if hard_break {
            line_width
        } else {
            line_width + advance
        };
        if !hard_break && next_width.raw() > width_limit && relative_offset > line_start {
            lines.push(line_metrics(
                line_start,
                relative_offset,
                line_width,
                ascent,
                descent,
                line_height,
                false,
            ));
            max_line_width = max_line_width.max(line_width);
            line_index = line_index.saturating_add(1);
            line_start = relative_offset;
            line_width = LayoutUnit::ZERO;
            cluster_x = LayoutUnit::ZERO;
        }

        if !hard_break {
            clusters.push(TextCluster {
                text_start: u32::try_from(relative_offset).unwrap_or(u32::MAX),
                text_end: u32::try_from(ch_end).unwrap_or(u32::MAX),
                line_index,
                x_start: cluster_x,
                x_end: cluster_x + advance,
            });
            cluster_x += advance;
            line_width += advance;
        }

        if hard_break {
            lines.push(line_metrics(
                line_start,
                relative_offset,
                line_width,
                ascent,
                descent,
                line_height,
                true,
            ));
            max_line_width = max_line_width.max(line_width);
            line_index = line_index.saturating_add(1);
            line_start = ch_end;
            line_width = LayoutUnit::ZERO;
            cluster_x = LayoutUnit::ZERO;
        }
    }

    if line_start < text.len() || lines.is_empty() {
        lines.push(line_metrics(
            line_start,
            text.len(),
            line_width,
            ascent,
            descent,
            line_height,
            false,
        ));
        max_line_width = max_line_width.max(line_width);
    }

    let height = LayoutUnit::from_raw(
        line_height
            .raw()
            .saturating_mul(i64::try_from(lines.len()).unwrap_or(i64::MAX)),
    );
    let fingerprint = item.request_fingerprint.wrapping_mul(1_099_511_628_211)
        ^ font_fingerprint.0
        ^ u64::try_from(text.len()).unwrap_or(u64::MAX);

    MeasuredText::new(
        item.id,
        item.request_fingerprint,
        max_line_width,
        height,
        u32::try_from(text.len()).unwrap_or(u32::MAX),
        lines,
        clusters,
        fingerprint,
    )
}

fn line_metrics(
    start: usize,
    end: usize,
    width: LayoutUnit,
    ascent: LayoutUnit,
    descent: LayoutUnit,
    line_height: LayoutUnit,
    hard_break: bool,
) -> LineMetrics {
    LineMetrics {
        text_start: u32::try_from(start).unwrap_or(u32::MAX),
        text_end: u32::try_from(end).unwrap_or(u32::MAX),
        baseline: ascent + LayoutUnit::from_raw((line_height - ascent - descent).raw() / 2),
        ascent,
        descent,
        line_height,
        width,
        ink_bounds: TextBounds {
            x: LayoutUnit::ZERO,
            y: LayoutUnit::from_raw((line_height - ascent - descent).raw() / 2),
            width,
            height: ascent + descent,
        },
        hard_break,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_backend_returns_line_ranges_and_clusters() {
        let backend = DefaultTextBackend::new();
        let batch = MeasureBatch::new(vec![MeasureRequest::new(
            1,
            "hello pagelet",
            LayoutUnit::from_px(16),
            LayoutUnit::from_px(40),
        )]);
        let measured = backend
            .measure_batch(&batch, &CancellationToken::new())
            .expect("measure");
        let result = measured.get(1).expect("result");

        assert!(result.line_count > 1);
        assert!(!result.lines.is_empty());
        assert!(!result.clusters.is_empty());
        assert_eq!(result.utf8_len, "hello pagelet".len() as u32);
    }

    #[test]
    fn host_backend_rejects_stale_request_fingerprints() {
        let requested = MeasureBatch::new(vec![MeasureRequest::new(
            7,
            "host measured",
            LayoutUnit::from_px(16),
            LayoutUnit::from_px(120),
        )]);
        let fallback = DefaultTextBackend::new();
        let mut measured = fallback
            .measure_batch(&requested, &CancellationToken::new())
            .expect("measure");
        measured.results[0].request_fingerprint ^= 1;

        let error = HostMeasuredTextBackend::new(&requested, measured).expect_err("stale batch");
        assert_eq!(error.code(), crate::core::DiagnosticCode::Protocol);
        assert!(error.to_string().contains("fingerprint"));
    }

    #[test]
    fn host_backend_rejects_incomplete_batches() {
        let requested = MeasureBatch::new(vec![MeasureRequest::new(
            9,
            "missing",
            LayoutUnit::from_px(16),
            LayoutUnit::from_px(120),
        )]);
        let measured = MeasuredBatch::new(TextBackendId(41), FontSetFingerprint(42), Vec::new());

        let error = HostMeasuredTextBackend::new(&requested, measured).expect_err("missing result");
        assert_eq!(error.code(), crate::core::DiagnosticCode::Protocol);
        assert!(error.to_string().contains("every requested"));
    }

    #[test]
    fn host_backend_accepts_visually_ordered_rtl_clusters() {
        let mut request =
            MeasureRequest::new(11, "אב", LayoutUnit::from_px(16), LayoutUnit::from_px(120));
        request.direction = TextDirection::Rtl;
        let requested = MeasureBatch::new(vec![request.clone()]);
        let line_height = LayoutUnit::from_px(20);
        let measured = MeasuredBatch::new(
            TextBackendId(51),
            FontSetFingerprint(52),
            vec![MeasuredText::new(
                request.id,
                request.request_fingerprint,
                LayoutUnit::from_px(16),
                line_height,
                4,
                vec![LineMetrics {
                    text_start: 0,
                    text_end: 4,
                    baseline: LayoutUnit::from_px(15),
                    ascent: LayoutUnit::from_px(15),
                    descent: LayoutUnit::from_px(5),
                    line_height,
                    width: LayoutUnit::from_px(16),
                    ink_bounds: TextBounds {
                        x: LayoutUnit::ZERO,
                        y: LayoutUnit::ZERO,
                        width: LayoutUnit::from_px(16),
                        height: line_height,
                    },
                    hard_break: false,
                }],
                vec![
                    TextCluster {
                        text_start: 2,
                        text_end: 4,
                        line_index: 0,
                        x_start: LayoutUnit::ZERO,
                        x_end: LayoutUnit::from_px(8),
                    },
                    TextCluster {
                        text_start: 0,
                        text_end: 2,
                        line_index: 0,
                        x_start: LayoutUnit::from_px(8),
                        x_end: LayoutUnit::from_px(16),
                    },
                ],
                53,
            )],
        );

        HostMeasuredTextBackend::new(&requested, measured).expect("valid RTL clusters");
    }
}
