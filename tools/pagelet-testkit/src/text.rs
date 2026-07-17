use pagelet::{
    core::{CancellationToken, LayoutUnit, PageletError},
    text::{
        FontSetFingerprint, LineMetrics, MeasureBatch, MeasuredBatch, MeasuredText, TextBackend,
        TextBackendId, TextBounds, TextCluster,
    },
};

/// Deterministic text metrics used by [`DeterministicTextBackend`].
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct DeterministicTextMetrics {
    pub backend_id: TextBackendId,
    pub font_fingerprint: FontSetFingerprint,
    pub advance_per_em: LayoutUnit,
    pub ascent_per_em: LayoutUnit,
    pub descent_per_em: LayoutUnit,
    pub leading_per_em: LayoutUnit,
}

impl Default for DeterministicTextMetrics {
    fn default() -> Self {
        Self {
            backend_id: TextBackendId(0x7061_6765_6c65_7401),
            font_fingerprint: FontSetFingerprint(0x666f_6e74_0000_0001),
            advance_per_em: LayoutUnit::from_raw(32),
            ascent_per_em: LayoutUnit::from_raw(51),
            descent_per_em: LayoutUnit::from_raw(13),
            leading_per_em: LayoutUnit::from_raw(10),
        }
    }
}

/// Deterministic backend with fixed cluster, fallback, bidi, and rounding rules.
#[derive(Debug, Clone, Copy, Default)]
pub struct DeterministicTextBackend {
    metrics: DeterministicTextMetrics,
}

impl DeterministicTextBackend {
    /// Create a backend with default deterministic metrics.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a backend with custom deterministic metrics.
    #[must_use]
    pub const fn with_metrics(metrics: DeterministicTextMetrics) -> Self {
        Self { metrics }
    }

    /// Return the deterministic cluster count used by measurement.
    #[must_use]
    pub fn cluster_count(text: &str) -> u32 {
        u32::try_from(text.chars().filter(|ch| !ch.is_control()).count()).unwrap_or(u32::MAX)
    }

    /// Return true when the test backend treats the text as RTL.
    #[must_use]
    pub fn has_rtl_level(text: &str) -> bool {
        text.chars()
            .any(|ch| matches!(ch, '\u{0590}'..='\u{08ff}' | '\u{fb1d}'..='\u{fdff}'))
    }

    /// Return true when fallback metrics are used.
    #[must_use]
    pub fn uses_fallback(text: &str) -> bool {
        text.chars().any(|ch| ch as u32 > 0xffff)
    }
}

impl TextBackend for DeterministicTextBackend {
    fn backend_id(&self) -> TextBackendId {
        self.metrics.backend_id
    }

    fn font_fingerprint(&self) -> FontSetFingerprint {
        self.metrics.font_fingerprint
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

            let font_raw = item.font_size.raw().max(LayoutUnit::SCALE);
            let fallback_extra = if Self::uses_fallback(&item.text) {
                font_raw / 8
            } else {
                0
            };
            let rtl_extra = if Self::has_rtl_level(&item.text) {
                font_raw / 16
            } else {
                0
            };
            let advance = scale_metric(font_raw, self.metrics.advance_per_em.raw())
                .saturating_add(fallback_extra)
                .saturating_add(rtl_extra)
                .max(1);
            let line_height = scale_metric(
                font_raw,
                self.metrics.ascent_per_em.raw()
                    + self.metrics.descent_per_em.raw()
                    + self.metrics.leading_per_em.raw(),
            )
            .max(1);
            let clusters = Self::cluster_count(&item.text);
            let max_width = item.max_width.raw().max(advance);
            let clusters_per_line = (max_width / advance).max(1);
            let line_count =
                u32::try_from((i64::from(clusters) + clusters_per_line - 1) / clusters_per_line)
                    .unwrap_or(u32::MAX)
                    .max(1);
            let natural_width = i64::from(clusters).saturating_mul(advance);
            let (lines, cluster_map, measured_width) = deterministic_lines(
                &item.text,
                advance,
                LayoutUnit::from_raw(line_height),
                LayoutUnit::from_raw(max_width),
            );

            results.push(MeasuredText::new(
                item.id,
                item.request_fingerprint,
                measured_width.max(LayoutUnit::from_raw(natural_width.min(max_width))),
                LayoutUnit::from_raw(i64::from(line_count).saturating_mul(line_height)),
                u32::try_from(item.text.len()).unwrap_or(u32::MAX),
                lines,
                cluster_map,
                item.request_fingerprint ^ self.metrics.font_fingerprint.0,
            ));
        }

