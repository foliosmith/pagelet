//! Deterministic layout, fragmentation, pagination, and page scenes.

use std::{collections::BTreeMap, sync::Arc};

use crate::{
    core::{
        CancellationToken, ContentHash, Diagnostic, DiagnosticCode, EngineVersions, LayoutError,
        LayoutUnit, NodeId, PageletError, ResourceLimitError, ResourceLimitKind, ResourceLimits,
        Severity, SourceRange, TextAffinity, TextAnchor,
    },
    document::{
        BlockText, ChapterIr, ComputedStyle as DocumentComputedStyle, DocumentNode,
        ImageLayoutRole, ImageNode, LinkKind, LinkTarget, StyleTable,
    },
    text::{
        FontDescriptor, FontFallbackChain, FontSetFingerprint, FontStyle, HeightBehavior,
        HostMeasuredTextBackend, LineMetrics, MeasureBatch, MeasureRequest, MeasuredBatch,
        MeasuredText, StrutStyle, TextBackend, TextBounds, TextCluster, TextDirection,
        TextStyleRun,
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
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct ComputedLayoutStyle {
    /// Font size.
    pub font_size: LayoutUnit,
    /// Line height.
    pub line_height: LayoutUnit,
    /// Ordered host-visible font fallback chain.
    pub font_candidates: FontFallbackChain,
    /// CSS-compatible numeric font weight.
    pub font_weight: u16,
    /// Requested font slant.
    pub font_style: FontStyle,
    /// CSS-compatible font stretch percentage.
    pub font_stretch: u16,
    /// Additional advance between text clusters.
    pub letter_spacing: LayoutUnit,
    /// BCP-47 paragraph locale.
    pub locale: Arc<str>,
    /// Paragraph base direction.
    pub direction: TextDirection,
    /// Physical top margin in the horizontal writing fallback.
    pub margin_top: LayoutUnit,
    /// Physical right margin in the horizontal writing fallback.
    pub margin_right: LayoutUnit,
    /// Physical bottom margin in the horizontal writing fallback.
    pub margin_bottom: LayoutUnit,
    /// Physical left margin in the horizontal writing fallback.
    pub margin_left: LayoutUnit,
    /// Physical top padding in the horizontal writing fallback.
    pub padding_top: LayoutUnit,
    /// Physical right padding in the horizontal writing fallback.
    pub padding_right: LayoutUnit,
    /// Physical bottom padding in the horizontal writing fallback.
    pub padding_bottom: LayoutUnit,
    /// Physical left padding in the horizontal writing fallback.
    pub padding_left: LayoutUnit,
    /// First-line indent.
    pub text_indent: LayoutUnit,
    /// Semantic/container physical left inset.
    pub block_indent_left: LayoutUnit,
    /// Container physical right inset.
    pub block_indent_right: LayoutUnit,
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
        containing_width: LayoutUnit,
        ancestor_left: LayoutUnit,
        ancestor_right: LayoutUnit,
    ) -> Self {
        let mut style = Self::for_kind(kind, depth);
        if let Some(document_style) = styles.get(style_id) {
            style.apply_document_style(document_style, containing_width);
        }
        style.block_indent_left += ancestor_left;
        style.block_indent_right += ancestor_right;
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
                    font_candidates: FontFallbackChain::default(),
                    font_weight: 700,
                    font_style: FontStyle::Normal,
                    font_stretch: 100,
                    letter_spacing: LayoutUnit::ZERO,
                    locale: Arc::from("und"),
                    direction: TextDirection::Auto,
                    margin_top: LayoutUnit::from_px(14),
                    margin_right: LayoutUnit::ZERO,
                    margin_bottom: LayoutUnit::from_px(8),
                    margin_left: LayoutUnit::ZERO,
                    padding_top: LayoutUnit::ZERO,
                    padding_right: LayoutUnit::ZERO,
                    padding_bottom: LayoutUnit::ZERO,
                    padding_left: LayoutUnit::ZERO,
                    text_indent: LayoutUnit::ZERO,
                    block_indent_left: depth_indent,
                    block_indent_right: LayoutUnit::ZERO,
                    alignment: TextAlignment::Start,
                    keep_with_next: true,
                    break_before: false,
                    break_after: false,
                    break_inside: BreakInside::Auto,
                }
            }
            BlockKind::ListItem => Self {
                block_indent_left: depth_indent + LayoutUnit::from_px(18),
                margin_top: LayoutUnit::from_px(2),
                margin_bottom: LayoutUnit::from_px(4),
                ..Self::default()
            },
            BlockKind::BlockQuote => Self {
                block_indent_left: depth_indent + LayoutUnit::from_px(18),
                padding_left: LayoutUnit::from_px(10),
                margin_top: LayoutUnit::from_px(8),
                margin_bottom: LayoutUnit::from_px(8),
                ..Self::default()
            },
            BlockKind::Image | BlockKind::Unsupported | BlockKind::Divider => Self {
                margin_top: LayoutUnit::from_px(8),
                margin_bottom: LayoutUnit::from_px(8),
                block_indent_left: depth_indent,
                ..Self::default()
            },
            BlockKind::Paragraph | BlockKind::Container => Self {
                block_indent_left: depth_indent,
                ..Self::default()
            },
            BlockKind::ForcedBreak => Self::default(),
        }
    }

    fn apply_document_style(
        &mut self,
        document_style: &DocumentComputedStyle,
        containing_width: LayoutUnit,
    ) {
        // Resolve typography first because `em` geometry is relative to the
        // element's computed font size.
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
        self.font_weight = document_style
            .properties
            .get("font-weight")
            .map_or(self.font_weight, |value| parse_font_weight(value));
        self.font_style = document_style
            .properties
            .get("font-style")
            .map_or(self.font_style, |value| parse_font_style(value));
        self.font_stretch = document_style
            .properties
            .get("font-stretch")
            .map_or(self.font_stretch, |value| parse_font_stretch(value));
        self.letter_spacing = document_style
            .properties
            .get("letter-spacing")
            .and_then(|value| {
                if value.trim().eq_ignore_ascii_case("normal") {
                    Some(LayoutUnit::ZERO)
                } else {
                    resolve_used_length(value, self.font_size, containing_width, true)
                }
            })
            .unwrap_or(self.letter_spacing);
        self.locale = document_style
            .properties
            .get("-pagelet-locale")
            .cloned()
            .unwrap_or_else(|| self.locale.clone());
        self.direction =
            document_style
                .properties
                .get("direction")
                .map_or(self.direction, |value| match value.trim() {
                    "ltr" => TextDirection::Ltr,
                    "rtl" => TextDirection::Rtl,
                    _ => TextDirection::Auto,
                });
        self.font_candidates = font_fallback_chain(
            document_style.properties.get("font-family"),
            self.font_weight,
            self.font_style,
            self.font_stretch,
        );
        let geometry =
            UsedBoxGeometry::from_document(document_style, self.font_size, containing_width);
        let has_margin = document_style.properties.contains_key("margin");
        let has_padding = document_style.properties.contains_key("padding");
        if has_margin || document_style.properties.contains_key("margin-top") {
            self.margin_top = geometry.margin_top;
        }
        if has_margin || document_style.properties.contains_key("margin-right") {
            self.margin_right = geometry.margin_right;
        }
        if has_margin || document_style.properties.contains_key("margin-bottom") {
            self.margin_bottom = geometry.margin_bottom;
        }
        if has_margin || document_style.properties.contains_key("margin-left") {
            self.margin_left = geometry.margin_left;
        }
        if has_padding || document_style.properties.contains_key("padding-top") {
            self.padding_top = geometry.padding_top;
        }
        if has_padding || document_style.properties.contains_key("padding-right") {
            self.padding_right = geometry.padding_right;
        }
        if has_padding || document_style.properties.contains_key("padding-bottom") {
            self.padding_bottom = geometry.padding_bottom;
        }
        if has_padding || document_style.properties.contains_key("padding-left") {
            self.padding_left = geometry.padding_left;
        }
        if document_style.properties.contains_key("text-indent") {
            self.text_indent = geometry.text_indent;
        }
    }
}

#[derive(Debug, Default, Clone, Copy, Eq, PartialEq, Hash)]
struct UsedBoxGeometry {
    margin_top: LayoutUnit,
    margin_right: LayoutUnit,
    margin_bottom: LayoutUnit,
    margin_left: LayoutUnit,
    padding_top: LayoutUnit,
    padding_right: LayoutUnit,
    padding_bottom: LayoutUnit,
    padding_left: LayoutUnit,
    text_indent: LayoutUnit,
}

impl UsedBoxGeometry {
    fn from_document(
        style: &DocumentComputedStyle,
        font_size: LayoutUnit,
        containing_width: LayoutUnit,
    ) -> Self {
        let mut geometry = Self::default();
        if let Some(value) = style.properties.get("margin") {
            if let Some([top, right, bottom, left]) =
                resolve_box_shorthand(value, font_size, containing_width, true)
            {
                geometry.margin_top = top;
                geometry.margin_right = right;
                geometry.margin_bottom = bottom;
                geometry.margin_left = left;
            }
        }
        if let Some(value) = style.properties.get("padding") {
            if let Some([top, right, bottom, left]) =
                resolve_box_shorthand(value, font_size, containing_width, false)
            {
                geometry.padding_top = top;
                geometry.padding_right = right;
                geometry.padding_bottom = bottom;
                geometry.padding_left = left;
            }
        }
        set_used_length(
            &mut geometry.margin_top,
            style.properties.get("margin-top"),
            font_size,
            containing_width,
            true,
        );
        set_used_length(
            &mut geometry.margin_right,
            style.properties.get("margin-right"),
            font_size,
            containing_width,
            true,
        );
        set_used_length(
            &mut geometry.margin_bottom,
            style.properties.get("margin-bottom"),
            font_size,
            containing_width,
            true,
        );
        set_used_length(
            &mut geometry.margin_left,
            style.properties.get("margin-left"),
            font_size,
            containing_width,
            true,
        );
        set_used_length(
            &mut geometry.padding_top,
            style.properties.get("padding-top"),
            font_size,
            containing_width,
            false,
        );
        set_used_length(
            &mut geometry.padding_right,
            style.properties.get("padding-right"),
            font_size,
            containing_width,
            false,
        );
        set_used_length(
            &mut geometry.padding_bottom,
            style.properties.get("padding-bottom"),
            font_size,
            containing_width,
            false,
        );
        set_used_length(
            &mut geometry.padding_left,
            style.properties.get("padding-left"),
            font_size,
            containing_width,
            false,
        );
        set_used_length(
            &mut geometry.text_indent,
            style.properties.get("text-indent"),
            font_size,
            containing_width,
            true,
        );
        geometry
    }

    fn horizontal(self) -> LayoutUnit {
        self.margin_left + self.padding_left + self.padding_right + self.margin_right
    }
}

fn resolve_box_shorthand(
    value: &str,
    font_size: LayoutUnit,
    containing_width: LayoutUnit,
    allow_negative: bool,
) -> Option<[LayoutUnit; 4]> {
    let values = value.split_ascii_whitespace().collect::<Vec<_>>();
    let [top, right, bottom, left] = match values.as_slice() {
        [all] => [*all, *all, *all, *all],
        [vertical, horizontal] => [*vertical, *horizontal, *vertical, *horizontal],
        [top, horizontal, bottom] => [*top, *horizontal, *bottom, *horizontal],
        [top, right, bottom, left] => [*top, *right, *bottom, *left],
        _ => return None,
    };
    Some([
        resolve_used_length(top, font_size, containing_width, allow_negative)?,
        resolve_used_length(right, font_size, containing_width, allow_negative)?,
        resolve_used_length(bottom, font_size, containing_width, allow_negative)?,
        resolve_used_length(left, font_size, containing_width, allow_negative)?,
    ])
}

fn set_used_length(
    target: &mut LayoutUnit,
    value: Option<&Arc<str>>,
    font_size: LayoutUnit,
    containing_width: LayoutUnit,
    allow_negative: bool,
) {
    if let Some(value) = value
        .and_then(|value| resolve_used_length(value, font_size, containing_width, allow_negative))
    {
        *target = value;
    }
}

fn resolve_used_length(
    value: &str,
    font_size: LayoutUnit,
    containing_width: LayoutUnit,
    allow_negative: bool,
) -> Option<LayoutUnit> {
    const MAX_USED_LENGTH_PX: f64 = 1_000_000.0;
    let value = value.trim().to_ascii_lowercase();
    if matches!(value.as_str(), "auto" | "initial" | "inherit" | "unset") {
        return None;
    }
    let pixels = if let Some(number) = value.strip_suffix("rem") {
        parse_number(number)? * 16.0
    } else if let Some(number) = value.strip_suffix("em") {
        parse_number(number)? * font_size.to_f64_px()
    } else if let Some(number) = value.strip_suffix('%') {
        parse_number(number)? * containing_width.to_f64_px() / 100.0
    } else if let Some(number) = value.strip_suffix("px") {
        parse_number(number)?
    } else if value == "0" {
        0.0
    } else {
        return None;
    };
    if !pixels.is_finite() || (!allow_negative && pixels < 0.0) {
        return None;
    }
    Some(LayoutUnit::from_f64_px(
        pixels.clamp(-MAX_USED_LENGTH_PX, MAX_USED_LENGTH_PX),
    ))
}

