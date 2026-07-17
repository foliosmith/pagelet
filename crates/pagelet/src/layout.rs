//! Deterministic layout, fragmentation, pagination, and page scenes.

use std::{collections::BTreeMap, sync::Arc};

use crate::{
    core::{
        CancellationToken, ContentHash, Diagnostic, DiagnosticCode, LayoutError, LayoutUnit,
        NodeId, PageletError, ResourceLimitError, ResourceLimitKind, ResourceLimits, Severity,
        SourceRange, TextAffinity, TextAnchor,
    },
    document::{
        BlockText, ChapterIr, ComputedStyle as DocumentComputedStyle, DocumentNode, ImageNode,
        LinkKind, LinkTarget, StyleTable,
    },
    text::{
        HeightBehavior, HostMeasuredTextBackend, LineMetrics, MeasureBatch, MeasureRequest,
        MeasuredBatch, MeasuredText, StrutStyle, TextBackend, TextCluster, TextDirection,
    },
};

/// Layout constraints for one reflowable page.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub struct LayoutConstraints {
    /// Viewport width.
    pub viewport_width: LayoutUnit,
    /// Viewport height.
    pub viewport_height: LayoutUnit,
    /// Left content inset.
    pub margin_start: LayoutUnit,
    /// Right content inset.
    pub margin_end: LayoutUnit,
    /// Top content inset.
    pub margin_top: LayoutUnit,
    /// Bottom content inset.
    pub margin_bottom: LayoutUnit,
}

impl LayoutConstraints {
    /// Create constraints with zero margins.
    #[must_use]
    pub const fn new(viewport_width: LayoutUnit, viewport_height: LayoutUnit) -> Self {
        Self {
            viewport_width,
            viewport_height,
            margin_start: LayoutUnit::ZERO,
            margin_end: LayoutUnit::ZERO,
            margin_top: LayoutUnit::ZERO,
            margin_bottom: LayoutUnit::ZERO,
        }
    }

    /// Create constraints with symmetric margins.
    #[must_use]
    pub const fn with_margin(mut self, margin: LayoutUnit) -> Self {
        self.margin_start = margin;
        self.margin_end = margin;
        self.margin_top = margin;
        self.margin_bottom = margin;
        self
    }

    /// Width available to the layout tree.
    #[must_use]
    pub fn content_width(self) -> LayoutUnit {
        let width = self.viewport_width - self.margin_start - self.margin_end;
        if width.raw() <= 0 {
            LayoutUnit::ZERO
        } else {
            width
        }
    }

    /// Height available to the layout tree.
    #[must_use]
    pub fn content_height(self) -> LayoutUnit {
        let height = self.viewport_height - self.margin_top - self.margin_bottom;
        if height.raw() <= 0 {
            LayoutUnit::ZERO
        } else {
            height
        }
    }
}

impl Default for LayoutConstraints {
    fn default() -> Self {
        Self::new(LayoutUnit::from_px(360), LayoutUnit::from_px(640))
            .with_margin(LayoutUnit::from_px(24))
    }
}

/// Pagination options.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct LayoutOptions {
    /// Page constraints.
    pub constraints: LayoutConstraints,
    /// Resource limits enforced by layout.
    pub limits: ResourceLimits,
    /// Hard page cap for one pagination request.
    pub max_pages: u32,
}

impl LayoutOptions {
    /// Create options from constraints.
    #[must_use]
    pub fn new(constraints: LayoutConstraints) -> Self {
        Self {
            constraints,
            ..Self::default()
        }
    }
}

impl Default for LayoutOptions {
    fn default() -> Self {
        Self {
            constraints: LayoutConstraints::default(),
            limits: ResourceLimits::default(),
            max_pages: 10_000,
        }
    }
}

/// Layout context passed to fragmentable blocks.
pub struct LayoutContext<'a> {
    /// Chapter being laid out.
    pub chapter: &'a ChapterIr,
    /// Layout options.
    pub options: LayoutOptions,
    /// Text backend used for paragraph measurement.
    pub text_backend: &'a dyn TextBackend,
    /// Cancellation token for long-running layout.
    pub cancel: &'a CancellationToken,
}

/// Layout-ready style subset.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub struct ComputedLayoutStyle {
    /// Font size.
    pub font_size: LayoutUnit,
    /// Line height.
    pub line_height: LayoutUnit,
    /// Space before block.
    pub margin_before: LayoutUnit,
    /// Space after block.
    pub margin_after: LayoutUnit,
    /// Start padding.
    pub padding_start: LayoutUnit,
    /// End padding.
    pub padding_end: LayoutUnit,
    /// First-line indent.
    pub text_indent: LayoutUnit,
    /// Container/list/blockquote indent.
    pub block_indent: LayoutUnit,
    /// Text alignment.
    pub alignment: TextAlignment,
    /// Whether this block should stay with the next block.
    pub keep_with_next: bool,
    /// Whether this block forces a break before itself.
    pub break_before: bool,
    /// Whether this block forces a break after itself.
    pub break_after: bool,
    /// Break-inside behavior.
    pub break_inside: BreakInside,
}

impl ComputedLayoutStyle {
    fn from_document(
        styles: &StyleTable,
        style_id: crate::core::StyleId,
        kind: BlockKind,
        depth: u32,
    ) -> Self {
        let mut style = Self::for_kind(kind, depth);
        if let Some(document_style) = styles.get(style_id) {
            style.apply_document_style(document_style);
        }
        style
    }

    fn for_kind(kind: BlockKind, depth: u32) -> Self {
        let depth_indent = LayoutUnit::from_px(i64::from(depth) * 18);
        match kind {
            BlockKind::Heading(level) => {
                let level = i64::from(level.clamp(1, 6));
                let font = (30 - ((level - 1) * 3)).max(18);
                Self {
                    font_size: LayoutUnit::from_px(font),
                    line_height: LayoutUnit::from_px(font + 8),
                    margin_before: LayoutUnit::from_px(14),
                    margin_after: LayoutUnit::from_px(8),
                    padding_start: LayoutUnit::ZERO,
                    padding_end: LayoutUnit::ZERO,
                    text_indent: LayoutUnit::ZERO,
                    block_indent: depth_indent,
                    alignment: TextAlignment::Start,
                    keep_with_next: true,
                    break_before: false,
                    break_after: false,
                    break_inside: BreakInside::Auto,
                }
            }
            BlockKind::ListItem => Self {
                block_indent: depth_indent + LayoutUnit::from_px(18),
                margin_before: LayoutUnit::from_px(2),
                margin_after: LayoutUnit::from_px(4),
                ..Self::default()
            },
            BlockKind::BlockQuote => Self {
                block_indent: depth_indent + LayoutUnit::from_px(18),
                padding_start: LayoutUnit::from_px(10),
                margin_before: LayoutUnit::from_px(8),
                margin_after: LayoutUnit::from_px(8),
                ..Self::default()
            },
            BlockKind::Image | BlockKind::Unsupported | BlockKind::Divider => Self {
                margin_before: LayoutUnit::from_px(8),
                margin_after: LayoutUnit::from_px(8),
                block_indent: depth_indent,
                ..Self::default()
            },
            BlockKind::Paragraph | BlockKind::Container => Self {
                block_indent: depth_indent,
                ..Self::default()
            },
            BlockKind::ForcedBreak => Self::default(),
        }
    }

    fn apply_document_style(&mut self, document_style: &DocumentComputedStyle) {
        // PageScene owns fragment geometry. Author CSS may affect typography and
        // pagination policy, but not margins, padding, dimensions, or text indent.
        for (name, value) in &document_style.properties {
            match &**name {
                "font-size" => set_unit(&mut self.font_size, value),
                "line-height" => set_unit(&mut self.line_height, value),
                "text-align" => {
                    self.alignment = match &**value {
                        "center" => TextAlignment::Center,
                        "right" | "end" => TextAlignment::End,
                        "justify" => TextAlignment::Justify,
                        _ => TextAlignment::Start,
                    };
                }
                "break-before" | "page-break-before" => {
                    self.break_before = matches!(&**value, "page" | "always");
                }
                "break-after" | "page-break-after" => {
                    self.break_after = matches!(&**value, "page" | "always");
                }
                "break-inside" | "page-break-inside" => {
                    self.break_inside = if &**value == "avoid" {
                        BreakInside::Avoid
                    } else {
                        BreakInside::Auto
                    };
                }
                "keep-with-next" => {
                    self.keep_with_next = matches!(&**value, "true" | "always" | "avoid");
                }
                _ => {}
            }
        }
    }
}

impl Default for ComputedLayoutStyle {
    fn default() -> Self {
        Self {
            font_size: LayoutUnit::from_px(16),
            line_height: LayoutUnit::from_px(22),
            margin_before: LayoutUnit::from_px(4),
            margin_after: LayoutUnit::from_px(8),
            padding_start: LayoutUnit::ZERO,
            padding_end: LayoutUnit::ZERO,
            text_indent: LayoutUnit::ZERO,
            block_indent: LayoutUnit::ZERO,
            alignment: TextAlignment::Start,
            keep_with_next: false,
            break_before: false,
            break_after: false,
            break_inside: BreakInside::Auto,
        }
    }
}

/// Text alignment.
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq, Hash)]
pub enum TextAlignment {
    /// Align to the inline start edge.
    #[default]
    Start,
    /// Center text.
    Center,
    /// Align to the inline end edge.
    End,
    /// Justified text. MVP treats this as start-aligned for geometry.
    Justify,
}

/// Break-inside policy.
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq, Hash)]
pub enum BreakInside {
    /// Normal fragmentation.
    #[default]
    Auto,
    /// Avoid splitting when a page can hold the block.
    Avoid,
}

/// Intrinsic block size estimate.
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq, Hash)]
pub struct IntrinsicLayout {
    /// Inline extent.
    pub inline_size: LayoutUnit,
    /// Block extent.
    pub block_size: LayoutUnit,
    /// Number of legal break opportunities.
    pub break_count: u32,
}

/// A block that can be fragmented across pages.
pub trait Fragmentable {
    /// Return an intrinsic size estimate.
    fn layout_intrinsic(
        &self,
        context: &LayoutContext<'_>,
    ) -> Result<IntrinsicLayout, PageletError>;

    /// Fragment into the available block size.
    fn fragment(
        &self,
        context: &LayoutContext<'_>,
        token: Option<&BreakToken>,
        available_block: LayoutUnit,
    ) -> Result<FragmentResult, PageletError>;
}

/// Stable pagination resume point.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct BreakToken {
    /// Node that will be laid out next.
    pub node_id: NodeId,
    /// Linear layout block index.
    pub child_index: u32,
    /// UTF-8 byte offset inside the current text node.
    pub text_offset: u32,
    /// True when resuming inside a fragmented node.
    pub continuation: bool,
    /// Page index expected for the resumed page.
    pub page_index: u32,
    /// Content hash this token belongs to.
    pub content_fingerprint: ContentHash,
    /// Layout config fingerprint.
    pub config_fingerprint: u64,
    /// Text backend identity.
    pub text_backend_id: crate::text::TextBackendId,
    /// Font set fingerprint.
    pub font_fingerprint: crate::text::FontSetFingerprint,
}