        Ok(MeasuredBatch::new(
            self.backend_id(),
            self.font_fingerprint(),
            results,
        ))
    }
}

fn scale_metric(font_raw: i64, per_em: i64) -> i64 {
    (font_raw.saturating_mul(per_em) + (LayoutUnit::SCALE / 2)) / LayoutUnit::SCALE
}

fn deterministic_lines(
    text: &str,
    advance: i64,
    line_height: LayoutUnit,
    max_width: LayoutUnit,
) -> (Vec<LineMetrics>, Vec<TextCluster>, LayoutUnit) {
    let advance = LayoutUnit::from_raw(advance.max(1));
    let clusters_per_line = (max_width.raw().max(advance.raw()) / advance.raw()).max(1);
    let mut lines = Vec::new();
    let mut clusters = Vec::new();
    let mut line_start = 0_usize;
    let mut line_width = LayoutUnit::ZERO;
    let mut line_index = 0_u32;
    let mut cluster_x = LayoutUnit::ZERO;
    let mut count_on_line = 0_i64;
    let mut max_line_width = LayoutUnit::ZERO;

    for (offset, ch) in text.char_indices() {
        if count_on_line >= clusters_per_line && offset > line_start {
            push_line(
                &mut lines,
                line_start,
                offset,
                line_width,
                line_height,
                false,
            );
            max_line_width = max_line_width.max(line_width);
            line_start = offset;
            line_width = LayoutUnit::ZERO;
            cluster_x = LayoutUnit::ZERO;
            count_on_line = 0;
            line_index = line_index.saturating_add(1);
        }
        let end = offset + ch.len_utf8();
        clusters.push(TextCluster {
            text_start: u32::try_from(offset).unwrap_or(u32::MAX),
            text_end: u32::try_from(end).unwrap_or(u32::MAX),
            line_index,
            x_start: cluster_x,
            x_end: cluster_x + advance,
        });
        line_width += advance;
        cluster_x += advance;
        count_on_line += 1;
    }

    if line_start < text.len() || lines.is_empty() {
        push_line(
            &mut lines,
            line_start,
            text.len(),
            line_width,
            line_height,
            false,
        );
        max_line_width = max_line_width.max(line_width);
    }

    (lines, clusters, max_line_width)
}

fn push_line(
    lines: &mut Vec<LineMetrics>,
    start: usize,
    end: usize,
    width: LayoutUnit,
    line_height: LayoutUnit,
    hard_break: bool,
) {
    lines.push(LineMetrics {
        text_start: u32::try_from(start).unwrap_or(u32::MAX),
        text_end: u32::try_from(end).unwrap_or(u32::MAX),
        baseline: LayoutUnit::from_raw((line_height.raw() * 4) / 5),
        ascent: LayoutUnit::from_raw((line_height.raw() * 4) / 5),
        descent: LayoutUnit::from_raw(line_height.raw() / 5),
        line_height,
        width,
        ink_bounds: TextBounds {
            x: LayoutUnit::ZERO,
            y: LayoutUnit::ZERO,
            width,
            height: line_height,
        },
        hard_break,
    });
}

#[cfg(test)]
mod tests {
    use pagelet::{core::LayoutUnit, text::MeasureRequest};

    use super::*;

    #[test]
    fn deterministic_backend_replays_measurements() {
        let backend = DeterministicTextBackend::new();
        let batch = MeasureBatch::new(vec![MeasureRequest::new(
            7,
            "Hello مرحبا",
            LayoutUnit::from_px(16),
            LayoutUnit::from_px(80),
        )]);
        let cancel = CancellationToken::new();

        let first = backend.measure_batch(&batch, &cancel).expect("measure");
        let second = backend.measure_batch(&batch, &cancel).expect("measure");

        assert_eq!(first, second);
        assert_eq!(
            backend.backend_id(),
            DeterministicTextMetrics::default().backend_id
        );
        assert_eq!(
            backend.font_fingerprint(),
            DeterministicTextMetrics::default().font_fingerprint
        );
    }
}