fn parse_number(value: &str) -> Option<f64> {
    value
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

fn parse_font_weight(value: &str) -> u16 {
    match value.trim().to_ascii_lowercase().as_str() {
        "normal" => 400,
        "bold" | "bolder" => 700,
        "lighter" => 300,
        value => value
            .parse::<u16>()
            .ok()
            .map_or(400, |weight| weight.clamp(1, 1_000)),
    }
}

fn parse_font_style(value: &str) -> FontStyle {
    match value.trim().to_ascii_lowercase().as_str() {
        "italic" => FontStyle::Italic,
        value if value.starts_with("oblique") => FontStyle::Oblique,
        _ => FontStyle::Normal,
    }
}

fn parse_font_stretch(value: &str) -> u16 {
    let value = value.trim().to_ascii_lowercase();
    let keyword = match value.as_str() {
        "ultra-condensed" => Some(50),
        "extra-condensed" => Some(62),
        "condensed" => Some(75),
        "semi-condensed" => Some(87),
        "normal" => Some(100),
        "semi-expanded" => Some(112),
        "expanded" => Some(125),
        "extra-expanded" => Some(150),
        "ultra-expanded" => Some(200),
        _ => None,
    };
    keyword.unwrap_or_else(|| {
        value
            .strip_suffix('%')
            .and_then(|number| number.trim().parse::<u16>().ok())
            .map_or(100, |stretch| stretch.clamp(1, 1_000))
    })
}

fn font_fallback_chain(
    value: Option<&Arc<str>>,
    weight: u16,
    style: FontStyle,
    stretch: u16,
) -> FontFallbackChain {
    let mut families = value
        .map(|value| {
            value
                .split(',')
                .filter_map(|family| {
                    let family = family.trim().trim_matches(['\'', '"']);
                    (!family.is_empty()).then(|| Arc::<str>::from(family))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if families.is_empty() {
        families.push(Arc::from("serif"));
    }
    let mut descriptors = families.into_iter().map(|family| FontDescriptor {
        font_id: None,
        family,
        weight,
        style,
        stretch,
        fingerprint: FontSetFingerprint::default(),
    });
    let primary = descriptors.next().unwrap_or_default();
    FontFallbackChain {
        primary,
        fallbacks: descriptors.collect(),
    }
}

impl Default for ComputedLayoutStyle {
    fn default() -> Self {
        Self {
            font_size: LayoutUnit::from_px(16),
            line_height: LayoutUnit::from_px(20),
            font_candidates: FontFallbackChain::default(),
            font_weight: 400,
            font_style: FontStyle::Normal,
            font_stretch: 100,
            letter_spacing: LayoutUnit::ZERO,
            locale: Arc::from("und"),
            direction: TextDirection::Auto,
            margin_top: LayoutUnit::ZERO,
            margin_right: LayoutUnit::ZERO,
            margin_bottom: LayoutUnit::ZERO,
            margin_left: LayoutUnit::ZERO,
            padding_top: LayoutUnit::ZERO,
            padding_right: LayoutUnit::ZERO,
            padding_bottom: LayoutUnit::ZERO,
            padding_left: LayoutUnit::ZERO,
            text_indent: LayoutUnit::ZERO,
            block_indent_left: LayoutUnit::ZERO,
            block_indent_right: LayoutUnit::ZERO,
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
    /// Legacy v1 text line. New scenes use [`TextPaintFragment`].
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

/// A point in page-local logical pixels.
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq, Hash)]
pub struct Point {
    /// Horizontal coordinate.
    pub x: LayoutUnit,
    /// Vertical coordinate.
    pub y: LayoutUnit,
}

/// One measured line retained for paragraph replay.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SceneParagraphLine {
    /// Host-provided metrics, including the original baseline.
    pub metrics: LineMetrics,
    /// Paragraph-local layout occupancy.
    pub layout_rect: Rect,
    /// Paragraph-local glyph ink, which may exceed the advance width.
    pub ink_bounds: Rect,
}

/// Complete measured paragraph retained as the authoritative paint input.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SceneParagraph {
    /// Stable paragraph id shared by every page fragment of this paragraph.
    pub paragraph_id: u32,
    /// Fingerprint of every input that produced the measurement request.
    pub request_fingerprint: u64,
    /// Host-provided fingerprint of the measured result.
    pub measurement_fingerprint: u64,
    /// Complete paragraph text.
    pub text: Arc<str>,
    /// UTF-8 byte range measured inside `text`.
    pub text_range: std::ops::Range<u32>,
    /// Exact style runs sent to the shaping adapter.
    pub style_runs: Vec<TextStyleRun>,
    /// Block-level font size supplied to the adapter.
    pub font_size: LayoutUnit,
    /// Final wrapping width supplied to the adapter.
    pub available_width: LayoutUnit,
    /// Maximum wrapping width supplied to the adapter.
    pub max_width: LayoutUnit,
    /// Paragraph locale supplied to the adapter.
    pub locale: Arc<str>,
    /// Paragraph direction supplied to the adapter.
    pub direction: TextDirection,
    /// Text scale supplied to the adapter.
    pub text_scale: LayoutUnit,
    /// Block-level fallback chain supplied to the adapter.
    pub font_candidates: FontFallbackChain,
    /// Strut supplied to the adapter.
    pub strut: StrutStyle,
    /// Height behavior supplied to the adapter.
    pub height_behavior: HeightBehavior,
    /// Paragraph-local measured line geometry.
    pub lines: Vec<SceneParagraphLine>,
    /// Host-provided cluster map for hit testing and selections.
    pub clusters: Vec<TextCluster>,
}

/// Cache/replay identity of one measured paragraph.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub struct SceneParagraphIdentity {
    /// Stable paragraph id.
    pub paragraph_id: u32,
    /// Fingerprint of the complete measurement request.
    pub request_fingerprint: u64,
    /// Fingerprint of the host measurement result.
    pub measurement_fingerprint: u64,
}

impl SceneParagraph {
    /// Return the complete identity required to reuse this paragraph measurement.
    #[must_use]
    pub const fn replay_identity(&self) -> SceneParagraphIdentity {
        SceneParagraphIdentity {
            paragraph_id: self.paragraph_id,
            request_fingerprint: self.request_fingerprint,
            measurement_fingerprint: self.measurement_fingerprint,
        }
    }
}

/// Complete identity required to reuse text in a cached page scene.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SceneTextIdentity {
    /// Text backend that measured the scene.
    pub text_backend_id: crate::text::TextBackendId,
    /// Font set used by the backend.
    pub font_fingerprint: FontSetFingerprint,
    /// Paragraph identities in scene-table order.
    pub paragraphs: Vec<SceneParagraphIdentity>,
}

/// One page-visible clipped replay of a complete measured paragraph.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct TextPaintFragment {
    /// Stable fragment id within the page.
    pub id: u32,
    /// Source semantic node.
    pub node_id: NodeId,
    /// Paragraph referenced from [`PageScene::paragraphs`].
    pub paragraph_id: u32,
    /// UTF-8 bytes visible on this page.
    pub visible_text_range: std::ops::Range<u32>,
    /// Origin used to paint the complete paragraph.
    pub paint_origin: Point,
    /// Layout occupancy of the visible lines on this page.
    pub layout_rect: Rect,
    /// Page clip; never derived from a line advance width.
    pub clip_rect: Rect,
    /// First visible line inside the complete paragraph.
    pub first_line: u32,
    /// Number of visible lines.
    pub line_count: u32,
    /// Optional source XHTML range.
    pub source_range: Option<SourceRange>,
    /// Semantic range visible on this page.
    pub anchor_range: TextAnchorRange,
    /// True only when an oversized line overflowed the page.
    pub overflow: bool,
}

/// Clickable link region.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct LinkRegion {
    /// Region bounds.
    pub rect: Rect,
    /// Source node.
    pub node_id: NodeId,
    /// Paragraph-local UTF-8 range covered by the link.
    pub text_range: Option<std::ops::Range<u32>>,
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
    /// Text backend that produced every paragraph measurement on this page.
    pub text_backend_id: crate::text::TextBackendId,
    /// Font set used by the measurement adapter.
    pub font_fingerprint: FontSetFingerprint,
    /// Complete measured paragraphs referenced by text paint fragments.
    pub paragraphs: Vec<SceneParagraph>,
    /// Clipped paragraph replay fragments.
    pub text_paints: Vec<TextPaintFragment>,
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

    /// Return the identity tuple that must match before cached text can be replayed.
    #[must_use]
    pub fn text_replay_identity(&self) -> SceneTextIdentity {
        SceneTextIdentity {
            text_backend_id: self.text_backend_id,
            font_fingerprint: self.font_fingerprint,
            paragraphs: self
                .paragraphs
                .iter()
                .map(SceneParagraph::replay_identity)
                .collect(),
        }
    }

    /// Reject a cached scene when its backend, font, request, or measurement identity changed.
    pub fn validate_text_replay_identity(
        &self,
        expected: &SceneTextIdentity,
    ) -> Result<(), PageletError> {
        if &self.text_replay_identity() == expected {
            Ok(())
        } else {
            Err(layout_error(
                "cached page scene text replay identity does not match",
            ))
        }
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
    blocks: Vec<LayoutBlock>,
    measure_batch: MeasureBatch,
}

impl HostMeasuredLayout {
    /// Prepare a chapter for one batched host measurement round trip.
    #[must_use]
    pub fn prepare(chapter: ChapterIr, options: LayoutOptions) -> Self {
        let blocks = layout_blocks(&chapter, options.constraints);
        let measure_batch = prepare_measure_batch_from_blocks(&blocks, options.constraints);
        Self {
            chapter,
            options,
            blocks,
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
        paginate_prepared_blocks(&self.chapter, &self.blocks, &backend, self.options)
    }
}

/// Build the single host measurement batch required to paginate a chapter.
#[must_use]
pub fn prepare_measure_batch(chapter: &ChapterIr, options: LayoutOptions) -> MeasureBatch {
    let blocks = layout_blocks(chapter, options.constraints);
    prepare_measure_batch_from_blocks(&blocks, options.constraints)
}

fn prepare_measure_batch_from_blocks(
    blocks: &[LayoutBlock],
    constraints: LayoutConstraints,
) -> MeasureBatch {
    let requests = blocks
        .iter()
        .enumerate()
        .filter_map(|(index, block)| {
            let LayoutBlockContent::Text {
                text, style_runs, ..
            } = &block.content
            else {
                return None;
            };
            Some(measure_request(
                u32::try_from(index).unwrap_or(u32::MAX),
                block.node_id.get(),
                text,
                &block.style,
                style_runs,
                measurement_available_width(constraints, &block.style),
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
        push_u32(
            &mut out,
            1,
            "scene_wire",
            EngineVersions::CURRENT.scene_wire,
            true,
        );
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
    let blocks = layout_blocks(chapter, options.constraints);
    paginate_prepared_blocks(chapter, &blocks, text_backend, options)
}

fn paginate_prepared_blocks(
    chapter: &ChapterIr,
    blocks: &[LayoutBlock],
    text_backend: &dyn TextBackend,
    options: LayoutOptions,
) -> Result<PaginatedDocument, PageletError> {
    let cancel = CancellationToken::new();
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
            paginate_page_from_blocks(chapter, blocks, text_backend, options, &cancel, start)?
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
    let blocks = layout_blocks(chapter, options.constraints);
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
    for paint in &page.text_paints {
        if !paint.clip_rect.contains(x, y) || !paint.layout_rect.contains(x, y) {
            continue;
        }
        let paragraph = page
            .paragraphs
            .iter()
            .find(|paragraph| paragraph.paragraph_id == paint.paragraph_id)?;
        let local_x = x - paint.paint_origin.x;
        let local_y = y - paint.paint_origin.y;
        let first_line = usize::try_from(paint.first_line).ok()?;
        let last_line = first_line
            .saturating_add(usize::try_from(paint.line_count).ok()?)
            .min(paragraph.lines.len());
        let (line_index, line) = paragraph.lines[first_line..last_line]
            .iter()
            .enumerate()
            .find(|(_, line)| {
                local_y >= line.layout_rect.y
                    && local_y <= line.layout_rect.y + line.layout_rect.height
            })
            .map(|(relative, line)| (first_line + relative, line))?;
        let line_x = local_x - line.layout_rect.x;
        let mut line_clusters = paragraph
            .clusters
            .iter()
            .filter(|cluster| {
                usize::try_from(cluster.line_index).ok() == Some(line_index)
                    && cluster.text_end > paint.visible_text_range.start
                    && cluster.text_start < paint.visible_text_range.end
            })
            .peekable();
        if line_clusters.peek().is_some() {
            let mut nearest = None;
            for cluster in line_clusters {
                let distance = if line_x < cluster.x_start {
                    cluster.x_start - line_x
                } else if line_x > cluster.x_end {
                    line_x - cluster.x_end
                } else {
                    LayoutUnit::ZERO
                };
                if nearest.is_none_or(|(_, best): (&TextCluster, LayoutUnit)| distance < best) {
                    nearest = Some((cluster, distance));
                }
            }
            if let Some((cluster, _)) = nearest {
                let midpoint = LayoutUnit::from_raw(
                    cluster.x_start.raw().saturating_add(cluster.x_end.raw()) / 2,
                );
                let (offset, affinity) = if line_x <= midpoint {
                    (cluster.text_start, TextAffinity::Downstream)
                } else {
                    (cluster.text_end, TextAffinity::Upstream)
                };
                return Some(HitTestResult {
                    node_id: paint.node_id,
                    utf8_byte_offset: offset
                        .clamp(paint.visible_text_range.start, paint.visible_text_range.end),
                    affinity,
                    fragment_id: paint.id,
                });
            }
        }

        let width = line.metrics.width.raw().max(1);
        let span = line
            .metrics
            .text_end
            .saturating_sub(line.metrics.text_start)
            .max(1);
        let relative_x = line_x.raw().clamp(0, width);
        let offset = line.metrics.text_start.saturating_add(
            u32::try_from((i64::from(span) * relative_x) / width).unwrap_or(u32::MAX),
        );
        return Some(HitTestResult {
            node_id: paint.node_id,
            utf8_byte_offset: offset
                .clamp(paint.visible_text_range.start, paint.visible_text_range.end),
            affinity: TextAffinity::Downstream,
            fragment_id: paint.id,
        });
    }

    // Explicit pageletScene v1 fallback.
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
    let mut paragraph_identities = BTreeMap::<u32, (u64, u64, Arc<str>)>::new();
    for page in pages {
        if page.fragments.is_empty()
            && page.text_paints.is_empty()
            && page.next_break_token.is_some()
        {
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
        for paragraph in &page.paragraphs {
            let identity = (
                paragraph.request_fingerprint,
                paragraph.measurement_fingerprint,
                paragraph.text.clone(),
            );
            if let Some(previous) =
                paragraph_identities.insert(paragraph.paragraph_id, identity.clone())
            {
                if previous != identity {
                    return Err(layout_error(
                        "paragraph replay identity changed between pages",
                    ));
                }
            }
        }
        for paint in &page.text_paints {
            if paint.layout_rect.width.raw() < 0
                || paint.layout_rect.height.raw() < 0
                || paint.clip_rect.width.raw() < 0
                || paint.clip_rect.height.raw() < 0
            {
                return Err(layout_error("text paint extent is negative"));
            }
            if paint.visible_text_range.end < paint.visible_text_range.start {
                return Err(layout_error("text paint range is reversed"));
            }
            let paragraph = page
                .paragraphs
                .iter()
                .find(|paragraph| paragraph.paragraph_id == paint.paragraph_id)
                .ok_or_else(|| layout_error("text paint references a missing paragraph"))?;
            if paint.visible_text_range.start < paragraph.text_range.start
                || paint.visible_text_range.end > paragraph.text_range.end
            {
                return Err(layout_error("text paint range is outside its paragraph"));
            }
            let first_line = usize::try_from(paint.first_line).unwrap_or(usize::MAX);
            let line_count = usize::try_from(paint.line_count).unwrap_or(usize::MAX);
            if line_count == 0
                || first_line
                    .checked_add(line_count)
                    .is_none_or(|end| end > paragraph.lines.len())
            {
                return Err(layout_error(
                    "text paint line range is outside its paragraph",
                ));
            }
            if let Some(previous) =
                last_range_by_node.insert(paint.node_id, paint.visible_text_range.end)
            {
                if paint.visible_text_range.start < previous {
                    return Err(layout_error("text paint ranges overlap"));
                }
            }
        }
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
        let request = measure_request(
            0,
            self.node_id.get(),
            &self.text,
            &self.style,
            &[],
            measurement_available_width(context.options.constraints, &self.style),
        );
        let measured = measure_text(context.text_backend, context.cancel, &request, &self.style)?;
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
        let request = measure_request(
            0,
            self.node_id.get(),
            &self.text,
            &self.style,
            &[],
            measurement_available_width(context.options.constraints, &self.style),
        );
        let measured = measure_text(context.text_backend, context.cancel, &request, &self.style)?;
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

    let mut page = PageBuilder::new(
        start.page_index,
        options.constraints,
        text_backend.backend_id(),
        text_backend.font_fingerprint(),
    );
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
        if page.fragments.len().saturating_add(page.text_paints.len())
            >= usize::try_from(options.limits.max_layout_fragments).unwrap_or(usize::MAX)
        {
            return Err(PageletError::ResourceLimitExceeded(
                ResourceLimitError::new(
                    ResourceLimitKind::LayoutFragments,
                    u64::from(options.limits.max_layout_fragments),
                    u64::try_from(page.fragments.len().saturating_add(page.text_paints.len()))
                        .unwrap_or(u64::MAX),
                ),
            ));
        }
        let block = &blocks[index];
        if block.style.break_before && !page.is_empty() {
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
                if page.is_empty() {
                    index += 1;
                    text_offset = 0;
                    continue;
                }
                next_token =
                    next_block_token(token_context, blocks, index + 1, start.page_index + 1);
                break;
            }
            LayoutBlockContent::Text {
                text,
                marker,
                style_runs,
            } => {
                let available_width =
                    measurement_available_width(options.constraints, &block.style);
                let request = measure_request(
                    u32::try_from(index).unwrap_or(u32::MAX),
                    block.node_id.get(),
                    text,
                    &block.style,
                    style_runs,
                    available_width,
                );
                let measured = measure_text(text_backend, cancel, &request, &block.style)?;
                if block.style.keep_with_next
                    && !page.is_empty()
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
                    marker: marker.as_deref(),
                    request: &request,
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

    page.links = link_regions(
        chapter,
        &page.text_paints,
        &page.paragraphs,
        &page.fragments,
    );
    page.anchors = anchor_regions(
        chapter,
        &page.text_paints,
        &page.paragraphs,
        &page.fragments,
    );
    page.selections = selection_maps(&page.text_paints, &page.paragraphs, &page.fragments);
    page.semantics = semantic_nodes(&page.text_paints, &page.paragraphs, &page.fragments);
    page.diagnostics = diagnostics;
    page.next_break_token = next_token;
    page.fingerprint = fingerprint_page(chapter, options, text_backend, &page);

    Ok(Some(page.finish()))
}

#[derive(Debug)]
struct PageBuilder {
    page_index: u32,
    size: PageSize,
    constraints: LayoutConstraints,
    y: LayoutUnit,
    pending_margin_bottom: LayoutUnit,
    text_backend_id: crate::text::TextBackendId,
    font_fingerprint: FontSetFingerprint,
    paragraphs: Vec<SceneParagraph>,
    text_paints: Vec<TextPaintFragment>,
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
    marker: Option<&'a str>,
    request: &'a MeasureRequest,
    measured: &'a MeasuredText,
    text_offset: u32,
    content_bottom: LayoutUnit,
}

impl PageBuilder {
    fn new(
        page_index: u32,
        constraints: LayoutConstraints,
        text_backend_id: crate::text::TextBackendId,
        font_fingerprint: FontSetFingerprint,
    ) -> Self {
        Self {
            page_index,
            size: PageSize {
                width: constraints.viewport_width,
                height: constraints.viewport_height,
            },
            constraints,
            y: constraints.margin_top,
            pending_margin_bottom: LayoutUnit::ZERO,
            text_backend_id,
            font_fingerprint,
            paragraphs: Vec::new(),
            text_paints: Vec::new(),
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
            marker,
            request,
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

        let is_continuation = text_offset > 0;
        let top_spacing = self.block_start_spacing(&block.style, is_continuation);
        if self.y + top_spacing >= content_bottom && !self.is_empty() {
            return Ok(BlockPushOutcome::NoFit);
        }
        let mut y = self.y + top_spacing;
        let mut fit_count = 0_usize;
        let total_remaining = measured.lines.len() - start_line;

        for line in &measured.lines[start_line..] {
            let completes_block = fit_count + 1 == total_remaining;
            let required_bottom = if completes_block {
                block.style.padding_bottom
            } else {
                LayoutUnit::ZERO
            };
            if fit_count > 0 && y + line.line_height + required_bottom > content_bottom {
                break;
            }
            if fit_count == 0
                && y + line.line_height + required_bottom > content_bottom
                && !self.is_empty()
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
            if fit_count == 1 && !self.is_empty() {
                return Ok(BlockPushOutcome::NoFit);
            }
            if total_remaining - fit_count == 1 && fit_count > 1 {
                fit_count -= 1;
            }
        }

        self.y += top_spacing;
        self.pending_margin_bottom = LayoutUnit::ZERO;
        let paragraph = scene_paragraph(request, measured, &block.style, options.constraints);
        let block_origin_x = block_x(options.constraints, &block.style);
        let first_local_line = paragraph.lines[start_line].layout_rect;
        let paint_origin = Point {
            x: block_origin_x,
            y: self.y - first_local_line.y,
        };
        if start_line == 0 {
            if let Some(marker) = marker {
                let fragment_id = self.alloc_fragment_id();
                self.push_fragment(SceneFragment {
                    id: fragment_id,
                    kind: SceneFragmentKind::Marker,
                    node_id: block.node_id,
                    rect: Rect {
                        x: (paint_origin.x + first_local_line.x - LayoutUnit::from_px(16))
                            .max(options.constraints.margin_start),
                        y: self.y,
                        width: LayoutUnit::from_px(12),
                        height: first_local_line.height,
                    },
                    text: Some(Arc::from(marker)),
                    source_range: block.source_range,
                    anchor_range: None,
                    line_index: Some(0),
                    overflow: false,
                });
            }
        }
        let last_line_index = start_line + fit_count - 1;
        let text_start = measured.lines[start_line].text_start.max(text_offset);
        let text_end = measured.lines[last_line_index].text_end;
        let visible_height = paragraph.lines[last_line_index].layout_rect.y
            + paragraph.lines[last_line_index].layout_rect.height
            - first_local_line.y;
        let layout_rect = Rect {
            x: block_origin_x,
            y: self.y,
            width: block_available_width(options.constraints, &block.style),
            height: visible_height,
        };
        let anchor_range = TextAnchorRange {
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
        let paint_id = self.alloc_fragment_id();
        self.text_paints.push(TextPaintFragment {
            id: paint_id,
            node_id: block.node_id,
            paragraph_id: request.paragraph_id,
            visible_text_range: text_start..text_end,
            paint_origin,
            layout_rect,
            clip_rect: Rect {
                x: options.constraints.margin_start,
                y: options.constraints.margin_top,
                width: options.constraints.content_width(),
                height: options.constraints.content_height(),
            },
            first_line: u32::try_from(start_line).unwrap_or(u32::MAX),
            line_count: u32::try_from(fit_count).unwrap_or(u32::MAX),
            source_range: block.source_range,
            anchor_range,
            overflow: self.y + visible_height > content_bottom,
        });
        if !self
            .paragraphs
            .iter()
            .any(|existing| existing.paragraph_id == paragraph.paragraph_id)
        {
            self.paragraphs.push(paragraph);
        }
        let line_y = self.y + visible_height;

        if start_line + fit_count >= measured.lines.len() {
            self.y = line_y + block.style.padding_bottom;
            self.pending_margin_bottom = block.style.margin_bottom;
            Ok(BlockPushOutcome::Complete)
        } else {
            self.y = line_y;
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
        let top_spacing = self.block_start_spacing(&block.style, false);
        if self.y + top_spacing + height + block.style.padding_bottom > content_bottom
            && !self.is_empty()
        {
            return false;
        }
        self.y += top_spacing;
        self.pending_margin_bottom = LayoutUnit::ZERO;
        let fragment_id = self.alloc_fragment_id();
        self.push_fragment(SceneFragment {
            id: fragment_id,
            kind,
            node_id: block.node_id,
            rect: Rect {
                x: block_x(self.constraints, &block.style),
                y: self.y,
                width: block_available_width(self.constraints, &block.style),
                height,
            },
            text,
            source_range: block.source_range,
            anchor_range: None,
            line_index: None,
            overflow: self.y + height > content_bottom,
        });
        self.y += height + block.style.padding_bottom;
        self.pending_margin_bottom = block.style.margin_bottom;
        true
    }

    fn push_image_box(
        &mut self,
        block: &LayoutBlock,
        image: &LayoutImage,
        content_bottom: LayoutUnit,
    ) -> (bool, Option<Diagnostic>) {
        let top_spacing = self.block_start_spacing(&block.style, false);
        let available_width = block_available_width(self.constraints, &block.style);
        let available_height = (self.constraints.content_height()
            - block.style.padding_top
            - block.style.padding_bottom)
            .max(LayoutUnit::from_px(1));
        let (width, mut height) =
            resolve_image_size(image, &block.style, available_width, available_height);
        if self.y + top_spacing + height + block.style.padding_bottom > content_bottom
            && !self.is_empty()
        {
            return (false, None);
        }

        self.y += top_spacing;
        self.pending_margin_bottom = LayoutUnit::ZERO;
        let mut diagnostic = None;
        if self.y + height > content_bottom {
            height = (content_bottom - self.y).max(LayoutUnit::from_px(1));
            diagnostic = Some(Diagnostic::new(
                DiagnosticCode::Layout,
                Severity::Warning,
                "image height exceeded page content and was clipped",
            ));
        }
        let base_x = block_x(self.constraints, &block.style);
        let remaining_inline = (available_width - width).max(LayoutUnit::ZERO);
        let x = match (image.layout_role, block.style.alignment) {
            (ImageLayoutRole::Cover | ImageLayoutRole::Standalone, _)
            | (_, TextAlignment::Center) => {
                base_x + LayoutUnit::from_raw(remaining_inline.raw() / 2)
            }
            (_, TextAlignment::End) => base_x + remaining_inline,
            (_, TextAlignment::Start | TextAlignment::Justify) => base_x,
        };
        let vertically_centered = matches!(
            image.layout_role,
            ImageLayoutRole::Cover | ImageLayoutRole::Standalone
        ) && !image.has_authored_size();
        if vertically_centered {
            let remaining_block = (content_bottom - self.y - block.style.padding_bottom - height)
                .max(LayoutUnit::ZERO);
            self.y += LayoutUnit::from_raw(remaining_block.raw() / 2);
        }
        let fragment_id = self.alloc_fragment_id();
        self.push_fragment(SceneFragment {
            id: fragment_id,
            kind: SceneFragmentKind::Image,
            node_id: block.node_id,
            rect: Rect {
                x,
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
        self.y += height + block.style.padding_bottom;
        self.pending_margin_bottom = block.style.margin_bottom;
        (true, diagnostic)
    }

    fn block_start_spacing(
        &self,
        style: &ComputedLayoutStyle,
        is_continuation: bool,
    ) -> LayoutUnit {
        if is_continuation {
            LayoutUnit::ZERO
        } else {
            collapse_margins(self.pending_margin_bottom, style.margin_top) + style.padding_top
        }
    }

    fn alloc_fragment_id(&mut self) -> u32 {
        let id = self.next_fragment_id;
        self.next_fragment_id = self.next_fragment_id.saturating_add(1);
        id
    }

    fn push_fragment(&mut self, fragment: SceneFragment) {
        self.fragments.push(fragment);
    }

    fn is_empty(&self) -> bool {
        self.fragments.is_empty() && self.text_paints.is_empty()
    }

    fn finish(self) -> PageScene {
        let start_anchor = self
            .text_paints
            .iter()
            .map(|fragment| fragment.anchor_range.start)
            .chain(
                self.fragments
                    .iter()
                    .filter_map(|fragment| fragment.anchor_range.map(|range| range.start)),
            )
            .min_by_key(|anchor| (anchor.node_id, anchor.utf8_byte_offset));
        let end_anchor = self
            .text_paints
            .iter()
            .map(|fragment| fragment.anchor_range.end)
            .chain(
                self.fragments
                    .iter()
                    .filter_map(|fragment| fragment.anchor_range.map(|range| range.end)),
            )
            .max_by_key(|anchor| (anchor.node_id, anchor.utf8_byte_offset));
        PageScene {
            page_index: self.page_index,
            size: self.size,
            start_anchor,
            end_anchor,
            text_backend_id: self.text_backend_id,
            font_fingerprint: self.font_fingerprint,
            paragraphs: self.paragraphs,
            text_paints: self.text_paints,
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
        style_runs: Vec<TextStyleRun>,
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
    layout_role: ImageLayoutRole,
    width: Option<Arc<str>>,
    height: Option<Arc<str>>,
    max_width: Option<Arc<str>>,
    max_height: Option<Arc<str>>,
}

impl LayoutImage {
    fn has_authored_size(&self) -> bool {
        self.width.is_some()
            || self.height.is_some()
            || self.max_width.is_some()
            || self.max_height.is_some()
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct ContainingBlock {
    inline_width: LayoutUnit,
    inset_left: LayoutUnit,
    inset_right: LayoutUnit,
}

struct LeafLayoutContext {
    depth: u32,
    marker: Option<Arc<str>>,
    containing: ContainingBlock,
}

impl ContainingBlock {
    fn root(constraints: LayoutConstraints) -> Self {
        Self {
            inline_width: constraints.content_width(),
            inset_left: LayoutUnit::ZERO,
            inset_right: LayoutUnit::ZERO,
        }
    }

    fn nested(self, geometry: UsedBoxGeometry) -> Self {
        let inline_width = self.inline_width - geometry.horizontal();
        Self {
            inline_width: if inline_width.raw() <= 0 {
                LayoutUnit::from_px(1)
            } else {
                inline_width
            },
            inset_left: self.inset_left + geometry.margin_left + geometry.padding_left,
            inset_right: self.inset_right + geometry.padding_right + geometry.margin_right,
        }
    }
}

fn layout_blocks(chapter: &ChapterIr, constraints: LayoutConstraints) -> Vec<LayoutBlock> {
    let mut blocks = Vec::new();
    collect_blocks(
        chapter,
        chapter.root,
        0,
        None,
        ContainingBlock::root(constraints),
        &mut blocks,
    );
    blocks
}

fn collect_blocks(
    chapter: &ChapterIr,
    node_id: NodeId,
    depth: u32,
    marker: Option<Arc<str>>,
    containing: ContainingBlock,
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
            text,
            LeafLayoutContext {
                depth,
                marker,
                containing,
            },
            blocks,
        ),
        DocumentNode::Heading(heading) => push_text_block(
            chapter,
            node_id,
            BlockKind::Heading(heading.level),
            &heading.content,
            LeafLayoutContext {
                depth,
                marker,
                containing,
            },
            blocks,
        ),
        DocumentNode::List(list) => {
            let start = blocks.len();
            let (geometry, child_containing) =
                container_geometry(&chapter.styles, list.style, containing);
            for (index, child) in list.children.iter().enumerate() {
                let marker = if list.ordered {
                    Arc::from(format!("{}.", index + 1))
                } else {
                    Arc::from("•")
                };
                collect_blocks(
                    chapter,
                    *child,
                    depth + 1,
                    Some(marker),
                    child_containing,
                    blocks,
                );
            }
            apply_container_vertical_geometry(blocks, start, geometry);
        }
        DocumentNode::ListItem(item) => {
            let start = blocks.len();
            let (geometry, child_containing) =
                container_geometry(&chapter.styles, item.style, containing);
            if item.children.is_empty() {
                blocks.push(layout_placeholder(
                    chapter,
                    node_id,
                    BlockKind::ListItem,
                    item.style,
                    "empty-list-item",
                    depth,
                    child_containing,
                ));
            }
            let mut pending_marker = marker;
            for child in &item.children {
                collect_blocks(
                    chapter,
                    *child,
                    depth,
                    pending_marker.take(),
                    child_containing,
                    blocks,
                );
            }
            apply_container_vertical_geometry(blocks, start, geometry);
        }
        DocumentNode::BlockQuote(container) => {
            let start = blocks.len();
            let (geometry, child_containing) =
                container_geometry(&chapter.styles, container.style, containing);
            if container.children.is_empty() {
                blocks.push(layout_placeholder(
                    chapter,
                    node_id,
                    BlockKind::BlockQuote,
                    container.style,
                    "blockquote",
                    depth + 1,
                    child_containing,
                ));
            }
            let mut pending_marker = marker;
            for child in &container.children {
                collect_blocks(
                    chapter,
                    *child,
                    depth + 1,
                    pending_marker.take(),
                    child_containing,
                    blocks,
                );
            }
            apply_container_vertical_geometry(blocks, start, geometry);
        }
        DocumentNode::Figure(container) => {
            let start = blocks.len();
            let (geometry, child_containing) =
                container_geometry(&chapter.styles, container.style, containing);
            if container.children.is_empty() {
                blocks.push(layout_placeholder(
                    chapter,
                    node_id,
                    BlockKind::Container,
                    container.style,
                    "container",
                    depth,
                    child_containing,
                ));
            }
            let mut pending_marker = marker;
            for child in &container.children {
                collect_blocks(
                    chapter,
                    *child,
                    depth,
                    pending_marker.take(),
                    child_containing,
                    blocks,
                );
            }
            apply_container_vertical_geometry(blocks, start, geometry);
        }
        DocumentNode::Container(container) => {
            let start = blocks.len();
            let (geometry, child_containing) =
                container_geometry(&chapter.styles, container.style, containing);
            let mut pending_marker = marker;
            for child in &container.children {
                collect_blocks(
                    chapter,
                    *child,
                    depth,
                    pending_marker.take(),
                    child_containing,
                    blocks,
                );
            }
            apply_container_vertical_geometry(blocks, start, geometry);
        }
        DocumentNode::Table(container) => {
            blocks.push(layout_placeholder(
                chapter,
                node_id,
                BlockKind::Unsupported,
                container.style,
                "unsupported:table",
                depth,
                containing,
            ));
            for child in &container.children {
                collect_blocks(chapter, *child, depth + 1, None, containing, blocks);
            }
        }
        DocumentNode::Image(image) => {
            blocks.push(layout_image(chapter, node_id, image, depth, containing));
        }
        DocumentNode::Divider => blocks.push(LayoutBlock {
            node_id,
            kind: BlockKind::Divider,
            content: LayoutBlockContent::Divider,
            style: with_containing_insets(
                ComputedLayoutStyle::for_kind(BlockKind::Divider, depth),
                containing,
            ),
            source_range: chapter.source_map.get(node_id),
        }),
        DocumentNode::ForcedBreak => blocks.push(LayoutBlock {
            node_id,
            kind: BlockKind::ForcedBreak,
            content: LayoutBlockContent::ForcedBreak,
            style: with_containing_insets(
                ComputedLayoutStyle::for_kind(BlockKind::ForcedBreak, depth),
                containing,
            ),
            source_range: chapter.source_map.get(node_id),
        }),
        DocumentNode::Footnote(note) => {
            let start = blocks.len();
            let (geometry, child_containing) =
                container_geometry(&chapter.styles, note.style, containing);
            let mut pending_marker = marker;
            for child in &note.children {
                collect_blocks(
                    chapter,
                    *child,
                    depth + 1,
                    pending_marker.take(),
                    child_containing,
                    blocks,
                );
            }
            apply_container_vertical_geometry(blocks, start, geometry);
        }
        DocumentNode::Unsupported(unsupported) => {
            blocks.push(layout_placeholder(
                chapter,
                node_id,
                BlockKind::Unsupported,
                unsupported.style,
                &format!("unsupported:{}", unsupported.element),
                depth,
                containing,
            ));
            for child in &unsupported.children {
                collect_blocks(chapter, *child, depth + 1, None, containing, blocks);
            }
        }
    }
}

fn container_geometry(
    styles: &StyleTable,
    style_id: crate::core::StyleId,
    containing: ContainingBlock,
) -> (UsedBoxGeometry, ContainingBlock) {
    let geometry = styles
        .get(style_id)
        .map_or_else(UsedBoxGeometry::default, |style| {
            let font_size = style
                .properties
                .get("font-size")
                .and_then(|value| parse_px(value))
                .unwrap_or(LayoutUnit::from_px(16));
            UsedBoxGeometry::from_document(style, font_size, containing.inline_width)
        });
    (geometry, containing.nested(geometry))
}

fn apply_container_vertical_geometry(
    blocks: &mut [LayoutBlock],
    start: usize,
    geometry: UsedBoxGeometry,
) {
    let Some(first) = blocks.get_mut(start) else {
        return;
    };
    if geometry.padding_top.raw() == 0 {
        first.style.margin_top = collapse_margins(geometry.margin_top, first.style.margin_top);
    } else {
        first.style.padding_top += geometry.padding_top + first.style.margin_top;
        first.style.margin_top = geometry.margin_top;
    }
    let Some(last) = blocks.last_mut() else {
        return;
    };
    if geometry.padding_bottom.raw() == 0 {
        last.style.margin_bottom =
            collapse_margins(last.style.margin_bottom, geometry.margin_bottom);
    } else {
        last.style.padding_bottom += geometry.padding_bottom + last.style.margin_bottom;
        last.style.margin_bottom = geometry.margin_bottom;
    }
}

fn collapse_margins(first: LayoutUnit, second: LayoutUnit) -> LayoutUnit {
    match (first.raw() >= 0, second.raw() >= 0) {
        (true, true) => first.max(second),
        (false, false) => first.min(second),
        _ => first + second,
    }
}

fn with_containing_insets(
    mut style: ComputedLayoutStyle,
    containing: ContainingBlock,
) -> ComputedLayoutStyle {
    style.block_indent_left += containing.inset_left;
    style.block_indent_right += containing.inset_right;
    style
}

fn push_text_block(
    chapter: &ChapterIr,
    node_id: NodeId,
    kind: BlockKind,
    text: &BlockText,
    context: LeafLayoutContext,
    blocks: &mut Vec<LayoutBlock>,
) {
    let Some(value) = chapter.text_pool.get(text.text) else {
        return;
    };
    let style = ComputedLayoutStyle::from_document(
        &chapter.styles,
        text.style,
        kind,
        context.depth,
        context.containing.inline_width,
        context.containing.inset_left,
        context.containing.inset_right,
    );
    let style_runs = compile_inline_style_runs(
        chapter,
        text,
        value,
        kind,
        context.depth,
        context.containing,
        &style,
    );
    blocks.push(LayoutBlock {
        node_id,
        kind,
        content: LayoutBlockContent::Text {
            text: Arc::from(value),
            marker: context.marker,
            style_runs,
        },
        style,
        source_range: chapter.source_map.get(node_id),
    });
}

fn compile_inline_style_runs(
    chapter: &ChapterIr,
    block: &BlockText,
    text: &str,
    kind: BlockKind,
    depth: u32,
    containing: ContainingBlock,
    base_style: &ComputedLayoutStyle,
) -> Vec<TextStyleRun> {
    if block.style_runs.is_empty() {
        return Vec::new();
    }
    let text_end = u32::try_from(text.len()).unwrap_or(u32::MAX);
    let mut source_runs = block.style_runs.iter().collect::<Vec<_>>();
    source_runs.sort_by_key(|run| (run.start, run.end));
    let mut compiled = Vec::with_capacity(source_runs.len().saturating_add(2));
    let mut cursor = 0_u32;
    for run in source_runs {
        let start = run.start.max(cursor).min(text_end);
        let end = run.end.min(text_end);
        let start_index = usize::try_from(start).unwrap_or(usize::MAX);
        let end_index = usize::try_from(end).unwrap_or(usize::MAX);
        if start >= end || !text.is_char_boundary(start_index) || !text.is_char_boundary(end_index)
        {
            continue;
        }
        if cursor < start {
            push_compiled_text_run(&mut compiled, text_style_run(cursor, start, base_style));
        }
        let inline_style = ComputedLayoutStyle::from_document(
            &chapter.styles,
            run.style,
            kind,
            depth,
            containing.inline_width,
            containing.inset_left,
            containing.inset_right,
        );
        push_compiled_text_run(&mut compiled, text_style_run(start, end, &inline_style));
        cursor = end;
    }
    if cursor < text_end {
        push_compiled_text_run(&mut compiled, text_style_run(cursor, text_end, base_style));
    }
    compiled
}

fn text_style_run(start: u32, end: u32, style: &ComputedLayoutStyle) -> TextStyleRun {
    TextStyleRun {
        start,
        end,
        font_size: style.font_size,
        letter_spacing: style.letter_spacing,
        fonts: style.font_candidates.clone(),
    }
}

fn push_compiled_text_run(runs: &mut Vec<TextStyleRun>, run: TextStyleRun) {
    if let Some(previous) = runs.last_mut() {
        if previous.end == run.start
            && previous.font_size == run.font_size
            && previous.letter_spacing == run.letter_spacing
            && previous.fonts == run.fonts
        {
            previous.end = run.end;
            return;
        }
    }
    runs.push(run);
}

fn layout_placeholder(
    chapter: &ChapterIr,
    node_id: NodeId,
    kind: BlockKind,
    style_id: crate::core::StyleId,
    label: &str,
    depth: u32,
    containing: ContainingBlock,
) -> LayoutBlock {
    LayoutBlock {
        node_id,
        kind,
        content: LayoutBlockContent::Unsupported(Arc::from(label)),
        style: ComputedLayoutStyle::from_document(
            &chapter.styles,
            style_id,
            kind,
            depth,
            containing.inline_width,
            containing.inset_left,
            containing.inset_right,
        ),
        source_range: chapter.source_map.get(node_id),
    }
}

fn layout_image(
    chapter: &ChapterIr,
    node_id: NodeId,
    image: &ImageNode,
    depth: u32,
    containing: ContainingBlock,
) -> LayoutBlock {
    let document_style = chapter.styles.get(image.style);
    let mut style = ComputedLayoutStyle::from_document(
        &chapter.styles,
        image.style,
        BlockKind::Image,
        depth,
        containing.inline_width,
        containing.inset_left,
        containing.inset_right,
    );
    if image.layout_role != ImageLayoutRole::Inline
        && document_style.is_none_or(|style| !has_explicit_margin(style))
    {
        style.margin_top = LayoutUnit::ZERO;
        style.margin_right = LayoutUnit::ZERO;
        style.margin_bottom = LayoutUnit::ZERO;
        style.margin_left = LayoutUnit::ZERO;
    }
    LayoutBlock {
        node_id,
        kind: BlockKind::Image,
        content: LayoutBlockContent::Image(LayoutImage {
            alt: image.alt.clone(),
            intrinsic_width: image
                .intrinsic_size
                .map(|size| LayoutUnit::from_px(i64::from(size.width))),
            intrinsic_height: image
                .intrinsic_size
                .map(|size| LayoutUnit::from_px(i64::from(size.height))),
            layout_role: image.layout_role,
            width: image_style_property(document_style, "width", image.layout_role),
            height: image_style_property(document_style, "height", image.layout_role),
            max_width: image_style_property(document_style, "max-width", image.layout_role),
            max_height: image_style_property(document_style, "max-height", image.layout_role),
        }),
        style,
        source_range: chapter.source_map.get(node_id),
    }
}

fn image_style_property(
    style: Option<&DocumentComputedStyle>,
    property: &str,
    role: ImageLayoutRole,
) -> Option<Arc<str>> {
    let value = style
        .and_then(|style| style.properties.get(property))
        .filter(|value| !value.trim().eq_ignore_ascii_case("auto"))
        .cloned()?;
    if matches!(role, ImageLayoutRole::Cover | ImageLayoutRole::Standalone)
        && matches!(property, "height" | "max-height")
        && value
            .trim()
            .strip_suffix('%')
            .and_then(parse_number)
            .is_some()
    {
        return None;
    }
    Some(value)
}

fn has_explicit_margin(style: &DocumentComputedStyle) -> bool {
    [
        "margin",
        "margin-top",
        "margin-right",
        "margin-bottom",
        "margin-left",
    ]
    .iter()
    .any(|property| style.properties.contains_key(*property))
}

fn resolve_image_size(
    image: &LayoutImage,
    style: &ComputedLayoutStyle,
    available_width: LayoutUnit,
    available_height: LayoutUnit,
) -> (LayoutUnit, LayoutUnit) {
    const INLINE_IMAGE_MAX_WIDTH_PX: i64 = 280;
    const FALLBACK_IMAGE_WIDTH_PX: i64 = 180;
    const FALLBACK_IMAGE_HEIGHT_PX: i64 = 140;

    let intrinsic_width = image
        .intrinsic_width
        .unwrap_or(LayoutUnit::from_px(FALLBACK_IMAGE_WIDTH_PX));
    let intrinsic_height = image
        .intrinsic_height
        .unwrap_or(LayoutUnit::from_px(FALLBACK_IMAGE_HEIGHT_PX));
    let authored_width = image
        .width
        .as_deref()
        .and_then(|value| resolve_used_length(value, style.font_size, available_width, false));
    let authored_height = image
        .height
        .as_deref()
        .and_then(|value| resolve_used_length(value, style.font_size, available_height, false));
    let authored_max_width = image
        .max_width
        .as_deref()
        .and_then(|value| resolve_used_length(value, style.font_size, available_width, false));
    let authored_max_height = image
        .max_height
        .as_deref()
        .and_then(|value| resolve_used_length(value, style.font_size, available_height, false));

    let role_max_width = match image.layout_role {
        ImageLayoutRole::Inline => {
            available_width.min(LayoutUnit::from_px(INLINE_IMAGE_MAX_WIDTH_PX))
        }
        ImageLayoutRole::Cover | ImageLayoutRole::Standalone => available_width,
    };
    let max_width = authored_max_width
        .map_or(role_max_width, |value| value.min(role_max_width))
        .max(LayoutUnit::from_px(1));
    let max_height = authored_max_height
        .map_or(available_height, |value| value.min(available_height))
        .max(LayoutUnit::from_px(1));

    let (mut width, mut height) = match (authored_width, authored_height) {
        (Some(width), Some(height)) => (width, height),
        (Some(width), None) => (
            width,
            scale_dimension(intrinsic_height, width, intrinsic_width),
        ),
        (None, Some(height)) => (
            scale_dimension(intrinsic_width, height, intrinsic_height),
            height,
        ),
        (None, None) => (intrinsic_width, intrinsic_height),
    };

    let preserve_aspect = authored_width.is_none() || authored_height.is_none();
    if preserve_aspect {
        let allow_upscale = authored_width.is_none()
            && authored_height.is_none()
            && matches!(
                image.layout_role,
                ImageLayoutRole::Cover | ImageLayoutRole::Standalone
            );
        let constrain_height = image.intrinsic_width.is_some()
            || image.intrinsic_height.is_some()
            || image.has_authored_size();
        (width, height) = fit_preserving_aspect(
            width,
            height,
            max_width,
            constrain_height.then_some(max_height),
            allow_upscale,
        );
    } else {
        width = width.min(max_width).max(LayoutUnit::from_px(1));
        height = height.min(max_height).max(LayoutUnit::from_px(1));
    }
    (width, height)
}

fn scale_dimension(value: LayoutUnit, target: LayoutUnit, original: LayoutUnit) -> LayoutUnit {
    if original.raw() <= 0 {
        return LayoutUnit::from_px(1);
    }
    LayoutUnit::from_f64_px(value.to_f64_px() * target.to_f64_px() / original.to_f64_px())
        .max(LayoutUnit::from_px(1))
}

fn fit_preserving_aspect(
    width: LayoutUnit,
    height: LayoutUnit,
    max_width: LayoutUnit,
    max_height: Option<LayoutUnit>,
    allow_upscale: bool,
) -> (LayoutUnit, LayoutUnit) {
    if width.raw() <= 0 || height.raw() <= 0 {
        return (LayoutUnit::from_px(1), LayoutUnit::from_px(1));
    }
    let width_scale = max_width.to_f64_px() / width.to_f64_px();
    let height_scale = max_height.map_or(f64::INFINITY, |max_height| {
        max_height.to_f64_px() / height.to_f64_px()
    });
    let mut scale = width_scale.min(height_scale);
    if !allow_upscale {
        scale = scale.min(1.0);
    }
    scale = scale.max(0.0);
    (
        LayoutUnit::from_f64_px(width.to_f64_px() * scale).max(LayoutUnit::from_px(1)),
        LayoutUnit::from_f64_px(height.to_f64_px() * scale).max(LayoutUnit::from_px(1)),
    )
}

fn measure_text(
    text_backend: &dyn TextBackend,
    cancel: &CancellationToken,
    request: &MeasureRequest,
    style: &ComputedLayoutStyle,
) -> Result<MeasuredText, PageletError> {
    let batch = text_backend.measure_batch(&MeasureBatch::new(vec![request.clone()]), cancel)?;
    let mut measured = batch
        .get(request.id)
        .cloned()
        .ok_or_else(|| layout_error("text backend did not return requested measurement"))?;
    if measured.lines.is_empty() {
        measured = synthesize_lines(request, style);
    }
    Ok(measured)
}

fn measure_request(
    request_id: u32,
    paragraph_id: u32,
    text: &str,
    style: &ComputedLayoutStyle,
    style_runs: &[TextStyleRun],
    width: LayoutUnit,
) -> MeasureRequest {
    let mut request = MeasureRequest::new(request_id, text, style.font_size, width);
    let text_end = u32::try_from(text.len()).unwrap_or(u32::MAX);
    request.paragraph_id = paragraph_id;
    request.style_runs = if style_runs.is_empty() {
        vec![text_style_run(0, text_end, style)]
    } else {
        style_runs.to_vec()
    };
    request.locale = style.locale.clone();
    request.direction = style.direction;
    request.font_candidates = style.font_candidates.clone();
    request.height_behavior = HeightBehavior::IncludeStrut;
    request.strut = line_height_strut(style);
    request.request_fingerprint = measure_request_fingerprint(&request);
    request
}

fn line_height_strut(style: &ComputedLayoutStyle) -> StrutStyle {
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

fn scene_paragraph(
    request: &MeasureRequest,
    measured: &MeasuredText,
    style: &ComputedLayoutStyle,
    constraints: LayoutConstraints,
) -> SceneParagraph {
    let mut line_y = LayoutUnit::ZERO;
    let lines = measured
        .lines
        .iter()
        .enumerate()
        .map(|(index, metrics)| {
            let is_first_line = index == 0;
            let base_x = if is_first_line {
                style.text_indent
            } else {
                LayoutUnit::ZERO
            };
            let available_width = if is_first_line {
                first_line_available_width(constraints, style)
            } else {
                block_available_width(constraints, style)
            };
            let line_x = aligned_x(base_x, available_width, metrics.width, style.alignment);
            let layout_rect = Rect {
                x: line_x,
                y: line_y,
                width: metrics.width,
                height: metrics.line_height,
            };
            let ink_bounds = Rect {
                x: line_x + metrics.ink_bounds.x,
                y: line_y + metrics.ink_bounds.y,
                width: metrics.ink_bounds.width,
                height: metrics.ink_bounds.height,
            };
            line_y += metrics.line_height;
            SceneParagraphLine {
                metrics: *metrics,
                layout_rect,
                ink_bounds,
            }
        })
        .collect();
    SceneParagraph {
        paragraph_id: request.paragraph_id,
        request_fingerprint: request.request_fingerprint,
        measurement_fingerprint: measured.measurement_fingerprint,
        text: request.text.clone(),
        text_range: request.text_range.clone(),
        style_runs: request.style_runs.clone(),
        font_size: request.font_size,
        available_width: request.available_width,
        max_width: request.max_width,
        locale: request.locale.clone(),
        direction: request.direction,
        text_scale: request.text_scale,
        font_candidates: request.font_candidates.clone(),
        strut: request.strut,
        height_behavior: request.height_behavior,
        lines,
        clusters: measured.clusters.clone(),
    }
}

fn synthesize_lines(request: &MeasureRequest, style: &ComputedLayoutStyle) -> MeasuredText {
    let text = &request.text;
    let width = request.available_width;
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
        request.id,
        request.request_fingerprint,
        width,
        height,
        u32::try_from(text.len()).unwrap_or(u32::MAX),
        lines,
        clusters,
        request.id as u64,
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
        ink_bounds: TextBounds {
            x: LayoutUnit::ZERO,
            y: LayoutUnit::ZERO,
            width,
            height: line_height,
        },
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

fn block_available_width(
    constraints: LayoutConstraints,
    style: &ComputedLayoutStyle,
) -> LayoutUnit {
    let width = constraints.content_width()
        - style.block_indent_left
        - style.margin_left
        - style.padding_left
        - style.padding_right
        - style.margin_right
        - style.block_indent_right;
    if width.raw() <= 0 {
        LayoutUnit::from_px(1)
    } else {
        width
    }
}

fn measurement_available_width(
    constraints: LayoutConstraints,
    style: &ComputedLayoutStyle,
) -> LayoutUnit {
    let width = block_available_width(constraints, style) - style.text_indent.max(LayoutUnit::ZERO);
    if width.raw() <= 0 {
        LayoutUnit::from_px(1)
    } else {
        width
    }
}

fn first_line_available_width(
    constraints: LayoutConstraints,
    style: &ComputedLayoutStyle,
) -> LayoutUnit {
    measurement_available_width(constraints, style)
}

fn block_x(constraints: LayoutConstraints, style: &ComputedLayoutStyle) -> LayoutUnit {
    constraints.margin_start + style.block_indent_left + style.margin_left + style.padding_left
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

fn link_regions(
    chapter: &ChapterIr,
    text_paints: &[TextPaintFragment],
    paragraphs: &[SceneParagraph],
    fragments: &[SceneFragment],
) -> Vec<LinkRegion> {
    let mut regions = Vec::new();
    for link in &chapter.links {
        for paint in text_paints
            .iter()
            .filter(|paint| paint.node_id == link.source_node)
        {
            if let Some(text_range) = &link.text_range {
                if let Some(paragraph) = paragraphs
                    .iter()
                    .find(|paragraph| paragraph.paragraph_id == paint.paragraph_id)
                {
                    regions.extend(
                        text_range_rects(paragraph, paint, text_range.clone())
                            .into_iter()
                            .map(|rect| link_region(link, rect)),
                    );
                } else {
                    regions.push(link_region(link, paint.layout_rect));
                }
            } else {
                regions.push(link_region(link, paint.layout_rect));
            }
        }
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
        text_range: link.text_range.clone(),
        href: link.href.clone(),
        resolved_document: link.resolved_document.clone(),
        fragment: link.fragment.clone(),
        kind: link.kind,
    }
}

fn anchor_regions(
    chapter: &ChapterIr,
    text_paints: &[TextPaintFragment],
    paragraphs: &[SceneParagraph],
    fragments: &[SceneFragment],
) -> Vec<AnchorRegion> {
    let mut regions = Vec::new();
    for anchor in chapter.anchors.anchors.values() {
        let mut text_region_added = false;
        for paint in text_paints
            .iter()
            .filter(|paint| paint.node_id == anchor.node_id)
        {
            if let Some(paragraph) = paragraphs
                .iter()
                .find(|paragraph| paragraph.paragraph_id == paint.paragraph_id)
            {
                if let Some(rect) = anchor_caret_rect(paragraph, paint, anchor.utf8_byte_offset) {
                    regions.push(AnchorRegion {
                        rect,
                        key: anchor.key.clone(),
                        node_id: anchor.node_id,
                    });
                    text_region_added = true;
                }
            } else {
                regions.push(AnchorRegion {
                    rect: paint.layout_rect,
                    key: anchor.key.clone(),
                    node_id: anchor.node_id,
                });
                text_region_added = true;
            }
        }
        if text_region_added {
            continue;
        }
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

fn text_range_rects(
    paragraph: &SceneParagraph,
    paint: &TextPaintFragment,
    range: std::ops::Range<u32>,
) -> Vec<Rect> {
    let start = range.start.max(paint.visible_text_range.start);
    let end = range.end.min(paint.visible_text_range.end);
    if start >= end {
        return Vec::new();
    }
    let first_line = paint.first_line;
    let line_end = first_line.saturating_add(paint.line_count);
    let mut rects = Vec::new();
    let mut current: Option<(u32, Rect)> = None;
    for cluster in paragraph.clusters.iter().filter(|cluster| {
        cluster.text_end > start
            && cluster.text_start < end
            && cluster.line_index >= first_line
            && cluster.line_index < line_end
    }) {
        let Some(line) = paragraph
            .lines
            .get(usize::try_from(cluster.line_index).unwrap_or(usize::MAX))
        else {
            continue;
        };
        let cluster_rect = Rect {
            x: paint.paint_origin.x + line.layout_rect.x + cluster.x_start,
            y: paint.paint_origin.y + line.layout_rect.y,
            width: cluster.x_end - cluster.x_start,
            height: line.layout_rect.height,
        };
        if let Some((line_index, rect)) = &mut current {
            if *line_index == cluster.line_index {
                let right = (rect.x + rect.width).max(cluster_rect.x + cluster_rect.width);
                rect.x = rect.x.min(cluster_rect.x);
                rect.width = right - rect.x;
                continue;
            }
        }
        if let Some((_, rect)) = current.take() {
            if let Some(clipped) = intersect_rect(rect, paint.clip_rect) {
                rects.push(clipped);
            }
        }
        current = Some((cluster.line_index, cluster_rect));
    }
    if let Some((_, rect)) = current {
        if let Some(clipped) = intersect_rect(rect, paint.clip_rect) {
            rects.push(clipped);
        }
    }
    rects
}

fn anchor_caret_rect(
    paragraph: &SceneParagraph,
    paint: &TextPaintFragment,
    offset: u32,
) -> Option<Rect> {
    let is_paragraph_end = offset == paragraph.text_range.end;
    if offset < paint.visible_text_range.start
        || offset > paint.visible_text_range.end
        || (offset == paint.visible_text_range.end && !is_paragraph_end)
    {
        return None;
    }
    let first_line = paint.first_line;
    let line_end = first_line.saturating_add(paint.line_count);
    let cluster =
        paragraph
            .clusters
            .iter()
            .filter(|cluster| cluster.line_index >= first_line && cluster.line_index < line_end)
            .find(|cluster| cluster.text_end > offset)
            .or_else(|| {
                paragraph.clusters.iter().rev().find(|cluster| {
                    cluster.line_index >= first_line && cluster.line_index < line_end
                })
            })?;
    let line = paragraph
        .lines
        .get(usize::try_from(cluster.line_index).unwrap_or(usize::MAX))?;
    let x = if offset >= cluster.text_end {
        cluster.x_end
    } else {
        cluster.x_start
    };
    intersect_rect(
        Rect {
            x: paint.paint_origin.x + line.layout_rect.x + x,
            y: paint.paint_origin.y + line.layout_rect.y,
            width: LayoutUnit::from_px(1),
            height: line.layout_rect.height,
        },
        paint.clip_rect,
    )
}

fn intersect_rect(first: Rect, second: Rect) -> Option<Rect> {
    let left = first.x.max(second.x);
    let top = first.y.max(second.y);
    let right = (first.x + first.width).min(second.x + second.width);
    let bottom = (first.y + first.height).min(second.y + second.height);
    (right > left && bottom > top).then_some(Rect {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    })
}

fn selection_maps(
    text_paints: &[TextPaintFragment],
    paragraphs: &[SceneParagraph],
    fragments: &[SceneFragment],
) -> Vec<SelectionMap> {
    let mut maps = Vec::new();
    for paint in text_paints {
        let Some(paragraph) = paragraphs
            .iter()
            .find(|paragraph| paragraph.paragraph_id == paint.paragraph_id)
        else {
            continue;
        };
        let first = usize::try_from(paint.first_line).unwrap_or(usize::MAX);
        let end = first
            .saturating_add(usize::try_from(paint.line_count).unwrap_or(usize::MAX))
            .min(paragraph.lines.len());
        let rects = paragraph
            .lines
            .get(first..end)
            .map_or_else(Vec::new, |lines| {
                lines
                    .iter()
                    .map(|line| translate_rect(line.layout_rect, paint.paint_origin))
                    .collect()
            });
        maps.push(SelectionMap {
            node_id: paint.node_id,
            start: paint.visible_text_range.start,
            end: paint.visible_text_range.end,
            rects,
        });
    }
    maps.extend(fragments.iter().filter_map(|fragment| {
        let range = fragment.anchor_range?;
        Some(SelectionMap {
            node_id: fragment.node_id,
            start: range.start.utf8_byte_offset,
            end: range.end.utf8_byte_offset,
            rects: vec![fragment.rect],
        })
    }));
    maps
}

fn semantic_nodes(
    text_paints: &[TextPaintFragment],
    paragraphs: &[SceneParagraph],
    fragments: &[SceneFragment],
) -> Vec<SemanticNode> {
    let mut nodes = text_paints
        .iter()
        .filter_map(|paint| {
            let paragraph = paragraphs
                .iter()
                .find(|paragraph| paragraph.paragraph_id == paint.paragraph_id)?;
            Some(SemanticNode {
                node_id: paint.node_id,
                rect: paint.layout_rect,
                role: Arc::from("text"),
                label: Arc::from(slice_text(
                    &paragraph.text,
                    paint.visible_text_range.start,
                    paint.visible_text_range.end,
                )),
            })
        })
        .collect::<Vec<_>>();
    nodes.extend(fragments.iter().map(|fragment| SemanticNode {
        node_id: fragment.node_id,
        rect: fragment.rect,
        role: Arc::from(match fragment.kind {
            SceneFragmentKind::TextLine => "text",
            SceneFragmentKind::Marker => "marker",
            SceneFragmentKind::Image => "image",
            SceneFragmentKind::Divider => "separator",
            SceneFragmentKind::UnsupportedPlaceholder => "note",
            SceneFragmentKind::BackgroundBorder | SceneFragmentKind::DebugOverlay => "presentation",
        }),
        label: fragment.text.clone().unwrap_or_else(|| Arc::from("")),
    }));
    nodes
}

fn translate_rect(rect: Rect, origin: Point) -> Rect {
    Rect {
        x: rect.x + origin.x,
        y: rect.y + origin.y,
        width: rect.width,
        height: rect.height,
    }
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
    for paragraph in &page.paragraphs {
        input.push_str(&format!(
            "|paragraph:{}:{}:{}",
            paragraph.paragraph_id,
            paragraph.request_fingerprint,
            paragraph.measurement_fingerprint
        ));
    }
    for paint in &page.text_paints {
        input.push_str(&format!(
            "|paint:{}:{}:{}-{}:{}:{}:{}:{}:{}:{}:{}:{}",
            paint.id,
            paint.paragraph_id,
            paint.visible_text_range.start,
            paint.visible_text_range.end,
            paint.paint_origin.x.raw(),
            paint.paint_origin.y.raw(),
            paint.clip_rect.x.raw(),
            paint.clip_rect.y.raw(),
            paint.clip_rect.width.raw(),
            paint.clip_rect.height.raw(),
            paint.first_line,
            paint.line_count,
        ));
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

fn measure_request_fingerprint(request: &MeasureRequest) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let direction = match request.direction {
        TextDirection::Auto => 0,
        TextDirection::Ltr => 1,
        TextDirection::Rtl => 2,
    };
    let height_behavior = match request.height_behavior {
        HeightBehavior::Natural => 0,
        HeightBehavior::IncludeStrut => 1,
        HeightBehavior::Tight => 2,
    };
    for value in [
        i64::from(request.paragraph_id),
        i64::from(request.text_range.start),
        i64::from(request.text_range.end),
        request.font_size.raw(),
        request.max_width.raw(),
        request.available_width.raw(),
        request.text_scale.raw(),
        request.strut.ascent.raw(),
        request.strut.descent.raw(),
        request.strut.leading.raw(),
        direction,
        height_behavior,
    ] {
        fingerprint_u64(&mut hash, value as u64);
    }
    fingerprint_bytes(&mut hash, request.text.as_bytes());
    fingerprint_bytes(&mut hash, request.locale.as_bytes());
    fingerprint_font_chain(&mut hash, &request.font_candidates);
    fingerprint_u64(
        &mut hash,
        u64::try_from(request.style_runs.len()).unwrap_or(u64::MAX),
    );
    for run in &request.style_runs {
        fingerprint_u64(&mut hash, u64::from(run.start));
        fingerprint_u64(&mut hash, u64::from(run.end));
        fingerprint_u64(&mut hash, run.font_size.raw() as u64);
        fingerprint_u64(&mut hash, run.letter_spacing.raw() as u64);
        fingerprint_font_chain(&mut hash, &run.fonts);
    }
    hash
}

fn fingerprint_font_chain(hash: &mut u64, chain: &FontFallbackChain) {
    fingerprint_font_descriptor(hash, &chain.primary);
    fingerprint_u64(
        hash,
        u64::try_from(chain.fallbacks.len()).unwrap_or(u64::MAX),
    );
    for descriptor in &chain.fallbacks {
        fingerprint_font_descriptor(hash, descriptor);
    }
}

fn fingerprint_font_descriptor(hash: &mut u64, descriptor: &FontDescriptor) {
    fingerprint_u64(
        hash,
        descriptor.font_id.map_or(0, |id| u64::from(id.get()) + 1),
    );
    fingerprint_bytes(hash, descriptor.family.as_bytes());
    fingerprint_u64(hash, u64::from(descriptor.weight));
    fingerprint_u64(
        hash,
        match descriptor.style {
            FontStyle::Normal => 0,
            FontStyle::Italic => 1,
            FontStyle::Oblique => 2,
        },
    );
    fingerprint_u64(hash, u64::from(descriptor.stretch));
    fingerprint_u64(hash, descriptor.fingerprint.0);
}

fn fingerprint_bytes(hash: &mut u64, bytes: &[u8]) {
    fingerprint_u64(hash, u64::try_from(bytes.len()).unwrap_or(u64::MAX));
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
}

fn fingerprint_u64(hash: &mut u64, value: u64) {
    for byte in value.to_le_bytes() {
        *hash ^= u64::from(byte);
        *hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
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
    push_u32(
        out,
        level + 1,
        "scene_wire",
        EngineVersions::CURRENT.scene_wire,
        true,
    );
    push_u32(out, level + 1, "page_index", page.page_index, true);
    push_string(
        out,
        level + 1,
        "fingerprint",
        &page.fingerprint.to_hex(),
        true,
    );
    push_string(
        out,
        level + 1,
        "text_backend_id",
        &format!("{:016x}", page.text_backend_id.0),
        true,
    );
    push_string(
        out,
        level + 1,
        "font_fingerprint",
        &format!("{:016x}", page.font_fingerprint.0),
        true,
    );
    indent(out, level + 1);
    out.push_str("\"size\": {");
    push_inline_i64(out, "width", page.size.width.raw(), true);
    push_inline_i64(out, "height", page.size.height.raw(), false);
    out.push_str("},\n");
    indent(out, level + 1);
    out.push_str("\"paragraphs\": [\n");
    for (index, paragraph) in page.paragraphs.iter().enumerate() {
        push_paragraph_json(out, paragraph, level + 2, index + 1 < page.paragraphs.len());
    }
    indent(out, level + 1);
    out.push_str("],\n");
    indent(out, level + 1);
    out.push_str("\"text_paints\": [\n");
    for (index, paint) in page.text_paints.iter().enumerate() {
        push_text_paint_json(out, paint, level + 2, index + 1 < page.text_paints.len());
    }
    indent(out, level + 1);
    out.push_str("],\n");
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
        push_inline_u32(out, "node_id", link.node_id.get(), true);
        push_inline_string(out, "href", &link.href, true);
        push_inline_opt_string(
            out,
            "resolved_document",
            link.resolved_document.as_deref(),
            true,
        );
        push_inline_opt_string(out, "fragment", link.fragment.as_deref(), true);
        push_inline_string(out, "kind", link_kind_name(link.kind), true);
        push_inline_opt_text_range(out, "text_range", link.text_range.as_ref(), true);
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

fn push_paragraph_json(out: &mut String, paragraph: &SceneParagraph, level: usize, trailing: bool) {
    indent(out, level);
    out.push_str("{\n");
    push_u32(out, level + 1, "paragraph_id", paragraph.paragraph_id, true);
    push_string(
        out,
        level + 1,
        "request_fingerprint",
        &format!("{:016x}", paragraph.request_fingerprint),
        true,
    );
    push_string(
        out,
        level + 1,
        "measurement_fingerprint",
        &format!("{:016x}", paragraph.measurement_fingerprint),
        true,
    );
    push_string(out, level + 1, "text", &paragraph.text, true);
    indent(out, level + 1);
    out.push_str("\"text_range\": {");
    push_inline_u32(out, "start", paragraph.text_range.start, true);
    push_inline_u32(out, "end", paragraph.text_range.end, false);
    out.push_str("},\n");
    indent(out, level + 1);
    out.push_str("\"style_runs\": [\n");
    for (index, run) in paragraph.style_runs.iter().enumerate() {
        indent(out, level + 2);
        out.push('{');
        push_inline_u32(out, "start", run.start, true);
        push_inline_u32(out, "end", run.end, true);
        push_inline_i64(out, "font_size", run.font_size.raw(), true);
        push_inline_i64(out, "letter_spacing", run.letter_spacing.raw(), true);
        push_inline_font_chain(out, "fonts", &run.fonts);
        out.push('}');
        if index + 1 < paragraph.style_runs.len() {
            out.push(',');
        }
        out.push('\n');
    }
    indent(out, level + 1);
    out.push_str("],\n");
    push_i64(out, level + 1, "font_size", paragraph.font_size.raw(), true);
    push_i64(
        out,
        level + 1,
        "available_width",
        paragraph.available_width.raw(),
        true,
    );
    push_i64(out, level + 1, "max_width", paragraph.max_width.raw(), true);
    push_string(out, level + 1, "locale", &paragraph.locale, true);
    push_string(
        out,
        level + 1,
        "direction",
        text_direction_name(paragraph.direction),
        true,
    );
    push_i64(
        out,
        level + 1,
        "text_scale",
        paragraph.text_scale.raw(),
        true,
    );
    indent(out, level + 1);
    push_inline_font_chain(out, "font_candidates", &paragraph.font_candidates);
    out.push_str(",\n");
    indent(out, level + 1);
    out.push_str("\"strut\": {");
    push_inline_i64(out, "ascent", paragraph.strut.ascent.raw(), true);
    push_inline_i64(out, "descent", paragraph.strut.descent.raw(), true);
    push_inline_i64(out, "leading", paragraph.strut.leading.raw(), false);
    out.push_str("},\n");
    push_string(
        out,
        level + 1,
        "height_behavior",
        match paragraph.height_behavior {
            HeightBehavior::Natural => "natural",
            HeightBehavior::IncludeStrut => "include-strut",
            HeightBehavior::Tight => "tight",
        },
        true,
    );
    indent(out, level + 1);
    out.push_str("\"lines\": [\n");
    for (index, line) in paragraph.lines.iter().enumerate() {
        indent(out, level + 2);
        out.push('{');
        push_inline_u32(out, "text_start", line.metrics.text_start, true);
        push_inline_u32(out, "text_end", line.metrics.text_end, true);
        push_inline_i64(out, "baseline", line.metrics.baseline.raw(), true);
        push_inline_i64(out, "ascent", line.metrics.ascent.raw(), true);
        push_inline_i64(out, "descent", line.metrics.descent.raw(), true);
        push_inline_i64(out, "line_height", line.metrics.line_height.raw(), true);
        push_inline_i64(out, "width", line.metrics.width.raw(), true);
        push_inline_text_bounds(out, "host_ink_bounds", line.metrics.ink_bounds, true);
        push_inline_rect_with_trailing(out, "layout_rect", line.layout_rect, true);
        push_inline_rect_with_trailing(out, "ink_bounds", line.ink_bounds, true);
        out.push_str("\"hard_break\": ");
        out.push_str(if line.metrics.hard_break {
            "true"
        } else {
            "false"
        });
        out.push('}');
        if index + 1 < paragraph.lines.len() {
            out.push(',');
        }
        out.push('\n');
    }
    indent(out, level + 1);
    out.push_str("],\n");
    indent(out, level + 1);
    out.push_str("\"clusters\": [\n");
    for (index, cluster) in paragraph.clusters.iter().enumerate() {
        indent(out, level + 2);
        out.push('{');
        push_inline_u32(out, "text_start", cluster.text_start, true);
        push_inline_u32(out, "text_end", cluster.text_end, true);
        push_inline_u32(out, "line_index", cluster.line_index, true);
        push_inline_i64(out, "x_start", cluster.x_start.raw(), true);
        push_inline_i64(out, "x_end", cluster.x_end.raw(), false);
        out.push('}');
        if index + 1 < paragraph.clusters.len() {
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

fn push_text_paint_json(out: &mut String, paint: &TextPaintFragment, level: usize, trailing: bool) {
    indent(out, level);
    out.push('{');
    push_inline_u32(out, "id", paint.id, true);
    push_inline_u32(out, "node_id", paint.node_id.get(), true);
    push_inline_u32(out, "paragraph_id", paint.paragraph_id, true);
    out.push_str("\"visible_text_range\": {");
    push_inline_u32(out, "start", paint.visible_text_range.start, true);
    push_inline_u32(out, "end", paint.visible_text_range.end, false);
    out.push_str("}, ");
    out.push_str("\"paint_origin\": {");
    push_inline_i64(out, "x", paint.paint_origin.x.raw(), true);
    push_inline_i64(out, "y", paint.paint_origin.y.raw(), false);
    out.push_str("}, ");
    push_inline_rect_with_trailing(out, "layout_rect", paint.layout_rect, true);
    push_inline_rect_with_trailing(out, "clip_rect", paint.clip_rect, true);
    push_inline_u32(out, "first_line", paint.first_line, true);
    push_inline_u32(out, "line_count", paint.line_count, true);
    if let Some(source_range) = paint.source_range {
        out.push_str("\"source_range\": {");
        push_inline_u32(out, "start", source_range.start, true);
        push_inline_u32(out, "end", source_range.end, false);
        out.push_str("}, ");
    }
    out.push_str("\"anchor_range\": {");
    push_inline_text_anchor(out, "start", paint.anchor_range.start, true);
    push_inline_text_anchor(out, "end", paint.anchor_range.end, false);
    out.push_str("}, ");
    out.push_str("\"overflow\": ");
    out.push_str(if paint.overflow { "true" } else { "false" });
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

const fn link_kind_name(kind: LinkKind) -> &'static str {
    match kind {
        LinkKind::Internal => "internal",
        LinkKind::External => "external",
        LinkKind::Resource => "resource",
        LinkKind::Footnote => "footnote",
        LinkKind::Unknown => "unknown",
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

fn push_i64(out: &mut String, level: usize, name: &str, value: i64, trailing: bool) {
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

fn push_inline_opt_string(out: &mut String, name: &str, value: Option<&str>, trailing: bool) {
    out.push('"');
    out.push_str(name);
    out.push_str("\": ");
    if let Some(value) = value {
        out.push('"');
        out.push_str(&escape_json(value));
        out.push('"');
    } else {
        out.push_str("null");
    }
    if trailing {
        out.push_str(", ");
    }
}

fn push_inline_opt_text_range(
    out: &mut String,
    name: &str,
    range: Option<&std::ops::Range<u32>>,
    trailing: bool,
) {
    out.push('"');
    out.push_str(name);
    out.push_str("\": ");
    if let Some(range) = range {
        out.push('{');
        push_inline_u32(out, "start", range.start, true);
        push_inline_u32(out, "end", range.end, false);
        out.push('}');
    } else {
        out.push_str("null");
    }
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

fn push_inline_rect_with_trailing(out: &mut String, name: &str, rect: Rect, trailing: bool) {
    push_inline_rect(out, name, rect);
    if trailing {
        out.push_str(", ");
    }
}

fn push_inline_text_bounds(out: &mut String, name: &str, bounds: TextBounds, trailing: bool) {
    out.push('"');
    out.push_str(name);
    out.push_str("\": {");
    push_inline_i64(out, "x", bounds.x.raw(), true);
    push_inline_i64(out, "y", bounds.y.raw(), true);
    push_inline_i64(out, "width", bounds.width.raw(), true);
    push_inline_i64(out, "height", bounds.height.raw(), false);
    out.push('}');
    if trailing {
        out.push_str(", ");
    }
}

fn push_inline_text_anchor(out: &mut String, name: &str, anchor: TextAnchor, trailing: bool) {
    out.push('"');
    out.push_str(name);
    out.push_str("\": {");
    push_inline_u32(out, "document_id", anchor.document_id.get(), true);
    push_inline_u32(out, "node_id", anchor.node_id.get(), true);
    push_inline_u32(out, "utf8_byte_offset", anchor.utf8_byte_offset, true);
    push_inline_string(
        out,
        "affinity",
        match anchor.affinity {
            TextAffinity::Upstream => "upstream",
            TextAffinity::Downstream => "downstream",
        },
        false,
    );
    out.push('}');
    if trailing {
        out.push_str(", ");
    }
}

fn push_inline_font_chain(out: &mut String, name: &str, chain: &FontFallbackChain) {
    out.push('"');
    out.push_str(name);
    out.push_str("\": {");
    out.push_str("\"primary\": ");
    push_inline_font_descriptor(out, &chain.primary);
    out.push_str(", \"fallbacks\": [");
    for (index, descriptor) in chain.fallbacks.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        push_inline_font_descriptor(out, descriptor);
    }
    out.push_str("]}");
}

fn push_inline_font_descriptor(out: &mut String, descriptor: &FontDescriptor) {
    out.push('{');
    out.push_str("\"font_id\": ");
    if let Some(font_id) = descriptor.font_id {
        out.push_str(&font_id.get().to_string());
    } else {
        out.push_str("null");
    }
    out.push_str(", ");
    push_inline_string(out, "family", &descriptor.family, true);
    push_inline_u32(out, "weight", u32::from(descriptor.weight), true);
    push_inline_string(
        out,
        "style",
        match descriptor.style {
            FontStyle::Normal => "normal",
            FontStyle::Italic => "italic",
            FontStyle::Oblique => "oblique",
        },
        true,
    );
    push_inline_u32(out, "stretch", u32::from(descriptor.stretch), true);
    push_inline_string(
        out,
        "fingerprint",
        &format!("{:016x}", descriptor.fingerprint.0),
        false,
    );
    out.push('}');
}

const fn text_direction_name(direction: TextDirection) -> &'static str {
    match direction {
        TextDirection::Auto => "auto",
        TextDirection::Ltr => "ltr",
        TextDirection::Rtl => "rtl",
    }
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
    for paint in &page.text_paints {
        out.push_str(&format!(
            r##"<rect x="{:.2}" y="{:.2}" width="{:.2}" height="{:.2}" fill="none" stroke="#1d4ed8" stroke-width="1"/>"##,
            paint.layout_rect.x.to_f64_px(),
            paint.layout_rect.y.to_f64_px(),
            paint.layout_rect.width.to_f64_px(),
            paint.layout_rect.height.to_f64_px(),
        ));
        if let Some(paragraph) = page
            .paragraphs
            .iter()
            .find(|paragraph| paragraph.paragraph_id == paint.paragraph_id)
        {
            let first = usize::try_from(paint.first_line).unwrap_or(usize::MAX);
            let end = first
                .saturating_add(usize::try_from(paint.line_count).unwrap_or(usize::MAX))
                .min(paragraph.lines.len());
            if let Some(lines) = paragraph.lines.get(first..end) {
                for line in lines {
                    let ink = translate_rect(line.ink_bounds, paint.paint_origin);
                    out.push_str(&format!(
                        r##"<rect x="{:.2}" y="{:.2}" width="{:.2}" height="{:.2}" fill="none" stroke="#dc2626" stroke-width="0.5"/>"##,
                        ink.x.to_f64_px(),
                        ink.y.to_f64_px(),
                        ink.width.to_f64_px(),
                        ink.height.to_f64_px(),
                    ));
                }
            }
        }
    }
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

    fn visible_text_line_rects(page: &PageScene) -> Vec<Rect> {
        let mut rects = Vec::new();
        for paint in &page.text_paints {
            let paragraph = page
                .paragraphs
                .iter()
                .find(|paragraph| paragraph.paragraph_id == paint.paragraph_id)
                .expect("paint paragraph");
            let first = usize::try_from(paint.first_line).expect("first line");
            let end = first + usize::try_from(paint.line_count).expect("line count");
            rects.extend(
                paragraph.lines[first..end]
                    .iter()
                    .map(|line| translate_rect(line.layout_rect, paint.paint_origin)),
            );
        }
        rects
    }

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
    fn css_font_request_is_replayed_with_host_baseline_and_ink_bounds() {
        let mut chapter = chapter_with_paragraph("One Two");
        let style = document::ComputedStyle::new()
            .with_property("font-family", "'Times New Roman', Georgia, serif")
            .with_property("font-size", "20px")
            .with_property("font-weight", "700")
            .with_property("font-style", "italic")
            .with_property("font-stretch", "semi-condensed")
            .with_property("letter-spacing", "2px")
            .with_property("-pagelet-locale", "en-US")
            .with_property("direction", "ltr");
        let style_id = chapter.styles.intern(style).expect("font style");
        let paragraph_node = chapter
            .nodes
            .iter_with_ids()
            .find_map(|(node_id, node)| {
                matches!(node, DocumentNode::Paragraph(_)).then_some(node_id)
            })
            .expect("paragraph");
        if let Some(DocumentNode::Paragraph(block)) = chapter.nodes.get_mut(paragraph_node) {
            block.style = style_id;
        }

        let prepared = HostMeasuredLayout::prepare(chapter, LayoutOptions::default());
        let request = prepared.measure_batch().requests[0].clone();
        assert_eq!(
            request.font_candidates.primary.family.as_ref(),
            "Times New Roman"
        );
        assert_eq!(request.font_candidates.primary.weight, 700);
        assert_eq!(request.font_candidates.primary.style, FontStyle::Italic);
        assert_eq!(request.font_candidates.primary.stretch, 87);
        assert_eq!(request.font_candidates.fallbacks.len(), 2);
        assert_eq!(request.locale.as_ref(), "en-US");
        assert_eq!(request.direction, TextDirection::Ltr);
        assert_eq!(request.style_runs.len(), 1);
        assert_eq!(request.style_runs[0].letter_spacing, LayoutUnit::from_px(2));

        let mut measured = DefaultTextBackend::new()
            .measure_batch(prepared.measure_batch(), &CancellationToken::new())
            .expect("measure Times paragraph");
        let host_baseline = LayoutUnit::from_raw(1_337);
        let host_ink_width = measured.results[0].lines[0].width + LayoutUnit::from_px(6);
        measured.results[0].lines[0].baseline = host_baseline;
        measured.results[0].lines[0].ink_bounds.x = LayoutUnit::from_px(-2);
        measured.results[0].lines[0].ink_bounds.width = host_ink_width;
        measured.results[0].measurement_fingerprint = 0xfeed_face;
        let pages = prepared
            .resume(MeasuredBatch::new(
                TextBackendId(0x6361_6e76_6173),
                FontSetFingerprint(0x7469_6d65_7301),
                measured.results,
            ))
            .expect("resume host paragraph");
        let page = &pages.pages[0];
        let paragraph = &page.paragraphs[0];
        let paint = &page.text_paints[0];

        assert_eq!(page.text_backend_id, TextBackendId(0x6361_6e76_6173));
        assert_eq!(page.font_fingerprint, FontSetFingerprint(0x7469_6d65_7301));
        assert_eq!(paragraph.request_fingerprint, request.request_fingerprint);
        assert_eq!(paragraph.measurement_fingerprint, 0xfeed_face);
        assert_eq!(paragraph.text, request.text);
        assert_eq!(paragraph.style_runs, request.style_runs);
        assert_eq!(paragraph.font_candidates, request.font_candidates);
        assert_eq!(paragraph.locale, request.locale);
        assert_eq!(paragraph.direction, request.direction);
        assert_eq!(paragraph.strut, request.strut);
        assert_eq!(paragraph.lines[0].metrics.baseline, host_baseline);
        assert_eq!(paragraph.lines[0].metrics.ink_bounds.width, host_ink_width);
        assert!(paragraph.lines[0].ink_bounds.width > paragraph.lines[0].layout_rect.width);
        assert_eq!(
            paint.clip_rect.width,
            LayoutOptions::default().constraints.content_width()
        );
        assert_ne!(paint.clip_rect.width, paragraph.lines[0].layout_rect.width);
    }

    #[test]
    fn paragraph_replay_keeps_one_identity_across_pages() {
        let text = "one two three four five six seven eight nine ten eleven twelve thirteen";
        let chapter = chapter_with_paragraph(text);
        let constraints = LayoutConstraints::new(LayoutUnit::from_px(90), LayoutUnit::from_px(72))
            .with_margin(LayoutUnit::from_px(8));
        let pages = paginate_chapter_with_options(
            &chapter,
            &DefaultTextBackend::new(),
            LayoutOptions {
                constraints,
                max_pages: 32,
                ..LayoutOptions::default()
            },
        )
        .expect("paginate replay paragraph");

        assert!(pages.pages.len() > 1);
        let first = &pages.pages[0];
        let second = &pages.pages[1];
        let paragraph_id = first.paragraphs[0].paragraph_id;
        let request_fingerprint = first.paragraphs[0].request_fingerprint;
        let measurement_fingerprint = first.paragraphs[0].measurement_fingerprint;
        for page in &pages.pages {
            assert_eq!(page.paragraphs.len(), 1);
            assert_eq!(page.paragraphs[0].paragraph_id, paragraph_id);
            assert_eq!(page.paragraphs[0].request_fingerprint, request_fingerprint);
            assert_eq!(
                page.paragraphs[0].measurement_fingerprint,
                measurement_fingerprint
            );
            assert_eq!(page.paragraphs[0].text.as_ref(), text);
            assert_eq!(page.paragraphs[0].text_range, 0..text.len() as u32);
        }
        assert_eq!(first.text_paints[0].first_line, 0);
        assert!(second.text_paints[0].first_line > 0);
        assert!(second.text_paints[0].paint_origin.y < second.text_paints[0].layout_rect.y);
        assert_eq!(
            second.text_paints[0].clip_rect,
            Rect {
                x: constraints.margin_start,
                y: constraints.margin_top,
                width: constraints.content_width(),
                height: constraints.content_height(),
            }
        );
        assert!(first
            .fragments
            .iter()
            .all(|fragment| { fragment.kind != SceneFragmentKind::TextLine }));
        assert!(second
            .fragments
            .iter()
            .all(|fragment| { fragment.kind != SceneFragmentKind::TextLine }));
    }

    #[test]
    fn cached_scene_rejects_every_text_replay_identity_mismatch() {
        let chapter = chapter_with_paragraph("identity");
        let pages = paginate_chapter(
            &chapter,
            &DefaultTextBackend::new(),
            LayoutConstraints::default(),
        )
        .expect("paginate identity");
        let page = &pages.pages[0];
        let identity = page.text_replay_identity();
        page.validate_text_replay_identity(&identity)
            .expect("matching identity");

        let mut stale = identity.clone();
        stale.text_backend_id.0 ^= 1;
        assert!(page.validate_text_replay_identity(&stale).is_err());
        let mut stale = identity.clone();
        stale.font_fingerprint.0 ^= 1;
        assert!(page.validate_text_replay_identity(&stale).is_err());
        let mut stale = identity.clone();
        stale.paragraphs[0].request_fingerprint ^= 1;
        assert!(page.validate_text_replay_identity(&stale).is_err());
        let mut stale = identity;
        stale.paragraphs[0].measurement_fingerprint ^= 1;
        assert!(page.validate_text_replay_identity(&stale).is_err());
    }

    #[test]
    fn request_fingerprint_covers_font_bytes_scale_and_strut() {
        let chapter = chapter_with_paragraph("fingerprint");
        let mut request =
            prepare_measure_batch(&chapter, LayoutOptions::default()).requests[0].clone();
        let original = request.request_fingerprint;

        request.font_candidates.primary.fingerprint.0 ^= 1;
        assert_ne!(measure_request_fingerprint(&request), original);
        request.font_candidates.primary.fingerprint.0 ^= 1;
        request.text_scale += LayoutUnit::from_raw(1);
        assert_ne!(measure_request_fingerprint(&request), original);
        request.text_scale -= LayoutUnit::from_raw(1);
        request.strut.leading += LayoutUnit::from_raw(1);
        assert_ne!(measure_request_fingerprint(&request), original);
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
        assert!(!pages.pages[0].text_paints.is_empty());
        assert!(!pages.pages[1].text_paints.is_empty());
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
        let text = visible_text_line_rects(&pages.pages[0])[0];

        assert!(marker.rect.x + marker.rect.width <= text.x);
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
    fn page_scene_applies_document_geometry_css() {
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
            .flat_map(visible_text_line_rects)
            .collect::<Vec<_>>();
        let styled_rects = styled_pages
            .pages
            .iter()
            .flat_map(visible_text_line_rects)
            .collect::<Vec<_>>();

        assert_ne!(styled_rects, plain_rects);
        assert_eq!(styled_rects[0].x, LayoutUnit::from_px(174));
        assert_eq!(styled_rects[0].y, LayoutUnit::from_px(144));
        assert!(styled_rects[0].width <= LayoutUnit::from_px(92));

        let batch = prepare_measure_batch(&styled, LayoutOptions::default());
        assert_eq!(batch.requests[0].available_width, LayoutUnit::from_px(92));
        assert_eq!(batch.requests[0].max_width, LayoutUnit::from_px(92));
    }

    #[test]
    fn nested_container_geometry_drives_host_measurement_and_page_scene() {
        let mut chapter = empty_chapter();
        let paragraph_style = chapter
            .styles
            .intern(
                document::ComputedStyle::new()
                    .with_property("font-size", "20px")
                    .with_property("text-indent", "2em"),
            )
            .expect("paragraph style");
        let range = chapter
            .text_pool
            .push("nested contents entry uses host measurement")
            .expect("text");
        let paragraph = chapter
            .nodes
            .push(DocumentNode::Paragraph(BlockText {
                text: TextRange {
                    start: range.start,
                    end: range.end,
                },
                style: paragraph_style,
                style_runs: Vec::new(),
            }))
            .expect("paragraph");
        let inner = chapter
            .nodes
            .push(DocumentNode::Container(ContainerNode {
                children: vec![paragraph],
                style: StyleId::new(0),
            }))
            .expect("inner container");
        let container_style = chapter
            .styles
            .intern(
                document::ComputedStyle::new()
                    .with_property("font-size", "20px")
                    .with_property("margin", "4.8em 10% .32em 3.125%")
                    .with_property("padding", "1em 2%"),
            )
            .expect("container style");
        let outer = chapter
            .nodes
            .push(DocumentNode::Container(ContainerNode {
                children: vec![inner],
                style: container_style,
            }))
            .expect("outer container");
        set_root_children(&mut chapter, vec![outer]);
        chapter.rebuild_utf16_index();

        let constraints =
            LayoutConstraints::new(LayoutUnit::from_px(400), LayoutUnit::from_px(500))
                .with_margin(LayoutUnit::from_px(40));
        let options = LayoutOptions::new(constraints);
        let prepared = HostMeasuredLayout::prepare(chapter, options);
        let request = &prepared.measure_batch().requests[0];
        let containing_width = LayoutUnit::from_px(320);
        let margin_left = LayoutUnit::from_f64_px(10.0);
        let margin_right = LayoutUnit::from_f64_px(32.0);
        let padding_inline = LayoutUnit::from_f64_px(6.4);
        let text_indent = LayoutUnit::from_px(40);
        let expected_width = containing_width
            - margin_left
            - margin_right
            - padding_inline
            - padding_inline
            - text_indent;

        assert_eq!(request.available_width, expected_width);
        assert_eq!(request.max_width, expected_width);

        let fallback = DefaultTextBackend::new();
        let measured = fallback
            .measure_batch(prepared.measure_batch(), &CancellationToken::new())
            .expect("measure nested geometry");
        let pages = prepared
            .resume(MeasuredBatch::new(
                TextBackendId(0x686f_7374),
                FontSetFingerprint(0x666f_6e74),
                measured.results,
            ))
            .expect("resume host-measured geometry");
        let first_line = visible_text_line_rects(&pages.pages[0])[0];

        assert_eq!(
            first_line.x,
            LayoutUnit::from_px(40) + margin_left + padding_inline + text_indent
        );
        assert_eq!(first_line.y, LayoutUnit::from_px(156));
    }

    #[test]
    fn contents_container_margins_preserve_first_page_space_and_collapse_between_entries() {
        let mut chapter = empty_chapter();
        let first_text = push_paragraph(&mut chapter, "Contents");
        let second_text = push_paragraph(&mut chapter, "Chapter one");
        let first_style = chapter
            .styles
            .intern(
                document::ComputedStyle::new()
                    .with_property("margin-top", "32px")
                    .with_property("margin-bottom", "10px"),
            )
            .expect("first entry style");
        let second_style = chapter
            .styles
            .intern(document::ComputedStyle::new().with_property("margin-top", "24px"))
            .expect("second entry style");
        let first = chapter
            .nodes
            .push(DocumentNode::Container(ContainerNode {
                children: vec![first_text],
                style: first_style,
            }))
            .expect("first entry");
        let second = chapter
            .nodes
            .push(DocumentNode::Container(ContainerNode {
                children: vec![second_text],
                style: second_style,
            }))
            .expect("second entry");
        set_root_children(&mut chapter, vec![first, second]);

        let pages = paginate_chapter(
            &chapter,
            &DefaultTextBackend::new(),
            LayoutConstraints::new(LayoutUnit::from_px(300), LayoutUnit::from_px(200)),
        )
        .expect("paginate contents entries");
        let lines = visible_text_line_rects(&pages.pages[0]);

        assert_eq!(lines[0].y, LayoutUnit::from_px(32));
        assert_eq!(lines[1].y, LayoutUnit::from_px(76));
    }

    #[test]
    fn paragraph_continuation_suppresses_top_margin_and_padding() {
        let mut chapter = chapter_with_paragraph(
            "one two three four five six seven eight nine ten eleven twelve thirteen",
        );
        let style = chapter
            .styles
            .intern(
                document::ComputedStyle::new()
                    .with_property("margin-top", "40px")
                    .with_property("padding-top", "20px"),
            )
            .expect("style");
        let paragraph = chapter
            .nodes
            .iter_with_ids()
            .find_map(|(node_id, node)| {
                matches!(node, DocumentNode::Paragraph(_)).then_some(node_id)
            })
            .expect("paragraph");
        if let Some(DocumentNode::Paragraph(text)) = chapter.nodes.get_mut(paragraph) {
            text.style = style;
        }

        let pages = paginate_chapter_with_options(
            &chapter,
            &DefaultTextBackend::new(),
            LayoutOptions {
                constraints: LayoutConstraints::new(
                    LayoutUnit::from_px(90),
                    LayoutUnit::from_px(100),
                ),
                max_pages: 16,
                ..LayoutOptions::default()
            },
        )
        .expect("paginate continuation");
        let first_lines = pages
            .pages
            .iter()
            .map(|page| visible_text_line_rects(page)[0])
            .collect::<Vec<_>>();

        assert!(first_lines.len() > 1);
        assert_eq!(first_lines[0].y, LayoutUnit::from_px(60));
        assert_eq!(first_lines[1].y, LayoutUnit::ZERO);
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
        let line = visible_text_line_rects(&pages.pages[0])[0];

        assert_eq!(line.height, LayoutUnit::from_px(30));
    }

    #[test]
    fn default_line_height_matches_epubjs_reader_baseline() {
        let mut chapter = chapter_with_paragraph("reader default line height");
        let style = document::ComputedStyle::new().with_property("font-size", "12.8px");
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
            &DefaultTextBackend::new(),
            LayoutConstraints::default(),
        )
        .expect("pagination");
        let line = visible_text_line_rects(&pages.pages[0])[0];

        assert_eq!(line.height, LayoutUnit::from_px(20));
    }

    #[test]
    fn default_paragraphs_do_not_add_implicit_block_margins() {
        let mut chapter = empty_chapter();
        let first = push_paragraph(&mut chapter, "first");
        let second = push_paragraph(&mut chapter, "second");
        set_root_children(&mut chapter, vec![first, second]);
        chapter.rebuild_utf16_index();

        let pages = paginate_chapter(
            &chapter,
            &DefaultTextBackend::new(),
            LayoutConstraints::default(),
        )
        .expect("pagination");
        let lines = visible_text_line_rects(&pages.pages[0]);

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1].y - lines[0].y, LayoutUnit::from_px(20));
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
        let paint = &pages.pages[0].text_paints[0];

        let hit = hit_test(
            &pages.pages[0],
            paint.layout_rect.x + LayoutUnit::from_px(4),
            paint.layout_rect.y + LayoutUnit::from_px(4),
        )
        .expect("hit");

        assert_eq!(hit.node_id, paint.node_id);
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
        assert!(json.contains(r#""paragraphs""#));
        assert!(json.contains(r#""text_paints""#));
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
                    LayoutUnit::from_px(20),
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
                style_runs: Vec::new(),
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
