//! Text measurement and shaping contracts.

use std::sync::Arc;

use crate::core::{CancellationToken, LayoutUnit, PageletError};

/// Stable text backend identifier.
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq, Hash)]
pub struct TextBackendId(pub u64);

/// Stable fingerprint of the font set used by a backend.
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq, Hash)]
pub struct FontSetFingerprint(pub u64);

/// One text measurement request.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct MeasureRequest {
    /// Host-defined request id echoed back in the measurement result.
    pub id: u32,
    /// Text to measure.
    pub text: Arc<str>,
    /// Font size in logical pixels.
    pub font_size: LayoutUnit,
    /// Maximum line width in logical pixels.
    pub max_width: LayoutUnit,
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
        Self {
            id,
            text: text.into(),
            font_size,
            max_width,
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
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
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