#[derive(Clone, Copy)]
struct TokenContext<'a> {
    chapter: &'a ChapterIr,
    options: LayoutOptions,
    text_backend: &'a dyn TextBackend,
}

#[derive(Clone, Copy)]
struct BreakTokenPosition {
    node_id: NodeId,
    child_index: usize,
    text_offset: u32,
    continuation: bool,
    page_index: u32,
}

impl BreakTokenPosition {
    const fn new(
        node_id: NodeId,
        child_index: usize,
        text_offset: u32,
        continuation: bool,
        page_index: u32,
    ) -> Self {
        Self {
            node_id,
            child_index,
            text_offset,
            continuation,
            page_index,
        }
    }
}

impl BreakToken {
    fn start(
        chapter: &ChapterIr,
        options: LayoutOptions,
        backend: &dyn TextBackend,
        first_node: NodeId,
    ) -> Self {
        Self {
            node_id: first_node,
            child_index: 0,
            text_offset: 0,
            continuation: false,
            page_index: 0,
            content_fingerprint: chapter.content_hash,
            config_fingerprint: config_fingerprint(options.constraints),
            text_backend_id: backend.backend_id(),
            font_fingerprint: backend.font_fingerprint(),
        }
    }

    fn for_position(context: TokenContext<'_>, position: BreakTokenPosition) -> Self {
        Self {
            node_id: position.node_id,
            child_index: u32::try_from(position.child_index).unwrap_or(u32::MAX),
            text_offset: position.text_offset,
            continuation: position.continuation,
            page_index: position.page_index,
            content_fingerprint: context.chapter.content_hash,
            config_fingerprint: config_fingerprint(context.options.constraints),
            text_backend_id: context.text_backend.backend_id(),
            font_fingerprint: context.text_backend.font_fingerprint(),
        }
    }

    /// Serialize to a compact stable string for tests and wire staging.
    #[must_use]
    pub fn to_wire_string(&self) -> String {
        format!(
            "{}:{}:{}:{}:{}:{}:{}",
            self.node_id.get(),
            self.child_index,
            self.text_offset,
            u8::from(self.continuation),
            self.page_index,
            self.config_fingerprint,
            self.text_backend_id.0
        )
    }

    /// Parse a string produced by [`BreakToken::to_wire_string`].
    #[must_use]
    pub fn from_wire_string(
        value: &str,
        content_fingerprint: ContentHash,
        font_fingerprint: crate::text::FontSetFingerprint,
    ) -> Option<Self> {
        let mut parts = value.split(':');
        let node_id = NodeId::new(parts.next()?.parse().ok()?);
        let child_index = parts.next()?.parse().ok()?;
        let text_offset = parts.next()?.parse().ok()?;
        let continuation = parts.next()? == "1";
        let page_index = parts.next()?.parse().ok()?;
        let config_fingerprint = parts.next()?.parse().ok()?;
        let text_backend_id = crate::text::TextBackendId(parts.next()?.parse().ok()?);
        if parts.next().is_some() {
            return None;
        }
        Some(Self {
            node_id,
            child_index,
            text_offset,
            continuation,
            page_index,
            content_fingerprint,
            config_fingerprint,
            text_backend_id,
            font_fingerprint,
        })
    }
}

/// Result of fragmenting one block.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum FragmentResult {
    /// Block is complete.
    Complete {
        /// Produced fragments.
        fragments: Vec<SceneFragment>,
        /// Consumed height.
        consumed: LayoutUnit,
    },
    /// Block split across pages.
    Split {
        /// Produced fragments for the current page.
        head: Vec<SceneFragment>,
        /// Resume token.
        next: BreakToken,
        /// Consumed height.
        consumed: LayoutUnit,
    },
    /// Block does not fit the available space.
    DoesNotFit,
    /// Block is larger than a page and was clipped or scaled.
    Oversized {
        /// Produced fragments.
        fragments: Vec<SceneFragment>,
        /// Consumed height.
        consumed: LayoutUnit,
        /// Diagnostic explaining the fallback.
        diagnostic: Diagnostic,
    },
}

/// Page size.
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq, Hash)]
pub struct PageSize {
    /// Width.
    pub width: LayoutUnit,
    /// Height.
    pub height: LayoutUnit,
}

/// Rectangle in page coordinates.
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq, Hash)]
pub struct Rect {
    /// Left x.
    pub x: LayoutUnit,
    /// Top y.
    pub y: LayoutUnit,
    /// Width.
    pub width: LayoutUnit,
    /// Height.
    pub height: LayoutUnit,
}

impl Rect {
    /// Return true when the point is inside this rectangle.
    #[must_use]
    pub fn contains(self, x: LayoutUnit, y: LayoutUnit) -> bool {
        x >= self.x && y >= self.y && x <= self.x + self.width && y <= self.y + self.height
    }
}

/// Anchor range covered by a scene fragment.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub struct TextAnchorRange {
    /// Inclusive start.
    pub start: TextAnchor,
    /// Exclusive end.
    pub end: TextAnchor,
}

/// Kind of scene fragment.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub enum SceneFragmentKind {
    /// Text line clipped from a paragraph.
    TextLine,
    /// List marker.
    Marker,
    /// Image box.
    Image,
    /// Divider rule.
    Divider,
    /// Background or border decoration.
    BackgroundBorder,
    /// Debug overlay geometry.
    DebugOverlay,
    /// Unsupported semantic placeholder.
    UnsupportedPlaceholder,
}

/// One drawable page fragment.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SceneFragment {
    /// Stable fragment id within page.
    pub id: u32,
    /// Fragment kind.
    pub kind: SceneFragmentKind,
    /// Source semantic node.
    pub node_id: NodeId,
    /// Fragment rectangle.
    pub rect: Rect,
    /// Optional text payload for deterministic renderers/debug output.
    pub text: Option<Arc<str>>,
    /// Optional source XHTML range.
    pub source_range: Option<SourceRange>,
    /// Optional semantic text anchor range.
    pub anchor_range: Option<TextAnchorRange>,
    /// Line index inside the text block, when applicable.
    pub line_index: Option<u32>,
    /// True if the fragment was clipped to avoid an empty page.
    pub overflow: bool,
}

/// Clickable link region.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct LinkRegion {
    /// Region bounds.
    pub rect: Rect,
    /// Source node.
    pub node_id: NodeId,
    /// Original href.
    pub href: Arc<str>,
    /// Resolved document target.
    pub resolved_document: Option<Arc<str>>,
    /// Fragment target.
    pub fragment: Option<Arc<str>>,
    /// Link kind.
    pub kind: LinkKind,
}

/// Named source anchor region.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct AnchorRegion {
    /// Region bounds.
    pub rect: Rect,
    /// Fully resolved anchor key.
    pub key: Arc<str>,
    /// Source node.
    pub node_id: NodeId,
}

/// Selection geometry for one text range.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SelectionMap {
    /// Node containing the range.
    pub node_id: NodeId,
    /// UTF-8 start.
    pub start: u32,
    /// UTF-8 end.
    pub end: u32,
    /// Rectangles covering the range.
    pub rects: Vec<Rect>,
}

/// Semantic tree node exposed to assistive renderers.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SemanticNode {
    /// Source node.
    pub node_id: NodeId,
    /// Bounds.
    pub rect: Rect,
    /// Role.
    pub role: Arc<str>,
    /// Label or visible text.
    pub label: Arc<str>,
}

/// Stable page fingerprint.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub struct PageFingerprint(pub ContentHash);

impl PageFingerprint {
    /// Hex encode the fingerprint.
    #[must_use]
    pub fn to_hex(self) -> String {
        let mut out = String::with_capacity(64);
        for byte in self.0.as_bytes() {
            push_hex_byte(&mut out, *byte);
        }
        out
    }
}

/// One laid-out page scene.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PageScene {
    /// Zero-based page index.
    pub page_index: u32,
    /// Page size.
    pub size: PageSize,
    /// First text anchor on this page.
    pub start_anchor: Option<TextAnchor>,
    /// Last text anchor on this page.
    pub end_anchor: Option<TextAnchor>,
    /// Drawable fragments.
    pub fragments: Vec<SceneFragment>,
    /// Clickable link regions.
    pub links: Vec<LinkRegion>,
    /// Named anchor regions.
    pub anchors: Vec<AnchorRegion>,
    /// Selection rectangles.
    pub selections: Vec<SelectionMap>,
    /// Semantic nodes.
    pub semantics: Vec<SemanticNode>,
    /// Page fingerprint.
    pub fingerprint: PageFingerprint,
    /// Token for the next page, absent when complete.
    pub next_break_token: Option<BreakToken>,
    /// Diagnostics emitted while building this page.
    pub diagnostics: Vec<Diagnostic>,
}

impl PageScene {
    /// Serialize this page to stable normalized JSON.
    #[must_use]
    pub fn to_normalized_json(&self) -> String {
        let mut out = String::new();
        push_page_json(&mut out, self, 0, false);
        out.push('\n');
        out
    }
}

/// Pagination output.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PaginatedDocument {
    /// Page scenes.
    pub pages: Vec<PageScene>,
    /// Cross-page diagnostics.
    pub diagnostics: Vec<Diagnostic>,
    /// True when pagination reached chapter end.
    pub complete: bool,
}

/// Prepared two-phase layout that requests all host text metrics in one batch.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct HostMeasuredLayout {
    chapter: ChapterIr,
    options: LayoutOptions,
    measure_batch: MeasureBatch,
}

impl HostMeasuredLayout {
    /// Prepare a chapter for one batched host measurement round trip.
    #[must_use]
    pub fn prepare(chapter: ChapterIr, options: LayoutOptions) -> Self {
        let measure_batch = prepare_measure_batch(&chapter, options);
        Self {
            chapter,
            options,
            measure_batch,
        }
    }

    /// Return the complete paragraph/run measurement request batch.
    #[must_use]
    pub const fn measure_batch(&self) -> &MeasureBatch {
        &self.measure_batch
    }

    /// Validate host metrics and resume layout to completion.
    pub fn resume(self, measured: MeasuredBatch) -> Result<PaginatedDocument, PageletError> {
        let backend = HostMeasuredTextBackend::new(&self.measure_batch, measured)?;
        paginate_chapter_with_options(&self.chapter, &backend, self.options)
    }
}

/// Build the single host measurement batch required to paginate a chapter.
#[must_use]
pub fn prepare_measure_batch(chapter: &ChapterIr, options: LayoutOptions) -> MeasureBatch {
    let requests = layout_blocks(chapter)
        .into_iter()
        .enumerate()
        .filter_map(|(index, block)| {
            let LayoutBlockContent::Text { text, .. } = block.content else {
                return None;
            };
            Some(measure_request(
                u32::try_from(index).unwrap_or(u32::MAX),
                block.node_id.get(),
                &text,
                block.style,
                block_available_width(options.constraints, block.style),
            ))
        })
        .collect();
    MeasureBatch::new(requests)
}

