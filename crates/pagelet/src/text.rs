//! Text measurement and shaping contracts.

use std::sync::Arc;

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
#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct FontFallbackChain {
    /// Primary requested font.
    pub primary: FontDescriptor,
    /// Ordered fallback candidates.
    pub fallbacks: Vec<FontDescriptor>,
}

/// Per-run style inside a measurement request.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct TextStyleRun {
    /// UTF-8 byte offset into [`MeasureRequest::text`].
    pub start: u32,
    /// Exclusive UTF-8 byte offset into [`MeasureRequest::text`].
    pub end: u32,
    /// Font size in logical pixels.
    pub font_size: LayoutUnit,
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
            fonts,
        }
    }
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
    pub fn new(
        request_id: u32,
        width: LayoutUnit,
        height: LayoutUnit,
        utf8_len: u32,
        lines: Vec<LineMetrics>,
        clusters: Vec<TextCluster>,
        measurement_fingerprint: u64,
    ) -> Self {
        Self {
            request_id,
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
    /// Measurement results in request order.
    pub results: Vec<MeasuredText>,
}

impl MeasuredBatch {
    /// Create a batch from results.
    #[must_use]
    pub fn new(results: Vec<MeasuredText>) -> Self {
        Self { results }
    }

    /// Find a result by request id.
    #[must_use]
    pub fn get(&self, request_id: u32) -> Option<&MeasuredText> {
        self.results
            .iter()
            .find(|result| result.request_id == request_id)
    }
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
        Ok(MeasuredBatch::new(results))
    }
}

fn measure_deterministic(
    item: &MeasureRequest,
    font_fingerprint: FontSetFingerprint,
) -> MeasuredText {
    let font_raw = item.font_size.raw().max(LayoutUnit::SCALE);
    let ascent = LayoutUnit::from_raw((font_raw * 4) / 5);
    let descent = LayoutUnit::from_raw((font_raw / 5).max(1));
    let leading = LayoutUnit::from_raw((font_raw / 8).max(1));
    let line_height = LayoutUnit::from_raw(ascent.raw() + descent.raw() + leading.raw());
    let advance = LayoutUnit::from_raw((font_raw / 2).max(1));
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
        baseline: ascent,
        ascent,
        descent,
        line_height,
        width,
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
}