impl PaginatedDocument {
    /// Serialize all pages to stable normalized JSON.
    #[must_use]
    pub fn to_normalized_json(&self) -> String {
        let mut out = String::new();
        out.push_str("{\n");
        indent(&mut out, 1);
        out.push_str("\"complete\": ");
        out.push_str(if self.complete { "true" } else { "false" });
        out.push_str(",\n");
        indent(&mut out, 1);
        out.push_str("\"pages\": [\n");
        for (index, page) in self.pages.iter().enumerate() {
            push_page_json(&mut out, page, 2, index + 1 < self.pages.len());
        }
        indent(&mut out, 1);
        out.push_str("]\n");
        out.push_str("}\n");
        out
    }
}

/// Hit-test result.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub struct HitTestResult {
    /// Hit node.
    pub node_id: NodeId,
    /// UTF-8 byte offset.
    pub utf8_byte_offset: u32,
    /// Affinity at the returned boundary.
    pub affinity: TextAffinity,
    /// Fragment id used for the hit.
    pub fragment_id: u32,
}

/// Layout one full chapter.
pub fn paginate_chapter(
    chapter: &ChapterIr,
    text_backend: &dyn TextBackend,
    constraints: LayoutConstraints,
) -> Result<PaginatedDocument, PageletError> {
    paginate_chapter_with_options(chapter, text_backend, LayoutOptions::new(constraints))
}

/// Layout one full chapter with explicit options.
pub fn paginate_chapter_with_options(
    chapter: &ChapterIr,
    text_backend: &dyn TextBackend,
    options: LayoutOptions,
) -> Result<PaginatedDocument, PageletError> {
    let cancel = CancellationToken::new();
    let blocks = layout_blocks(chapter);
    if blocks.is_empty() {
        return Ok(PaginatedDocument {
            pages: Vec::new(),
            diagnostics: Vec::new(),
            complete: true,
        });
    }
    let mut pages = Vec::new();
    let mut diagnostics = Vec::new();
    let mut token = Some(BreakToken::start(
        chapter,
        options,
        text_backend,
        blocks[0].node_id,
    ));

    while let Some(start) = token {
        if cancel.is_cancelled() {
            return Err(PageletError::Cancelled);
        }
        if pages.len() >= usize::try_from(options.max_pages).unwrap_or(usize::MAX) {
            return Err(PageletError::ResourceLimitExceeded(
                ResourceLimitError::new(
                    ResourceLimitKind::LayoutFragments,
                    u64::from(options.max_pages),
                    u64::try_from(pages.len()).unwrap_or(u64::MAX),
                ),
            ));
        }
        let Some(page) =
            paginate_page_from_blocks(chapter, &blocks, text_backend, options, &cancel, start)?
        else {
            break;
        };
        token = page.next_break_token.clone();
        diagnostics.extend(page.diagnostics.iter().cloned());
        pages.push(page);
    }

    validate_layout_invariants(chapter, &pages)?;

    Ok(PaginatedDocument {
        complete: pages
            .last()
            .is_none_or(|page| page.next_break_token.is_none()),
        pages,
        diagnostics,
    })
}

/// Layout the page starting at `token`.
pub fn paginate_next_page(
    chapter: &ChapterIr,
    text_backend: &dyn TextBackend,
    options: LayoutOptions,
    token: Option<BreakToken>,
) -> Result<Option<PageScene>, PageletError> {
    let blocks = layout_blocks(chapter);
    if blocks.is_empty() {
        return Ok(None);
    }
    let start = token
        .unwrap_or_else(|| BreakToken::start(chapter, options, text_backend, blocks[0].node_id));
    paginate_page_from_blocks(
        chapter,
        &blocks,
        text_backend,
        options,
        &CancellationToken::new(),
        start,
    )
}

/// Return the page index containing an anchor or the nearest preceding page.
#[must_use]
pub fn anchor_to_page(pages: &[PageScene], anchor: TextAnchor) -> Option<u32> {
    let mut nearest = None;
    for page in pages {
        if let (Some(start), Some(end)) = (page.start_anchor, page.end_anchor) {
            if anchor.node_id == start.node_id
                && anchor.utf8_byte_offset >= start.utf8_byte_offset
                && anchor.utf8_byte_offset <= end.utf8_byte_offset
            {
                return Some(page.page_index);
            }
            if start.node_id <= anchor.node_id {
                nearest = Some(page.page_index);
            }
        }
    }
    nearest.or_else(|| pages.first().map(|page| page.page_index))
}

/// Hit-test a page coordinate.
#[must_use]
pub fn hit_test(page: &PageScene, x: LayoutUnit, y: LayoutUnit) -> Option<HitTestResult> {
    for fragment in &page.fragments {
        if !matches!(fragment.kind, SceneFragmentKind::TextLine) || !fragment.rect.contains(x, y) {
            continue;
        }
        let Some(range) = fragment.anchor_range else {
            continue;
        };
        let relative_x = (x - fragment.rect.x).raw().max(0);
        let width = fragment.rect.width.raw().max(1);
        let span = range
            .end
            .utf8_byte_offset
            .saturating_sub(range.start.utf8_byte_offset)
            .max(1);
        let offset = range.start.utf8_byte_offset.saturating_add(
            u32::try_from((i64::from(span) * relative_x) / width).unwrap_or(u32::MAX),
        );
        return Some(HitTestResult {
            node_id: fragment.node_id,
            utf8_byte_offset: offset.min(range.end.utf8_byte_offset),
            affinity: TextAffinity::Downstream,
            fragment_id: fragment.id,
        });
    }
    None
}

/// Validate core layout invariants.
pub fn validate_layout_invariants(
    _chapter: &ChapterIr,
    pages: &[PageScene],
) -> Result<(), PageletError> {
    let mut last_anchor: Option<TextAnchor> = None;
    for page in pages {
        if page.fragments.is_empty() && page.next_break_token.is_some() {
            return Err(layout_error(
                "pagination produced an empty non-terminal page",
            ));
        }
        if page.size.width.raw() < 0 || page.size.height.raw() < 0 {
            return Err(layout_error("page extent is negative"));
        }
        if let Some(start) = page.start_anchor {
            if let Some(previous) = last_anchor {
                if (start.node_id, start.utf8_byte_offset)
                    < (previous.node_id, previous.utf8_byte_offset)
                {
                    return Err(layout_error("page start anchors are not monotonic"));
                }
            }
            last_anchor = Some(start);
        }
        let mut last_range_by_node = BTreeMap::<NodeId, u32>::new();
        for fragment in &page.fragments {
            if fragment.rect.width.raw() < 0 || fragment.rect.height.raw() < 0 {
                return Err(layout_error("fragment extent is negative"));
            }
            if let Some(range) = fragment.anchor_range {
                if range.end.utf8_byte_offset < range.start.utf8_byte_offset {
                    return Err(layout_error("fragment text range is reversed"));
                }
                if let Some(previous) =
                    last_range_by_node.insert(range.start.node_id, range.end.utf8_byte_offset)
                {
                    if range.start.utf8_byte_offset < previous {
                        return Err(layout_error("fragment text ranges overlap"));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Paragraph fragment layout.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ParagraphLayout {
    /// Source node.
    pub node_id: NodeId,
    /// Text content.
    pub text: Arc<str>,
    /// Style.
    pub style: ComputedLayoutStyle,
}

impl Fragmentable for ParagraphLayout {
    fn layout_intrinsic(
        &self,
        context: &LayoutContext<'_>,
    ) -> Result<IntrinsicLayout, PageletError> {
        let measured = measure_text(
            context.text_backend,
            context.cancel,
            0,
            self.node_id.get(),
            &self.text,
            self.style,
            context.options.constraints.content_width(),
        )?;
        Ok(IntrinsicLayout {
            inline_size: measured.width,
            block_size: measured.height,
            break_count: measured.line_count.saturating_sub(1),
        })
    }

    fn fragment(
        &self,
        context: &LayoutContext<'_>,
        token: Option<&BreakToken>,
        available_block: LayoutUnit,
    ) -> Result<FragmentResult, PageletError> {
        let measured = measure_text(
            context.text_backend,
            context.cancel,
            0,
            self.node_id.get(),
            &self.text,
            self.style,
            context.options.constraints.content_width(),
        )?;
        let start_offset = token.map_or(0, |token| token.text_offset);
        let start_line = measured
            .lines
            .iter()
            .position(|line| line.text_end > start_offset)
            .unwrap_or(measured.lines.len());
        if start_line >= measured.lines.len() {
            return Ok(FragmentResult::Complete {
                fragments: Vec::new(),
                consumed: LayoutUnit::ZERO,
            });
        }
        let mut consumed = LayoutUnit::ZERO;
        let mut count = 0_usize;
        for line in &measured.lines[start_line..] {
            if count > 0 && consumed + line.line_height > available_block {
                break;
            }
            consumed += line.line_height;
            count += 1;
            if consumed >= available_block {
                break;
            }
        }
        if count == 0 {
            return Ok(FragmentResult::DoesNotFit);
        }
        let complete = start_line + count >= measured.lines.len();
        if complete {
            Ok(FragmentResult::Complete {
                fragments: Vec::new(),
                consumed,
            })
        } else {
            let next_offset = measured.lines[start_line + count - 1].text_end;
            Ok(FragmentResult::Split {
                head: Vec::new(),
                next: BreakToken::for_position(
                    TokenContext {
                        chapter: context.chapter,
                        options: context.options,
                        text_backend: context.text_backend,
                    },
                    BreakTokenPosition::new(self.node_id, 0, next_offset, true, 0),
                ),
                consumed,
            })
        }
    }
}

fn paginate_page_from_blocks(
    chapter: &ChapterIr,
    blocks: &[LayoutBlock],
    text_backend: &dyn TextBackend,
    options: LayoutOptions,
    cancel: &CancellationToken,
    start: BreakToken,
) -> Result<Option<PageScene>, PageletError> {
    if usize::try_from(start.child_index).unwrap_or(usize::MAX) >= blocks.len() {
        return Ok(None);
    }

    let mut page = PageBuilder::new(start.page_index, options.constraints);
    let mut diagnostics = Vec::new();
    let mut index = usize::try_from(start.child_index).unwrap_or(usize::MAX);
    let mut text_offset = start.text_offset;
    let content_bottom = options.constraints.viewport_height - options.constraints.margin_bottom;
    let token_context = TokenContext {
        chapter,
        options,
        text_backend,
    };
    let mut next_token = None;

    while index < blocks.len() {
        if cancel.is_cancelled() {
            return Err(PageletError::Cancelled);
        }
        if page.fragments.len()
            >= usize::try_from(options.limits.max_layout_fragments).unwrap_or(usize::MAX)
        {
            return Err(PageletError::ResourceLimitExceeded(
                ResourceLimitError::new(
                    ResourceLimitKind::LayoutFragments,
                    u64::from(options.limits.max_layout_fragments),
                    u64::try_from(page.fragments.len()).unwrap_or(u64::MAX),
                ),
            ));
        }
        let block = &blocks[index];
        if block.style.break_before && !page.fragments.is_empty() {
            next_token = Some(BreakToken::for_position(
                token_context,
                BreakTokenPosition::new(
                    block.node_id,
                    index,
                    text_offset,
                    text_offset > 0,
                    start.page_index.saturating_add(1),
                ),
            ));
            break;
        }

        match &block.content {
            LayoutBlockContent::ForcedBreak => {
                if page.fragments.is_empty() {
                    index += 1;
                    text_offset = 0;
                    continue;
                }
                next_token =
                    next_block_token(token_context, blocks, index + 1, start.page_index + 1);
                break;
            }
            LayoutBlockContent::Text { text, marker } => {
                let available_width = block_available_width(options.constraints, block.style);
                let measured = measure_text(
                    text_backend,
                    cancel,
                    u32::try_from(index).unwrap_or(u32::MAX),
                    block.node_id.get(),
                    text,
                    block.style,
                    available_width,
                )?;
                if block.style.keep_with_next
                    && !page.fragments.is_empty()
                    && should_keep_with_next(blocks, index, &measured, page.y, content_bottom)
                {
                    next_token = Some(BreakToken::for_position(
                        token_context,
                        BreakTokenPosition::new(
                            block.node_id,
                            index,
                            text_offset,
                            text_offset > 0,
                            start.page_index + 1,
                        ),
                    ));
                    break;
                }
                let outcome = page.push_text_block(TextBlockPush {
                    token_context,
                    block,
                    block_index: index,
                    text,
                    marker: marker.as_deref(),
                    measured: &measured,
                    text_offset,
                    content_bottom,
                })?;
                match outcome {
                    BlockPushOutcome::Complete => {
                        if block.style.break_after {
                            next_token = next_block_token(
                                token_context,
                                blocks,
                                index + 1,
                                start.page_index + 1,
                            );
                            break;
                        }
                        index += 1;
                        text_offset = 0;
                    }
                    BlockPushOutcome::Split { next } => {
                        next_token = Some(next);
                        break;
                    }
                    BlockPushOutcome::NoFit => {
                        next_token = Some(BreakToken::for_position(
                            token_context,
                            BreakTokenPosition::new(
                                block.node_id,
                                index,
                                text_offset,
                                text_offset > 0,
                                start.page_index + 1,
                            ),
                        ));
                        break;
                    }
                }
            }
            LayoutBlockContent::Divider => {
                if !page.push_simple_box(
                    block,
                    SceneFragmentKind::Divider,
                    None,
                    LayoutUnit::from_px(1),
                    content_bottom,
                ) {
                    next_token = Some(BreakToken::for_position(
                        token_context,
                        BreakTokenPosition::new(
                            block.node_id,
                            index,
                            0,
                            false,
                            start.page_index + 1,
                        ),
                    ));
                    break;
                }
                index += 1;
                text_offset = 0;
            }
            LayoutBlockContent::Image(image) => {
                let (fit, diagnostic) = page.push_image_box(block, image, content_bottom);
                if let Some(diagnostic) = diagnostic {
                    diagnostics.push(diagnostic);
                }
                if !fit {
                    next_token = Some(BreakToken::for_position(
                        token_context,
                        BreakTokenPosition::new(
                            block.node_id,
                            index,
                            0,
                            false,
                            start.page_index + 1,
                        ),
                    ));
                    break;
                }
                index += 1;
                text_offset = 0;
            }
            LayoutBlockContent::Unsupported(label) => {
                if !page.push_simple_box(
                    block,
                    SceneFragmentKind::UnsupportedPlaceholder,
                    Some(label.clone()),
                    block.style.line_height,
                    content_bottom,
                ) {
                    next_token = Some(BreakToken::for_position(
                        token_context,
                        BreakTokenPosition::new(
                            block.node_id,
                            index,
                            0,
                            false,
                            start.page_index + 1,
                        ),
                    ));
                    break;
                }
                index += 1;
                text_offset = 0;
            }
        }
    }

    if index >= blocks.len() {
        next_token = None;
    }

    page.links = link_regions(chapter, &page.fragments);
    page.anchors = anchor_regions(chapter, &page.fragments);
    page.selections = selection_maps(&page.fragments);
    page.semantics = semantic_nodes(&page.fragments);
    page.diagnostics = diagnostics;
    page.next_break_token = next_token;
    page.fingerprint = fingerprint_page(chapter, options, text_backend, &page);

    Ok(Some(page.finish()))
}

#[derive(Debug)]
struct PageBuilder {
    page_index: u32,
    size: PageSize,
    y: LayoutUnit,
    next_fragment_id: u32,
    fragments: Vec<SceneFragment>,
    links: Vec<LinkRegion>,
    anchors: Vec<AnchorRegion>,
    selections: Vec<SelectionMap>,
    semantics: Vec<SemanticNode>,
    diagnostics: Vec<Diagnostic>,
    next_break_token: Option<BreakToken>,
    fingerprint: PageFingerprint,
}

struct TextBlockPush<'a> {
    token_context: TokenContext<'a>,
    block: &'a LayoutBlock,
    block_index: usize,
    text: &'a str,
    marker: Option<&'a str>,
    measured: &'a MeasuredText,
    text_offset: u32,
    content_bottom: LayoutUnit,
}

impl PageBuilder {
    fn new(page_index: u32, constraints: LayoutConstraints) -> Self {
        Self {
            page_index,
            size: PageSize {
                width: constraints.viewport_width,
                height: constraints.viewport_height,
            },
            y: constraints.margin_top,
            next_fragment_id: 0,
            fragments: Vec::new(),
            links: Vec::new(),
            anchors: Vec::new(),
            selections: Vec::new(),
            semantics: Vec::new(),
            diagnostics: Vec::new(),
            next_break_token: None,
            fingerprint: PageFingerprint(ContentHash::from_bytes(&[])),
        }
    }

    fn push_text_block(
        &mut self,
        input: TextBlockPush<'_>,
    ) -> Result<BlockPushOutcome, PageletError> {
        let TextBlockPush {
            token_context,
            block,
            block_index,
            text,
            marker,
            measured,
            text_offset,
            content_bottom,
        } = input;
        let chapter = token_context.chapter;
        let options = token_context.options;
        let start_line = measured
            .lines
            .iter()
            .position(|line| line.text_end > text_offset)
            .unwrap_or(measured.lines.len());
        if start_line >= measured.lines.len() {
            return Ok(BlockPushOutcome::Complete);
        }

        let top_margin = if self.fragments.is_empty() {
            LayoutUnit::ZERO
        } else {
            block.style.margin_before
        };
        if self.y + top_margin >= content_bottom && !self.fragments.is_empty() {
            return Ok(BlockPushOutcome::NoFit);
        }
        let mut y = self.y + top_margin;
        let mut fit_count = 0_usize;
        let total_remaining = measured.lines.len() - start_line;

        for line in &measured.lines[start_line..] {
            if fit_count > 0 && y + line.line_height > content_bottom {
                break;
            }
            if fit_count == 0 && y + line.line_height > content_bottom && !self.fragments.is_empty()
            {
                return Ok(BlockPushOutcome::NoFit);
            }
            y += line.line_height;
            fit_count += 1;
            if y >= content_bottom {
                break;
            }
        }

        if fit_count == 0 {
            fit_count = 1;
        }
        if fit_count < total_remaining {
            if fit_count == 1 && !self.fragments.is_empty() {
                return Ok(BlockPushOutcome::NoFit);
            }
            if total_remaining - fit_count == 1 && fit_count > 1 {
                fit_count -= 1;
            }
        }

        self.y += top_margin;
        let x = block_x(options.constraints, block.style);
        let mut line_y = self.y;
        let first_line_indent = block.style.text_indent;
        for relative_index in 0..fit_count {
            let line_index = start_line + relative_index;
            let line = measured.lines[line_index];
            let is_first_line = line_index == 0;
            let line_x = x + if is_first_line {
                first_line_indent
            } else {
                LayoutUnit::ZERO
            };
            if is_first_line {
                if let Some(marker) = marker {
                    let fragment_id = self.alloc_fragment_id();
                    self.push_fragment(SceneFragment {
                        id: fragment_id,
                        kind: SceneFragmentKind::Marker,
                        node_id: block.node_id,
                        rect: Rect {
                            x: (line_x - LayoutUnit::from_px(16))
                                .max(options.constraints.margin_start),
                            y: line_y,
                            width: LayoutUnit::from_px(12),
                            height: line.line_height,
                        },
                        text: Some(Arc::from(marker)),
                        source_range: block.source_range,
                        anchor_range: None,
                        line_index: Some(u32::try_from(line_index).unwrap_or(u32::MAX)),
                        overflow: false,
                    });
                }
            }
            let text_start = line.text_start.max(text_offset);
            let text_end = line.text_end;
            let text_slice = slice_text(text, text_start, text_end);
            let rect = Rect {
                x: aligned_x(
                    line_x,
                    block_available_width(options.constraints, block.style),
                    line.width,
                    block.style.alignment,
                ),
                y: line_y,
                width: line.width,
                height: line.line_height,
            };
            let range = TextAnchorRange {
                start: TextAnchor::new(
                    chapter.document_id,
                    block.node_id,
                    text_start,
                    TextAffinity::Downstream,
                ),
                end: TextAnchor::new(
                    chapter.document_id,
                    block.node_id,
                    text_end,
                    TextAffinity::Upstream,
                ),
            };
            let fragment_id = self.alloc_fragment_id();
            self.push_fragment(SceneFragment {
                id: fragment_id,
                kind: SceneFragmentKind::TextLine,
                node_id: block.node_id,
                rect,
                text: Some(Arc::from(text_slice)),
                source_range: block.source_range,
                anchor_range: Some(range),
                line_index: Some(u32::try_from(line_index).unwrap_or(u32::MAX)),
                overflow: line_y + line.line_height > content_bottom,
            });
            line_y += line.line_height;
        }

        self.y = line_y + block.style.margin_after;

        if start_line + fit_count >= measured.lines.len() {
            Ok(BlockPushOutcome::Complete)
        } else {
            let next_offset = measured.lines[start_line + fit_count - 1].text_end;
            Ok(BlockPushOutcome::Split {
                next: BreakToken::for_position(
                    token_context,
                    BreakTokenPosition::new(
                        block.node_id,
                        block_index,
                        next_offset,
                        true,
                        self.page_index + 1,
                    ),
                ),
            })
        }
    }

    fn push_simple_box(
        &mut self,
        block: &LayoutBlock,
        kind: SceneFragmentKind,
        text: Option<Arc<str>>,
        height: LayoutUnit,
        content_bottom: LayoutUnit,
    ) -> bool {
        let top_margin = if self.fragments.is_empty() {
            LayoutUnit::ZERO
        } else {
            block.style.margin_before
        };
        if self.y + top_margin + height > content_bottom && !self.fragments.is_empty() {
            return false;
        }
        self.y += top_margin;
        let fragment_id = self.alloc_fragment_id();
        self.push_fragment(SceneFragment {
            id: fragment_id,
            kind,
            node_id: block.node_id,
            rect: Rect {
                x: block.style.block_indent,
                y: self.y,
                width: LayoutUnit::from_px(160),
                height,
            },
            text,
            source_range: block.source_range,
            anchor_range: None,
            line_index: None,
            overflow: self.y + height > content_bottom,
        });
        self.y += height + block.style.margin_after;
        true
    }

    fn push_image_box(
        &mut self,
        block: &LayoutBlock,
        image: &LayoutImage,
        content_bottom: LayoutUnit,
    ) -> (bool, Option<Diagnostic>) {
        let top_margin = if self.fragments.is_empty() {
            LayoutUnit::ZERO
        } else {
            block.style.margin_before
        };
        let mut width = image.intrinsic_width.unwrap_or(LayoutUnit::from_px(180));
        let mut height = image.intrinsic_height.unwrap_or(LayoutUnit::from_px(140));
        if width > LayoutUnit::from_px(280) {
            let ratio_raw =
                (LayoutUnit::from_px(280).raw() * LayoutUnit::SCALE) / width.raw().max(1);
            width = LayoutUnit::from_px(280);
            height = LayoutUnit::from_raw((height.raw() * ratio_raw) / LayoutUnit::SCALE);
        }
        if self.y + top_margin + height > content_bottom && !self.fragments.is_empty() {
            return (false, None);
        }

        self.y += top_margin;
        let mut diagnostic = None;
        if self.y + height > content_bottom {
            height = (content_bottom - self.y).max(LayoutUnit::from_px(1));
            diagnostic = Some(Diagnostic::new(
                DiagnosticCode::Layout,
                Severity::Warning,
                "image height exceeded page content and was clipped",
            ));
        }
        let fragment_id = self.alloc_fragment_id();
        self.push_fragment(SceneFragment {
            id: fragment_id,
            kind: SceneFragmentKind::Image,
            node_id: block.node_id,
            rect: Rect {
                x: block.style.block_indent,
                y: self.y,
                width,
                height,
            },
            text: Some(image.alt.clone()),
            source_range: block.source_range,
            anchor_range: None,
            line_index: None,
            overflow: diagnostic.is_some(),
        });
        self.y += height + block.style.margin_after;
        (true, diagnostic)
    }

    fn alloc_fragment_id(&mut self) -> u32 {
        let id = self.next_fragment_id;
        self.next_fragment_id = self.next_fragment_id.saturating_add(1);
        id
    }

    fn push_fragment(&mut self, fragment: SceneFragment) {
        self.fragments.push(fragment);
    }

    fn finish(self) -> PageScene {
        let start_anchor = self
            .fragments
            .iter()
            .filter_map(|fragment| fragment.anchor_range.map(|range| range.start))
            .min_by_key(|anchor| (anchor.node_id, anchor.utf8_byte_offset));
        let end_anchor = self
            .fragments
            .iter()
            .filter_map(|fragment| fragment.anchor_range.map(|range| range.end))
            .max_by_key(|anchor| (anchor.node_id, anchor.utf8_byte_offset));
        PageScene {
            page_index: self.page_index,
            size: self.size,
            start_anchor,
            end_anchor,
            fragments: self.fragments,
            links: self.links,
            anchors: self.anchors,
            selections: self.selections,
            semantics: self.semantics,
            fingerprint: self.fingerprint,
            next_break_token: self.next_break_token,
            diagnostics: self.diagnostics,
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
enum BlockPushOutcome {
    Complete,
    Split { next: BreakToken },
    NoFit,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
enum BlockKind {
    Paragraph,
    Heading(u8),
    ListItem,
    BlockQuote,
    Image,
    Divider,
    ForcedBreak,
    Unsupported,
    Container,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct LayoutBlock {
    node_id: NodeId,
    kind: BlockKind,
    content: LayoutBlockContent,
    style: ComputedLayoutStyle,
    source_range: Option<SourceRange>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
enum LayoutBlockContent {
    Text {
        text: Arc<str>,
        marker: Option<Arc<str>>,
    },
    Image(LayoutImage),
    Divider,
    ForcedBreak,
    Unsupported(Arc<str>),
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct LayoutImage {
    alt: Arc<str>,
    intrinsic_width: Option<LayoutUnit>,
    intrinsic_height: Option<LayoutUnit>,
}

fn layout_blocks(chapter: &ChapterIr) -> Vec<LayoutBlock> {
    let mut blocks = Vec::new();
    collect_blocks(chapter, chapter.root, 0, None, &mut blocks);
    blocks
}

fn collect_blocks(
    chapter: &ChapterIr,
    node_id: NodeId,
    depth: u32,
    marker: Option<Arc<str>>,
    blocks: &mut Vec<LayoutBlock>,
) {
    let Some(node) = chapter.nodes.get(node_id) else {
        return;
    };
    match node {
        DocumentNode::Paragraph(text) => push_text_block(
            chapter,
            node_id,
            BlockKind::Paragraph,
            *text,
            depth,
            marker,
            blocks,
        ),
        DocumentNode::Heading(heading) => push_text_block(
            chapter,
            node_id,
            BlockKind::Heading(heading.level),
            heading.content,
            depth,
            marker,
            blocks,
        ),
        DocumentNode::List(list) => {
            for (index, child) in list.children.iter().enumerate() {
                let marker = if list.ordered {
                    Arc::from(format!("{}.", index + 1))
                } else {
                    Arc::from("•")
                };
                collect_blocks(chapter, *child, depth + 1, Some(marker), blocks);
            }
        }
        DocumentNode::ListItem(item) => {
            if item.children.is_empty() {
                blocks.push(layout_placeholder(
                    chapter,
                    node_id,
                    BlockKind::ListItem,
                    item.style,
                    "empty-list-item",
                    depth,
                ));
            }
            let mut pending_marker = marker;
            for child in &item.children {
                collect_blocks(chapter, *child, depth, pending_marker.take(), blocks);
            }
        }
        DocumentNode::BlockQuote(container) => {
            if container.children.is_empty() {
                blocks.push(layout_placeholder(
                    chapter,
                    node_id,
                    BlockKind::BlockQuote,
                    container.style,
                    "blockquote",
                    depth + 1,
                ));
            }
            for child in &container.children {
                collect_blocks(chapter, *child, depth + 1, None, blocks);
            }
        }
        DocumentNode::Figure(container) => {
            if container.children.is_empty() {
                blocks.push(layout_placeholder(
                    chapter,
                    node_id,
                    BlockKind::Container,
                    container.style,
                    "container",
                    depth,
                ));
            }
            for child in &container.children {
                collect_blocks(chapter, *child, depth, None, blocks);
            }
        }
        DocumentNode::Container(container) => {
            for child in &container.children {
                collect_blocks(chapter, *child, depth, None, blocks);
            }
        }
        DocumentNode::Table(container) => {
            blocks.push(layout_placeholder(
                chapter,
                node_id,
                BlockKind::Unsupported,
                container.style,
                "unsupported:table",
                depth,
            ));
            for child in &container.children {
                collect_blocks(chapter, *child, depth + 1, None, blocks);
            }
        }
        DocumentNode::Image(image) => blocks.push(layout_image(chapter, node_id, image, depth)),
        DocumentNode::Divider => blocks.push(LayoutBlock {
            node_id,
            kind: BlockKind::Divider,
            content: LayoutBlockContent::Divider,
            style: ComputedLayoutStyle::for_kind(BlockKind::Divider, depth),
            source_range: chapter.source_map.get(node_id),
        }),
        DocumentNode::ForcedBreak => blocks.push(LayoutBlock {
            node_id,
            kind: BlockKind::ForcedBreak,
            content: LayoutBlockContent::ForcedBreak,
            style: ComputedLayoutStyle::for_kind(BlockKind::ForcedBreak, depth),
            source_range: chapter.source_map.get(node_id),
        }),
        DocumentNode::Footnote(note) => {
            for child in &note.children {
                collect_blocks(chapter, *child, depth + 1, None, blocks);
            }
        }
        DocumentNode::Unsupported(unsupported) => {
            blocks.push(layout_placeholder(
                chapter,
                node_id,
                BlockKind::Unsupported,
                unsupported.style,
                &format!("unsupported:{}", unsupported.element),
                depth,
            ));
            for child in &unsupported.children {
                collect_blocks(chapter, *child, depth + 1, None, blocks);
            }
        }
    }
}

fn push_text_block(
    chapter: &ChapterIr,
    node_id: NodeId,
    kind: BlockKind,
    text: BlockText,
    depth: u32,
    marker: Option<Arc<str>>,
    blocks: &mut Vec<LayoutBlock>,
) {
    let Some(value) = chapter.text_pool.get(text.text) else {
        return;
    };
    blocks.push(LayoutBlock {
        node_id,
        kind,
        content: LayoutBlockContent::Text {
            text: Arc::from(value),
            marker,
        },
        style: ComputedLayoutStyle::from_document(&chapter.styles, text.style, kind, depth),
        source_range: chapter.source_map.get(node_id),
    });
}

fn layout_placeholder(
    chapter: &ChapterIr,
    node_id: NodeId,
    kind: BlockKind,
    style_id: crate::core::StyleId,
    label: &str,
    depth: u32,
) -> LayoutBlock {
    LayoutBlock {
        node_id,
        kind,
        content: LayoutBlockContent::Unsupported(Arc::from(label)),
        style: ComputedLayoutStyle::from_document(&chapter.styles, style_id, kind, depth),
        source_range: chapter.source_map.get(node_id),
    }
}

fn layout_image(
    chapter: &ChapterIr,
    node_id: NodeId,
    image: &ImageNode,
    depth: u32,
) -> LayoutBlock {
    LayoutBlock {
        node_id,
        kind: BlockKind::Image,
        content: LayoutBlockContent::Image(LayoutImage {
            alt: image.alt.clone(),
            intrinsic_width: None,
            intrinsic_height: None,
        }),
        style: ComputedLayoutStyle::from_document(
            &chapter.styles,
            image.style,
            BlockKind::Image,
            depth,
        ),
        source_range: chapter.source_map.get(node_id),
    }
}

fn measure_text(
    text_backend: &dyn TextBackend,
    cancel: &CancellationToken,
    request_id: u32,
    paragraph_id: u32,
    text: &str,
    style: ComputedLayoutStyle,
    width: LayoutUnit,
) -> Result<MeasuredText, PageletError> {
    let request = measure_request(request_id, paragraph_id, text, style, width);
    let batch = text_backend.measure_batch(&MeasureBatch::new(vec![request]), cancel)?;
    let mut measured = batch
        .get(request_id)
        .cloned()
        .ok_or_else(|| layout_error("text backend did not return requested measurement"))?;
    if measured.lines.is_empty() {
        measured = synthesize_lines(text, request_id, style, width);
    } else {
        apply_resolved_line_height(&mut measured, style.line_height);
    }
    Ok(measured)
}

fn measure_request(
    request_id: u32,
    paragraph_id: u32,
    text: &str,
    style: ComputedLayoutStyle,
    width: LayoutUnit,
) -> MeasureRequest {
    let mut request = MeasureRequest::new(request_id, text, style.font_size, width);
    request.paragraph_id = paragraph_id;
    request.direction = TextDirection::Auto;
    request.height_behavior = HeightBehavior::IncludeStrut;
    request.strut = line_height_strut(style);
    request.request_fingerprint = request_fingerprint(text, style, width);
    request
}

fn line_height_strut(style: ComputedLayoutStyle) -> StrutStyle {
    let font_size = style.font_size.max(LayoutUnit::from_px(1));
    let ascent = LayoutUnit::from_raw((font_size.raw() * 4) / 5);
    let descent = font_size - ascent;
    let leading = (style.line_height - font_size).max(LayoutUnit::ZERO);
    StrutStyle {
        ascent,
        descent,
        leading,
    }
}

fn apply_resolved_line_height(measured: &mut MeasuredText, line_height: LayoutUnit) {
    let line_height = line_height.max(LayoutUnit::from_px(1));
    for line in &mut measured.lines {
        let glyph_height = line.ascent + line.descent;
        let half_leading = LayoutUnit::from_raw((line_height - glyph_height).raw() / 2);
        line.baseline = line.ascent + half_leading;
        line.line_height = line_height;
    }
    measured.height = LayoutUnit::from_raw(
        line_height
            .raw()
            .saturating_mul(i64::try_from(measured.lines.len()).unwrap_or(i64::MAX)),
    );
}

fn synthesize_lines(
    text: &str,
    request_id: u32,
    style: ComputedLayoutStyle,
    width: LayoutUnit,
) -> MeasuredText {
    let line_height = style.line_height.max(LayoutUnit::from_px(1));
    let advance = LayoutUnit::from_raw((style.font_size.raw() / 2).max(1));
    let max_clusters = (width.raw().max(advance.raw()) / advance.raw()).max(1);
    let mut lines = Vec::new();
    let mut clusters = Vec::new();
    let mut line_start = 0_usize;
    let mut line_index = 0_u32;
    let mut line_width = LayoutUnit::ZERO;
    let mut x = LayoutUnit::ZERO;
    let mut clusters_on_line = 0_i64;
    for (offset, ch) in text.char_indices() {
        if clusters_on_line >= max_clusters && offset > line_start {
            lines.push(line_metrics(
                line_start,
                offset,
                line_width,
                line_height,
                false,
            ));
            line_start = offset;
            line_index = line_index.saturating_add(1);
            line_width = LayoutUnit::ZERO;
            x = LayoutUnit::ZERO;
            clusters_on_line = 0;
        }
        let end = offset + ch.len_utf8();
        clusters.push(TextCluster {
            text_start: u32::try_from(offset).unwrap_or(u32::MAX),
            text_end: u32::try_from(end).unwrap_or(u32::MAX),
            line_index,
            x_start: x,
            x_end: x + advance,
        });
        line_width += advance;
        x += advance;
        clusters_on_line += 1;
    }
    if line_start < text.len() || lines.is_empty() {
        lines.push(line_metrics(
            line_start,
            text.len(),
            line_width,
            line_height,
            false,
        ));
    }
    let height = LayoutUnit::from_raw(
        line_height
            .raw()
            .saturating_mul(i64::try_from(lines.len()).unwrap_or(i64::MAX)),
    );
    MeasuredText::new(
        request_id,
        request_fingerprint(text, style, width),
        width,
        height,
        u32::try_from(text.len()).unwrap_or(u32::MAX),
        lines,
        clusters,
        request_id as u64,
    )
}

fn line_metrics(
    start: usize,
    end: usize,
    width: LayoutUnit,
    line_height: LayoutUnit,
    hard_break: bool,
) -> LineMetrics {
    LineMetrics {
        text_start: u32::try_from(start).unwrap_or(u32::MAX),
        text_end: u32::try_from(end).unwrap_or(u32::MAX),
        baseline: LayoutUnit::from_raw((line_height.raw() * 4) / 5),
        ascent: LayoutUnit::from_raw((line_height.raw() * 4) / 5),
        descent: LayoutUnit::from_raw(line_height.raw() / 5),
        line_height,
        width,
        hard_break,
    }
}

fn next_block_token(
    token_context: TokenContext<'_>,
    blocks: &[LayoutBlock],
    next_index: usize,
    page_index: u32,
) -> Option<BreakToken> {
    blocks.get(next_index).map(|block| {
        BreakToken::for_position(
            token_context,
            BreakTokenPosition::new(block.node_id, next_index, 0, false, page_index),
        )
    })
}

fn should_keep_with_next(
    blocks: &[LayoutBlock],
    index: usize,
    measured: &MeasuredText,
    y: LayoutUnit,
    content_bottom: LayoutUnit,
) -> bool {
    let heading_height = measured
        .lines
        .first()
        .map_or(LayoutUnit::from_px(24), |line| line.line_height);
    let next_height = blocks
        .get(index + 1)
        .map_or(LayoutUnit::from_px(22), |block| block.style.line_height);
    y + heading_height + next_height > content_bottom
}

fn block_available_width(constraints: LayoutConstraints, style: ComputedLayoutStyle) -> LayoutUnit {
    let width =
        constraints.content_width() - style.block_indent - style.padding_start - style.padding_end;
    if width.raw() <= 0 {
        LayoutUnit::from_px(1)
    } else {
        width
    }
}

fn block_x(constraints: LayoutConstraints, style: ComputedLayoutStyle) -> LayoutUnit {
    constraints.margin_start + style.block_indent + style.padding_start
}

fn aligned_x(
    x: LayoutUnit,
    available_width: LayoutUnit,
    line_width: LayoutUnit,
    alignment: TextAlignment,
) -> LayoutUnit {
    let extra = available_width - line_width;
    if extra.raw() <= 0 {
        return x;
    }
    match alignment {
        TextAlignment::Center => x + LayoutUnit::from_raw(extra.raw() / 2),
        TextAlignment::End => x + extra,
        TextAlignment::Start | TextAlignment::Justify => x,
    }
}

fn link_regions(chapter: &ChapterIr, fragments: &[SceneFragment]) -> Vec<LinkRegion> {
    let mut regions = Vec::new();
    for link in &chapter.links {
        for fragment in fragments
            .iter()
            .filter(|fragment| fragment.node_id == link.source_node)
        {
            regions.push(link_region(link, fragment.rect));
        }
    }
    regions
}

fn link_region(link: &LinkTarget, rect: Rect) -> LinkRegion {
    LinkRegion {
        rect,
        node_id: link.source_node,
        href: link.href.clone(),
        resolved_document: link.resolved_document.clone(),
        fragment: link.fragment.clone(),
        kind: link.kind,
    }
}

fn anchor_regions(chapter: &ChapterIr, fragments: &[SceneFragment]) -> Vec<AnchorRegion> {
    let mut regions = Vec::new();
    for anchor in chapter.anchors.anchors.values() {
        if let Some(fragment) = fragments
            .iter()
            .find(|fragment| fragment.node_id == anchor.node_id)
        {
            regions.push(AnchorRegion {
                rect: fragment.rect,
                key: anchor.key.clone(),
                node_id: anchor.node_id,
            });
        }
    }
    regions
}

fn selection_maps(fragments: &[SceneFragment]) -> Vec<SelectionMap> {
    fragments
        .iter()
        .filter_map(|fragment| {
            let range = fragment.anchor_range?;
            Some(SelectionMap {
                node_id: fragment.node_id,
                start: range.start.utf8_byte_offset,
                end: range.end.utf8_byte_offset,
                rects: vec![fragment.rect],
            })
        })
        .collect()
}

fn semantic_nodes(fragments: &[SceneFragment]) -> Vec<SemanticNode> {
    fragments
        .iter()
        .map(|fragment| SemanticNode {
            node_id: fragment.node_id,
            rect: fragment.rect,
            role: Arc::from(match fragment.kind {
                SceneFragmentKind::TextLine => "text",
                SceneFragmentKind::Marker => "marker",
                SceneFragmentKind::Image => "image",
                SceneFragmentKind::Divider => "separator",
                SceneFragmentKind::UnsupportedPlaceholder => "note",
                SceneFragmentKind::BackgroundBorder | SceneFragmentKind::DebugOverlay => {
                    "presentation"
                }
            }),
            label: fragment.text.clone().unwrap_or_else(|| Arc::from("")),
        })
        .collect()
}

fn fingerprint_page(
    chapter: &ChapterIr,
    options: LayoutOptions,
    text_backend: &dyn TextBackend,
    page: &PageBuilder,
) -> PageFingerprint {
    let mut input = String::new();
    input.push_str(&format!(
        "p:{}:c:{}:b:{}:f:{}:",
        page.page_index,
        config_fingerprint(options.constraints),
        text_backend.backend_id().0,
        text_backend.font_fingerprint().0
    ));
    input.push_str(&hex_hash(chapter.content_hash));
    for fragment in &page.fragments {
        input.push_str(&format!(
            "|{}:{:?}:{}:{}:{}:{}",
            fragment.node_id.get(),
            fragment.kind,
            fragment.rect.x.raw(),
            fragment.rect.y.raw(),
            fragment.rect.width.raw(),
            fragment.rect.height.raw()
        ));
        if let Some(range) = fragment.anchor_range {
            input.push_str(&format!(
                ":{}-{}",
                range.start.utf8_byte_offset, range.end.utf8_byte_offset
            ));
        }
    }
    PageFingerprint(ContentHash::from_bytes(input.as_bytes()))
}

fn config_fingerprint(constraints: LayoutConstraints) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for value in [
        constraints.viewport_width.raw(),
        constraints.viewport_height.raw(),
        constraints.margin_start.raw(),
        constraints.margin_end.raw(),
        constraints.margin_top.raw(),
        constraints.margin_bottom.raw(),
    ] {
        hash ^= value as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn request_fingerprint(text: &str, style: ComputedLayoutStyle, width: LayoutUnit) -> u64 {
    let mut hash = config_fingerprint(LayoutConstraints::new(width, style.line_height));
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn slice_text(text: &str, start: u32, end: u32) -> &str {
    let start = usize::try_from(start).unwrap_or(0).min(text.len());
    let end = usize::try_from(end).unwrap_or(text.len()).min(text.len());
    if start <= end {
        text.get(start..end).unwrap_or("")
    } else {
        ""
    }
}

fn set_unit(target: &mut LayoutUnit, value: &str) {
    if let Some(px) = parse_px(value) {
        *target = px;
    }
}

fn parse_px(value: &str) -> Option<LayoutUnit> {
    let value = value.trim();
    let number = value.strip_suffix("px").unwrap_or(value).trim();
    number
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .map(LayoutUnit::from_f64_px)
}

fn layout_error(message: impl Into<Arc<str>>) -> PageletError {
    PageletError::Layout(LayoutError::new(message))
}

fn push_page_json(out: &mut String, page: &PageScene, level: usize, trailing: bool) {
    indent(out, level);
    out.push_str("{\n");
    push_u32(out, level + 1, "page_index", page.page_index, true);
    push_string(
        out,
        level + 1,
        "fingerprint",
        &page.fingerprint.to_hex(),
        true,
    );
    indent(out, level + 1);
    out.push_str("\"size\": {");
    push_inline_i64(out, "width", page.size.width.raw(), true);
    push_inline_i64(out, "height", page.size.height.raw(), false);
    out.push_str("},\n");
    indent(out, level + 1);
    out.push_str("\"fragments\": [\n");
    for (index, fragment) in page.fragments.iter().enumerate() {
        push_fragment_json(out, fragment, level + 2, index + 1 < page.fragments.len());
    }
    indent(out, level + 1);
    out.push_str("],\n");
    indent(out, level + 1);
    out.push_str("\"links\": [\n");
    for (index, link) in page.links.iter().enumerate() {
        indent(out, level + 2);
        out.push('{');
        push_inline_string(out, "href", &link.href, true);
        push_inline_rect(out, "rect", link.rect);
        out.push('}');
        if index + 1 < page.links.len() {
            out.push(',');
        }
        out.push('\n');
    }
    indent(out, level + 1);
    out.push_str("],\n");
    indent(out, level + 1);
    out.push_str("\"anchors\": [\n");
    for (index, anchor) in page.anchors.iter().enumerate() {
        indent(out, level + 2);
        out.push('{');
        push_inline_string(out, "key", &anchor.key, true);
        push_inline_rect(out, "rect", anchor.rect);
        out.push('}');
        if index + 1 < page.anchors.len() {
            out.push(',');
        }
        out.push('\n');
    }
    indent(out, level + 1);
    out.push_str("]\n");
    indent(out, level);
    out.push('}');
    if trailing {
        out.push(',');
    }
    out.push('\n');
}

fn push_fragment_json(out: &mut String, fragment: &SceneFragment, level: usize, trailing: bool) {
    indent(out, level);
    out.push('{');
    push_inline_u32(out, "id", fragment.id, true);
    push_inline_string(out, "kind", fragment_kind_name(&fragment.kind), true);
    push_inline_u32(out, "node_id", fragment.node_id.get(), true);
    push_inline_rect(out, "rect", fragment.rect);
    if let Some(text) = &fragment.text {
        out.push_str(", ");
        push_inline_string(out, "text", text, false);
    }
    if let Some(range) = fragment.anchor_range {
        out.push_str(", \"range\": {");
        push_inline_u32(out, "start", range.start.utf8_byte_offset, true);
        push_inline_u32(out, "end", range.end.utf8_byte_offset, false);
        out.push('}');
    }
    out.push('}');
    if trailing {
        out.push(',');
    }
    out.push('\n');
}

fn fragment_kind_name(kind: &SceneFragmentKind) -> &'static str {
    match kind {
        SceneFragmentKind::TextLine => "text-line",
        SceneFragmentKind::Marker => "marker",
        SceneFragmentKind::Image => "image",
        SceneFragmentKind::Divider => "divider",
        SceneFragmentKind::BackgroundBorder => "background-border",
        SceneFragmentKind::DebugOverlay => "debug-overlay",
        SceneFragmentKind::UnsupportedPlaceholder => "unsupported-placeholder",
    }
}

fn push_string(out: &mut String, level: usize, name: &str, value: &str, trailing: bool) {
    indent(out, level);
    out.push('"');
    out.push_str(name);
    out.push_str("\": \"");
    out.push_str(&escape_json(value));
    out.push('"');
    if trailing {
        out.push(',');
    }
    out.push('\n');
}

fn push_u32(out: &mut String, level: usize, name: &str, value: u32, trailing: bool) {
    indent(out, level);
    out.push('"');
    out.push_str(name);
    out.push_str("\": ");
    out.push_str(&value.to_string());
    if trailing {
        out.push(',');
    }
    out.push('\n');
}

fn push_inline_string(out: &mut String, name: &str, value: &str, trailing: bool) {
    out.push('"');
    out.push_str(name);
    out.push_str("\": \"");
    out.push_str(&escape_json(value));
    out.push('"');
    if trailing {
        out.push_str(", ");
    }
}

fn push_inline_u32(out: &mut String, name: &str, value: u32, trailing: bool) {
    out.push('"');
    out.push_str(name);
    out.push_str("\": ");
    out.push_str(&value.to_string());
    if trailing {
        out.push_str(", ");
    }
}

fn push_inline_i64(out: &mut String, name: &str, value: i64, trailing: bool) {
    out.push('"');
    out.push_str(name);
    out.push_str("\": ");
    out.push_str(&value.to_string());
    if trailing {
        out.push_str(", ");
    }
}

fn push_inline_rect(out: &mut String, name: &str, rect: Rect) {
    out.push('"');
    out.push_str(name);
    out.push_str("\": {");
    push_inline_i64(out, "x", rect.x.raw(), true);
    push_inline_i64(out, "y", rect.y.raw(), true);
    push_inline_i64(out, "width", rect.width.raw(), true);
    push_inline_i64(out, "height", rect.height.raw(), false);
    out.push('}');
}

fn indent(out: &mut String, level: usize) {
    for _ in 0..level {
        out.push_str("  ");
    }
}

fn escape_json(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out
}

fn hex_hash(hash: ContentHash) -> String {
    let mut out = String::with_capacity(64);
    for byte in hash.as_bytes() {
        push_hex_byte(&mut out, *byte);
    }
    out
}

fn push_hex_byte(out: &mut String, byte: u8) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    out.push(HEX[(byte >> 4) as usize] as char);
    out.push(HEX[(byte & 0x0f) as usize] as char);
}

/// Create a debug SVG from page scene geometry.
#[must_use]
pub fn page_debug_svg(page: &PageScene) -> String {
    let width = page.size.width.to_f64_px();
    let height = page.size.height.to_f64_px();
    let mut out = String::new();
    out.push_str(&format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width:.2} {height:.2}" width="{width:.2}" height="{height:.2}">"#
    ));
    out.push_str(r#"<rect x="0" y="0" width="100%" height="100%" fill="white"/>"#);
    for fragment in &page.fragments {
        let color = match fragment.kind {
            SceneFragmentKind::TextLine => "#1d4ed8",
            SceneFragmentKind::Marker => "#7c3aed",
            SceneFragmentKind::Image => "#047857",
            SceneFragmentKind::Divider => "#334155",
            SceneFragmentKind::UnsupportedPlaceholder => "#b45309",
            SceneFragmentKind::BackgroundBorder | SceneFragmentKind::DebugOverlay => "#64748b",
        };
        out.push_str(&format!(
            r#"<rect x="{:.2}" y="{:.2}" width="{:.2}" height="{:.2}" fill="none" stroke="{}" stroke-width="1"/>"#,
            fragment.rect.x.to_f64_px(),
            fragment.rect.y.to_f64_px(),
            fragment.rect.width.to_f64_px(),
            fragment.rect.height.to_f64_px(),
            color,
        ));
        if let Some(text) = &fragment.text {
            out.push_str(&format!(
                r#"<text x="{:.2}" y="{:.2}" font-size="10" fill="{}">{}</text>"#,
                fragment.rect.x.to_f64_px(),
                (fragment.rect.y + LayoutUnit::from_px(10)).to_f64_px(),
                color,
                escape_xml(text),
            ));
        }
    }
    out.push_str("</svg>\n");
    out
}

fn escape_xml(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        core::{DocumentId, StyleId},
        document::{
            self, BlockText, ChapterIr, ContainerNode, ImageNode, ListItemNode, ListNode, TextRange,
        },
        text::{DefaultTextBackend, FontSetFingerprint, MeasuredBatch, TextBackend, TextBackendId},
    };

    #[test]
    fn host_measured_layout_prepares_one_batch_and_resumes_with_host_identity() {
        let chapter = chapter_with_paragraph("host metrics must drive the page scene");
        let options = LayoutOptions::new(
            LayoutConstraints::new(LayoutUnit::from_px(180), LayoutUnit::from_px(100))
                .with_margin(LayoutUnit::from_px(8)),
        );
        let prepared = HostMeasuredLayout::prepare(chapter.clone(), options);
        assert_eq!(prepared.measure_batch().requests.len(), 1);
        assert_eq!(
            prepared.measure_batch().requests[0].paragraph_id,
            NodeId::new(1).get()
        );

        let fallback = DefaultTextBackend::new();
        let fallback_measured = fallback
            .measure_batch(prepared.measure_batch(), &CancellationToken::new())
            .expect("measure batch");
        let fallback_pages =
            paginate_chapter_with_options(&chapter, &fallback, options).expect("fallback layout");
        let host_measured = MeasuredBatch::new(
            TextBackendId(0x686f_7374),
            FontSetFingerprint(0x666f_6e74),
            fallback_measured.results,
        );
        let pages = prepared.resume(host_measured).expect("resume layout");

        assert_ne!(
            pages.pages[0].fingerprint, fallback_pages.pages[0].fingerprint,
            "host backend identity must participate in the page fingerprint"
        );
        if let Some(token) = pages.pages[0].next_break_token.as_ref() {
            assert_eq!(token.text_backend_id, TextBackendId(0x686f_7374));
            assert_eq!(token.font_fingerprint, FontSetFingerprint(0x666f_6e74));
        }
    }

    #[test]
    fn paragraph_splits_across_pages_with_break_token() {
        let chapter = chapter_with_paragraph("one two three four five six seven eight nine ten");
        let backend = DefaultTextBackend::new();
        let options = LayoutOptions {
            constraints: LayoutConstraints::new(LayoutUnit::from_px(90), LayoutUnit::from_px(72))
                .with_margin(LayoutUnit::from_px(8)),
            max_pages: 16,
            ..LayoutOptions::default()
        };

        let pages = paginate_chapter_with_options(&chapter, &backend, options).expect("paginate");

        assert!(pages.pages.len() > 1);
        assert!(pages.pages[0].next_break_token.is_some());
        assert!(pages.complete);
    }

    #[test]
    fn forced_break_ends_current_page_without_empty_page() {
        let mut chapter = empty_chapter();
        let first = push_paragraph(&mut chapter, "before");
        let forced = chapter
            .nodes
            .push(DocumentNode::ForcedBreak)
            .expect("break");
        let second = push_paragraph(&mut chapter, "after");
        set_root_children(&mut chapter, vec![first, forced, second]);
        chapter.rebuild_utf16_index();

        let pages = paginate_chapter(
            &chapter,
            &DefaultTextBackend::new(),
            LayoutConstraints::default(),
        )
        .expect("paginate");

        assert_eq!(pages.pages.len(), 2);
        assert!(!pages.pages[0].fragments.is_empty());
        assert!(!pages.pages[1].fragments.is_empty());
    }

    #[test]
    fn list_marker_stays_before_text() {
        let mut chapter = empty_chapter();
        let paragraph = push_paragraph(&mut chapter, "item text");
        let item = chapter
            .nodes
            .push(DocumentNode::ListItem(ListItemNode {
                children: vec![paragraph],
                style: StyleId::new(0),
            }))
            .expect("item");
        let list = chapter
            .nodes
            .push(DocumentNode::List(ListNode {
                ordered: false,
                children: vec![item],
                style: StyleId::new(0),
            }))
            .expect("list");
        set_root_children(&mut chapter, vec![list]);

        let pages = paginate_chapter(
            &chapter,
            &DefaultTextBackend::new(),
            LayoutConstraints::default(),
        )
        .expect("paginate");
        let marker = pages.pages[0]
            .fragments
            .iter()
            .find(|fragment| fragment.kind == SceneFragmentKind::Marker)
            .expect("marker");
        let text = pages.pages[0]
            .fragments
            .iter()
            .find(|fragment| fragment.kind == SceneFragmentKind::TextLine)
            .expect("text");

        assert!(marker.rect.x + marker.rect.width <= text.rect.x);
    }

    #[test]
    fn unsupported_node_emits_placeholder() {
        let mut chapter = empty_chapter();
        let unsupported = chapter
            .nodes
            .push(DocumentNode::Unsupported(document::UnsupportedNode {
                element: Arc::from("math"),
                children: Vec::new(),
                style: StyleId::new(0),
            }))
            .expect("unsupported");
        set_root_children(&mut chapter, vec![unsupported]);

        let pages = paginate_chapter(
            &chapter,
            &DefaultTextBackend::new(),
            LayoutConstraints::default(),
        )
        .expect("paginate");

        assert_eq!(
            pages.pages[0].fragments[0].kind,
            SceneFragmentKind::UnsupportedPlaceholder
        );
    }

    #[test]
    fn empty_container_does_not_emit_placeholder() {
        let mut chapter = empty_chapter();
        let empty_container = chapter
            .nodes
            .push(DocumentNode::Container(ContainerNode::default()))
            .expect("container");
        let paragraph = push_paragraph(&mut chapter, "visible text");
        set_root_children(&mut chapter, vec![empty_container, paragraph]);

        let pages = paginate_chapter(
            &chapter,
            &DefaultTextBackend::new(),
            LayoutConstraints::default(),
        )
        .expect("paginate");

        assert!(pages.pages.iter().all(|page| {
            page.fragments
                .iter()
                .all(|fragment| fragment.kind != SceneFragmentKind::UnsupportedPlaceholder)
        }));
    }

    #[test]
    fn page_scene_ignores_document_geometry_css() {
        let plain = chapter_with_paragraph("geometry must stay stable");
        let mut styled = chapter_with_paragraph("geometry must stay stable");
        let style = document::ComputedStyle::new()
            .with_property("margin-top", "120px")
            .with_property("margin-bottom", "80px")
            .with_property("padding-left", "90px")
            .with_property("padding-right", "70px")
            .with_property("text-indent", "60px")
            .with_property("width", "1px")
            .with_property("height", "1px");
        let style_id = styled.styles.intern(style).expect("style");
        let paragraph = styled
            .nodes
            .iter_with_ids()
            .find_map(|(node_id, node)| {
                matches!(node, DocumentNode::Paragraph(_)).then_some(node_id)
            })
            .expect("paragraph");
        if let Some(DocumentNode::Paragraph(text)) = styled.nodes.get_mut(paragraph) {
            text.style = style_id;
        }

        let plain_pages = paginate_chapter(
            &plain,
            &DefaultTextBackend::new(),
            LayoutConstraints::default(),
        )
        .expect("plain pagination");
        let styled_pages = paginate_chapter(
            &styled,
            &DefaultTextBackend::new(),
            LayoutConstraints::default(),
        )
        .expect("styled pagination");
        let plain_rects = plain_pages
            .pages
            .iter()
            .flat_map(|page| page.fragments.iter().map(|fragment| fragment.rect))
            .collect::<Vec<_>>();
        let styled_rects = styled_pages
            .pages
            .iter()
            .flat_map(|page| page.fragments.iter().map(|fragment| fragment.rect))
            .collect::<Vec<_>>();

        assert_eq!(styled_rects, plain_rects);
    }

    #[test]
    fn resolved_font_metrics_drive_page_scene_line_boxes() {
        let mut chapter = chapter_with_paragraph("resolved metrics");
        let style = document::ComputedStyle::new()
            .with_property("font-size", "20px")
            .with_property("line-height", "30px");
        let style_id = chapter.styles.intern(style).expect("style");
        let paragraph = chapter
            .nodes
            .iter_with_ids()
            .find_map(|(node_id, node)| {
                matches!(node, DocumentNode::Paragraph(_)).then_some(node_id)
            })
            .expect("paragraph");
        if let Some(DocumentNode::Paragraph(text)) = chapter.nodes.get_mut(paragraph) {
            text.style = style_id;
        }

        let pages = paginate_chapter(
            &chapter,
            &StrutCheckingBackend,
            LayoutConstraints::default(),
        )
        .expect("pagination");
        let line = pages.pages[0]
            .fragments
            .iter()
            .find(|fragment| fragment.kind == SceneFragmentKind::TextLine)
            .expect("text line");

        assert_eq!(line.rect.height, LayoutUnit::from_px(30));
    }

    struct StrutCheckingBackend;

    impl TextBackend for StrutCheckingBackend {
        fn backend_id(&self) -> crate::text::TextBackendId {
            crate::text::TextBackendId(1)
        }

        fn font_fingerprint(&self) -> crate::text::FontSetFingerprint {
            crate::text::FontSetFingerprint(2)
        }

        fn measure_batch(
            &self,
            batch: &MeasureBatch,
            cancel: &CancellationToken,
        ) -> Result<crate::text::MeasuredBatch, PageletError> {
            let request = batch.requests.first().expect("measure request");
            assert_eq!(request.height_behavior, HeightBehavior::IncludeStrut);
            assert_eq!(request.strut.ascent, LayoutUnit::from_px(16));
            assert_eq!(request.strut.descent, LayoutUnit::from_px(4));
            assert_eq!(request.strut.leading, LayoutUnit::from_px(10));
            DefaultTextBackend::new().measure_batch(batch, cancel)
        }
    }

    #[test]
    fn oversized_image_is_clipped_with_diagnostic() {
        let mut chapter = empty_chapter();
        let image = chapter
            .nodes
            .push(DocumentNode::Image(ImageNode {
                alt: Arc::from("cover"),
                ..ImageNode::default()
            }))
            .expect("image");
        set_root_children(&mut chapter, vec![image]);

        let pages = paginate_chapter_with_options(
            &chapter,
            &DefaultTextBackend::new(),
            LayoutOptions::new(LayoutConstraints::new(
                LayoutUnit::from_px(160),
                LayoutUnit::from_px(80),
            )),
        )
        .expect("paginate");

        assert!(pages.pages[0].fragments[0].overflow);
        assert_eq!(pages.pages[0].diagnostics[0].code, DiagnosticCode::Layout);
    }

    #[test]
    fn hit_test_returns_text_offset_inside_fragment() {
        let chapter = chapter_with_paragraph("hello pagelet");
        let pages = paginate_chapter(
            &chapter,
            &DefaultTextBackend::new(),
            LayoutConstraints::default(),
        )
        .expect("paginate");
        let fragment = pages.pages[0]
            .fragments
            .iter()
            .find(|fragment| fragment.kind == SceneFragmentKind::TextLine)
            .expect("text");

        let hit = hit_test(
            &pages.pages[0],
            fragment.rect.x + LayoutUnit::from_px(4),
            fragment.rect.y + LayoutUnit::from_px(4),
        )
        .expect("hit");

        assert_eq!(hit.node_id, fragment.node_id);
    }

    #[test]
    fn page_scene_json_contains_fragments() {
        let chapter = chapter_with_paragraph("json");
        let pages = paginate_chapter(
            &chapter,
            &DefaultTextBackend::new(),
            LayoutConstraints::default(),
        )
        .expect("paginate");
        let json = pages.to_normalized_json();

        assert!(json.contains(r#""pages""#));
        assert!(json.contains(r#""fragments""#));
        assert!(json.contains(r#""fingerprint""#));
    }

    #[test]
    fn break_token_wire_round_trips() {
        let chapter = chapter_with_paragraph("token");
        let backend = DefaultTextBackend::new();
        let token = BreakToken::start(&chapter, LayoutOptions::default(), &backend, NodeId::new(1));
        let encoded = token.to_wire_string();
        let decoded = BreakToken::from_wire_string(
            &encoded,
            chapter.content_hash,
            backend.font_fingerprint(),
        )
        .expect("decode");

        assert_eq!(decoded, token);
    }

    #[test]
    fn paginate_all_matches_incremental_pages() {
        let chapter = chapter_with_paragraph("one two three four five six seven eight nine ten");
        let backend = DefaultTextBackend::new();
        let options = LayoutOptions {
            constraints: LayoutConstraints::new(LayoutUnit::from_px(90), LayoutUnit::from_px(72))
                .with_margin(LayoutUnit::from_px(8)),
            max_pages: 16,
            ..LayoutOptions::default()
        };
        let all = paginate_chapter_with_options(&chapter, &backend, options).expect("all");
        let mut token = None;
        let mut incremental = Vec::new();
        while let Some(page) = paginate_next_page(&chapter, &backend, options, token).expect("next")
        {
            token = page.next_break_token.clone();
            incremental.push(page);
            if token.is_none() {
                break;
            }
        }

        assert_eq!(all.pages.len(), incremental.len());
        assert_eq!(
            all.pages
                .iter()
                .map(|page| page.fingerprint)
                .collect::<Vec<_>>(),
            incremental
                .iter()
                .map(|page| page.fingerprint)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn stress_1000_page_synthetic_document() {
        let mut chapter = empty_chapter();
        let mut children = Vec::new();
        for index in 0..1_050 {
            children.push(push_paragraph(&mut chapter, &format!("paragraph {index}")));
        }
        set_root_children(&mut chapter, children);
        chapter.rebuild_utf16_index();

        let pages = paginate_chapter_with_options(
            &chapter,
            &DefaultTextBackend::new(),
            LayoutOptions {
                constraints: LayoutConstraints::new(
                    LayoutUnit::from_px(220),
                    LayoutUnit::from_px(42),
                ),
                max_pages: 2_000,
                ..LayoutOptions::default()
            },
        )
        .expect("paginate");

        assert!(pages.pages.len() >= 1_000);
        assert!(pages.complete);
    }

    fn chapter_with_paragraph(text: &str) -> ChapterIr {
        let mut chapter = empty_chapter();
        let paragraph = push_paragraph(&mut chapter, text);
        set_root_children(&mut chapter, vec![paragraph]);
        chapter.rebuild_utf16_index();
        chapter
    }

    fn empty_chapter() -> ChapterIr {
        let mut chapter = ChapterIr::empty(
            DocumentId::new(1),
            "chapter.xhtml",
            "Chapter",
            ContentHash::from_bytes(b"chapter"),
        );
        let root = chapter
            .nodes
            .push(DocumentNode::Container(document::ContainerNode {
                children: Vec::new(),
                style: StyleId::new(0),
            }))
            .expect("root");
        chapter.root = root;
        chapter
    }

    fn push_paragraph(chapter: &mut ChapterIr, text: &str) -> NodeId {
        let range = chapter.text_pool.push(text).expect("text");
        chapter
            .nodes
            .push(DocumentNode::Paragraph(BlockText {
                text: TextRange {
                    start: range.start,
                    end: range.end,
                },
                style: StyleId::new(0),
            }))
            .expect("paragraph")
    }

    fn set_root_children(chapter: &mut ChapterIr, children: Vec<NodeId>) {
        if let Some(DocumentNode::Container(ContainerNode {
            children: root_children,
            ..
        })) = chapter.nodes.get_mut(chapter.root)
        {
            *root_children = children;
        }
    }
}
