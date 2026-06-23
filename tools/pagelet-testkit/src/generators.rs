use std::{collections::BTreeSet, fmt};

use pagelet::{core::LayoutUnit, text::FontSetFingerprint};

use crate::random::DeterministicRng;

/// Upper bounds applied to every property generator.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct GeneratorLimits {
    /// Maximum recursive tree depth.
    pub max_depth: usize,
    /// Maximum structural nodes.
    pub max_nodes: usize,
    /// Maximum generated UTF-8 byte length for text-bearing cases.
    pub max_text_len: usize,
    /// Maximum generated resources or archive entries.
    pub max_resources: usize,
    /// Maximum generated page count hints.
    pub max_pages: usize,
}

impl GeneratorLimits {
    /// Small deterministic limits for unit-test smoke coverage.
    pub const fn smoke() -> Self {
        Self {
            max_depth: 4,
            max_nodes: 24,
            max_text_len: 160,
            max_resources: 8,
            max_pages: 6,
        }
    }

    fn normalized(self) -> Self {
        Self {
            max_depth: self.max_depth.max(1),
            max_nodes: self.max_nodes.max(1),
            max_text_len: self.max_text_len.max(1),
            max_resources: self.max_resources.max(1),
            max_pages: self.max_pages.max(1),
        }
    }
}

impl Default for GeneratorLimits {
    fn default() -> Self {
        Self {
            max_depth: 6,
            max_nodes: 64,
            max_text_len: 512,
            max_resources: 32,
            max_pages: 16,
        }
    }
}

/// Seed and limits to print from failing property assertions.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PropertyFailureContext {
    /// Logical case name supplied by the test.
    pub case_name: String,
    /// Reproducible seed.
    pub seed: u64,
    /// Limits used while generating the case.
    pub limits: GeneratorLimits,
}

impl fmt::Display for PropertyFailureContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "property case '{}' failed with seed={} limits={:?}",
            self.case_name, self.seed, self.limits
        )
    }
}

/// Reproducible generator for parser, document, text, and layout properties.
#[derive(Debug, Clone)]
pub struct PropertyGenerator {
    seed: u64,
    rng: DeterministicRng,
    limits: GeneratorLimits,
}

impl PropertyGenerator {
    /// Create a generator with default limits.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self::with_limits(seed, GeneratorLimits::default())
    }

    /// Create a generator with explicit limits.
    #[must_use]
    pub fn with_limits(seed: u64, limits: GeneratorLimits) -> Self {
        Self {
            seed,
            rng: DeterministicRng::new(seed),
            limits: limits.normalized(),
        }
    }

    /// Return the generator seed.
    #[must_use]
    pub const fn seed(&self) -> u64 {
        self.seed
    }

    /// Return normalized generation limits.
    #[must_use]
    pub const fn limits(&self) -> GeneratorLimits {
        self.limits
    }

    /// Build context suitable for property assertion failure messages.
    #[must_use]
    pub fn failure_context(&self, case_name: impl Into<String>) -> PropertyFailureContext {
        PropertyFailureContext {
            case_name: case_name.into(),
            seed: self.seed,
            limits: self.limits,
        }
    }

    /// Generate valid OCF-style relative paths and entry bytes.
    #[must_use]
    pub fn ocf_entries(&mut self) -> GeneratedOcfArchive {
        let entry_count = self.count_between(1, self.limits.max_resources);
        let mut entries = Vec::with_capacity(entry_count);
        let mut paths = BTreeSet::new();

        entries.push(GeneratedOcfEntry {
            path: GeneratedOcfPath::new("META-INF/container.xml")
                .expect("static OCF path is valid"),
            media_type: "application/xml".into(),
            bytes: br#"<?xml version="1.0"?><container/>"#.to_vec(),
        });
        paths.insert("META-INF/container.xml".to_owned());

        for index in 1..entry_count {
            let path = self.unique_resource_path(index, &mut paths);
            let media_type = media_type_for_path(path.as_str());
            let bytes = bytes_for_entry(path.as_str(), media_type, index);
            entries.push(GeneratedOcfEntry {
                path,
                media_type: media_type.into(),
                bytes,
            });
        }

        GeneratedOcfArchive { entries }
    }

    /// Generate an XML tree with bounded depth, node count, namespaces, and attributes.
    #[must_use]
    pub fn xml_tree(&mut self) -> GeneratedXmlElement {
        let mut remaining_nodes = self.limits.max_nodes;
        let mut remaining_text = self.limits.max_text_len;
        self.xml_element(0, &mut remaining_nodes, &mut remaining_text)
    }

    /// Generate a manifest, spine, and acyclic fallback graph.
    #[must_use]
    pub fn manifest_graph(&mut self) -> GeneratedManifestGraph {
        let item_count = self.count_between(1, self.limits.max_resources);
        let mut manifest = Vec::with_capacity(item_count);
        let mut fallback_edges = Vec::new();

        for index in 0..item_count {
            let media_type = match index % 4 {
                0 => "application/xhtml+xml",
                1 => "text/css",
                2 => "image/png",
                _ => "application/octet-stream",
            };
            let id = format!("item{index}");
            let fallback = if index + 1 < item_count && self.rng.next_bool() {
                Some(format!("item{}", index + 1))
            } else {
                None
            };
            if let Some(to) = &fallback {
                fallback_edges.push(GeneratedFallbackEdge {
                    from: id.clone(),
                    to: to.clone(),
                });
            }
            manifest.push(GeneratedManifestItem {
                id,
                href: resource_href(index, media_type),
                media_type: media_type.into(),
                fallback,
                properties: generated_properties(index),
            });
        }

        let mut spine: Vec<GeneratedSpineItem> = manifest
            .iter()
            .filter(|item| item.media_type == "application/xhtml+xml")
            .map(|item| GeneratedSpineItem {
                idref: item.id.clone(),
                linear: self.rng.next_bool(),
                properties: Vec::new(),
            })
            .collect();
        if spine.is_empty() {
            spine.push(GeneratedSpineItem {
                idref: manifest[0].id.clone(),
                linear: true,
                properties: Vec::new(),
            });
        }

        GeneratedManifestGraph {
            manifest,
            spine,
            fallback_edges,
        }
    }

    /// Generate a lightweight ChapterIR-shaped document tree.
    #[must_use]
    pub fn chapter_ir(&mut self) -> GeneratedChapterIr {
        let mut remaining_nodes = self.limits.max_nodes.saturating_sub(1);
        let mut remaining_text = self.limits.max_text_len;
        let mut blocks = Vec::new();
        let block_target = self.count_between(1, self.limits.max_nodes.min(8));

        for index in 0..block_target {
            if remaining_nodes == 0 {
                break;
            }
            remaining_nodes -= 1;
            let block = match self.rng.bounded(4) {
                0 => GeneratedBlock::Heading {
                    level: 1 + self.rng.bounded(3) as u8,
                    inlines: self.inline_runs(&mut remaining_nodes, &mut remaining_text),
                },
                1 => GeneratedBlock::Image {
                    resource_path: format!("EPUB/images/image-{index}.png"),
                    alt_text: self.take_unicode_text(&mut remaining_text, 24),
                },
                2 => GeneratedBlock::Footnote {
                    id: format!("fn{index}"),
                    inlines: self.inline_runs(&mut remaining_nodes, &mut remaining_text),
                },
                _ => GeneratedBlock::Paragraph {
                    inlines: self.inline_runs(&mut remaining_nodes, &mut remaining_text),
                },
            };
            blocks.push(block);
        }

        GeneratedChapterIr {
            chapter_id: format!("chapter-{:x}", self.rng.next_u64()),
            blocks,
        }
    }

    /// Generate Unicode text with grapheme-shaped spans, bidi runs, and break opportunities.
    #[must_use]
    pub fn unicode_text(&mut self) -> GeneratedUnicodeText {
        self.unicode_text_with_limit(self.limits.max_text_len)
    }

    /// Generate viewport and font metrics for layout properties.
    #[must_use]
    pub fn layout_config(&mut self) -> GeneratedLayoutConfig {
        let width = 240 + self.rng.bounded(801) as i64;
        let height = 320 + self.rng.bounded(1_281) as i64;
        let margin = self.rng.bounded(25) as i64;
        let font_size = 12 + self.rng.bounded(17) as i64;
        let ascent = (font_size * 4) / 5;
        let descent = (font_size / 5).max(1);
        let leading = self.rng.bounded(5) as i64;

        GeneratedLayoutConfig {
            viewport: GeneratedViewport {
                width: LayoutUnit::from_px(width),
                height: LayoutUnit::from_px(height),
                margin_start: LayoutUnit::from_px(margin),
                margin_end: LayoutUnit::from_px(margin),
                margin_top: LayoutUnit::from_px(margin / 2),
                margin_bottom: LayoutUnit::from_px(margin / 2),
            },
            font_metrics: GeneratedFontMetrics {
                font_size: LayoutUnit::from_px(font_size),
                ascent: LayoutUnit::from_px(ascent),
                descent: LayoutUnit::from_px(descent),
                leading: LayoutUnit::from_px(leading),
                fingerprint: FontSetFingerprint(self.rng.next_u64()),
            },
            page_count_hint: 1 + self.rng.bounded(self.limits.max_pages as u64) as u32,
        }
    }

    fn count_between(&mut self, min: usize, max: usize) -> usize {
        let max = max.max(min);
        min + self.rng.bounded_usize(max - min + 1)
    }

    fn unique_resource_path(
        &mut self,
        index: usize,
        existing: &mut BTreeSet<String>,
    ) -> GeneratedOcfPath {
        loop {
            let path = match self.rng.bounded(4) {
                0 => format!("EPUB/chapter-{index}.xhtml"),
                1 => format!("EPUB/styles/style-{index}.css"),
                2 => format!("EPUB/images/image-{index}.png"),
                _ => format!("EPUB/resources/resource-{index}.bin"),
            };
            if existing.insert(path.clone()) {
                return GeneratedOcfPath::new(path).expect("generated OCF path is valid");
            }
        }
    }

    fn xml_element(
        &mut self,
        depth: usize,
        remaining_nodes: &mut usize,
        remaining_text: &mut usize,
    ) -> GeneratedXmlElement {
        *remaining_nodes = remaining_nodes.saturating_sub(1);
        let name = format!("node{depth}_{}", self.rng.bounded(1_000));
        let namespaces = if depth == 0 {
            vec![
                GeneratedXmlNamespace {
                    prefix: None,
                    uri: "urn:pagelet:test".into(),
                },
                GeneratedXmlNamespace {
                    prefix: Some("p".into()),
                    uri: "urn:pagelet:prefixed".into(),
                },
            ]
        } else {
            Vec::new()
        };

        let attr_count = self.count_between(0, 2);
        let mut attributes = Vec::with_capacity(attr_count);
        for index in 0..attr_count {
            attributes.push(GeneratedXmlAttribute {
                prefix: if depth == 0 && index == 0 {
                    Some("p".into())
                } else {
                    None
                },
                name: format!("attr{index}"),
                value: self.take_ascii_text(remaining_text, 16),
            });
        }

        let can_have_children = depth + 1 < self.limits.max_depth && *remaining_nodes > 0;
        let child_count = if can_have_children {
            self.count_between(0, (*remaining_nodes).min(3))
        } else {
            0
        };
        let mut children = Vec::with_capacity(child_count);
        for _ in 0..child_count {
            if *remaining_nodes == 0 {
                break;
            }
            children.push(self.xml_element(depth + 1, remaining_nodes, remaining_text));
        }

        let text = if (children.is_empty() || self.rng.next_bool()) && *remaining_text > 0 {
            Some(self.take_ascii_text(remaining_text, 32))
        } else {
            None
        };

        GeneratedXmlElement {
            name,
            namespaces,
            attributes,
            text,
            children,
        }
    }

    fn inline_runs(
        &mut self,
        remaining_nodes: &mut usize,
        remaining_text: &mut usize,
    ) -> Vec<GeneratedInline> {
        if *remaining_nodes == 0 {
            return Vec::new();
        }
        let inline_count = self.count_between(1, (*remaining_nodes).min(3));
        let mut inlines = Vec::with_capacity(inline_count);
        for index in 0..inline_count {
            if *remaining_nodes == 0 {
                break;
            }
            *remaining_nodes -= 1;
            let text = self.take_unicode_text(remaining_text, 32);
            let inline = match self.rng.bounded(3) {
                0 => GeneratedInline::Emphasis(text),
                1 => GeneratedInline::Link {
                    href: format!("chapter-{}.xhtml#p{index}", self.rng.bounded(4)),
                    text,
                },
                _ => GeneratedInline::Text(text),
            };
            inlines.push(inline);
        }
        inlines
    }

    fn take_unicode_text(&mut self, remaining_text: &mut usize, preferred: usize) -> String {
        if *remaining_text == 0 {
            return String::new();
        }
        let limit = (*remaining_text).min(preferred.max(1));
        let generated = self.unicode_text_with_limit(limit).text;
        *remaining_text = remaining_text.saturating_sub(generated.len());
        generated
    }

    fn unicode_text_with_limit(&mut self, max_len: usize) -> GeneratedUnicodeText {
        let max_len = max_len.max(1);
        let target_clusters = self.count_between(1, max_len.min(24));
        let mut text = String::new();
        let mut graphemes = Vec::new();
        let mut runs = Vec::<GeneratedBidiRun>::new();
        let mut line_breaks = Vec::new();

        for index in 0..target_clusters {
            if index > 0 && self.rng.bounded(4) == 0 && text.len() < max_len {
                let start = text.len();
                text.push(' ');
                let end = text.len();
                graphemes.push(GeneratedGrapheme { start, end });
                line_breaks.push(end);
                push_bidi_run(&mut runs, start, end, GeneratedTextDirection::Ltr);
            }

            let cluster = unicode_cluster(self.rng.bounded_usize(UNICODE_CLUSTERS.len()));
            if text.len() + cluster.len() > max_len {
                break;
            }
            let start = text.len();
            text.push_str(cluster);
            let end = text.len();
            graphemes.push(GeneratedGrapheme { start, end });
            push_bidi_run(&mut runs, start, end, direction_for_cluster(cluster));
            if self.rng.bounded(5) == 0 {
                line_breaks.push(end);
            }
        }

        if text.is_empty() {
            text.push('a');
            graphemes.push(GeneratedGrapheme { start: 0, end: 1 });
            runs.push(GeneratedBidiRun {
                start: 0,
                end: 1,
                direction: GeneratedTextDirection::Ltr,
                level: 0,
            });
        }

        line_breaks.sort_unstable();
        line_breaks.dedup();

        GeneratedUnicodeText {
            text,
            graphemes,
            bidi_runs: runs,
            line_breaks,
        }
    }

    fn ascii_text(&mut self, max_len: usize) -> String {
        let len = self.count_between(1, max_len.max(1));
        let mut out = String::with_capacity(len);
        for _ in 0..len {
            let ch = (b'a' + self.rng.bounded(26) as u8) as char;
            out.push(ch);
        }
        out
    }

    fn take_ascii_text(&mut self, remaining_text: &mut usize, preferred: usize) -> String {
        if *remaining_text == 0 {
            return String::new();
        }
        let limit = (*remaining_text).min(preferred.max(1));
        let generated = self.ascii_text(limit);
        *remaining_text = remaining_text.saturating_sub(generated.len());
        generated
    }
}

/// Valid relative OCF path generated for archive entries.
#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct GeneratedOcfPath {
    value: String,
}

impl GeneratedOcfPath {
    /// Validate and create a relative OCF path.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Option<Self> {
        let value = value.into();
        if is_valid_ocf_path(&value) {
            Some(Self { value })
        } else {
            None
        }
    }

    /// Return the path string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.value
    }
}

/// Generated OCF archive entry.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct GeneratedOcfEntry {
    pub path: GeneratedOcfPath,
    pub media_type: String,
    pub bytes: Vec<u8>,
}

/// Generated OCF archive entry set.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct GeneratedOcfArchive {
    pub entries: Vec<GeneratedOcfEntry>,
}

impl GeneratedOcfArchive {
    /// Return true when all entry paths are relative and unique.
    #[must_use]
    pub fn has_unique_valid_paths(&self) -> bool {
        let mut seen = BTreeSet::new();
        self.entries
            .iter()
            .all(|entry| seen.insert(entry.path.as_str()) && is_valid_ocf_path(entry.path.as_str()))
    }
}

/// Generated XML namespace declaration.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct GeneratedXmlNamespace {
    pub prefix: Option<String>,
    pub uri: String,
}

/// Generated XML attribute.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct GeneratedXmlAttribute {
    pub prefix: Option<String>,
    pub name: String,
    pub value: String,
}

/// Generated XML element tree.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct GeneratedXmlElement {
    pub name: String,
    pub namespaces: Vec<GeneratedXmlNamespace>,
    pub attributes: Vec<GeneratedXmlAttribute>,
    pub text: Option<String>,
    pub children: Vec<GeneratedXmlElement>,
}

impl GeneratedXmlElement {
    /// Count element nodes.
    #[must_use]
    pub fn node_count(&self) -> usize {
        1 + self
            .children
            .iter()
            .map(GeneratedXmlElement::node_count)
            .sum::<usize>()
    }

    /// Count maximum tree depth, with a leaf depth of 1.
    #[must_use]
    pub fn tree_depth(&self) -> usize {
        1 + self
            .children
            .iter()
            .map(GeneratedXmlElement::tree_depth)
            .max()
            .unwrap_or(0)
    }

    /// Count UTF-8 bytes in text nodes.
    #[must_use]
    pub fn text_len(&self) -> usize {
        self.text.as_ref().map_or(0, String::len)
            + self
                .attributes
                .iter()
                .map(|attribute| attribute.value.len())
                .sum::<usize>()
            + self
                .children
                .iter()
                .map(GeneratedXmlElement::text_len)
                .sum::<usize>()
    }
}

/// Generated OPF manifest item.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct GeneratedManifestItem {
    pub id: String,
    pub href: String,
    pub media_type: String,
    pub fallback: Option<String>,
    pub properties: Vec<String>,
}

/// Generated spine itemref.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct GeneratedSpineItem {
    pub idref: String,
    pub linear: bool,
    pub properties: Vec<String>,
}

/// Generated fallback graph edge.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct GeneratedFallbackEdge {
    pub from: String,
    pub to: String,
}

/// Generated manifest, spine, and fallback graph.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct GeneratedManifestGraph {
    pub manifest: Vec<GeneratedManifestItem>,
    pub spine: Vec<GeneratedSpineItem>,
    pub fallback_edges: Vec<GeneratedFallbackEdge>,
}

impl GeneratedManifestGraph {
    /// Return true when all spine and fallback references resolve to manifest ids.
    #[must_use]
    pub fn references_are_resolved(&self) -> bool {
        let ids: BTreeSet<&str> = self.manifest.iter().map(|item| item.id.as_str()).collect();
        self.spine
            .iter()
            .all(|item| ids.contains(item.idref.as_str()))
            && self
                .fallback_edges
                .iter()
                .all(|edge| ids.contains(edge.from.as_str()) && ids.contains(edge.to.as_str()))
    }

    /// Return true when generated fallback edges are acyclic by item order.
    #[must_use]
    pub fn fallback_edges_are_acyclic(&self) -> bool {
        self.fallback_edges.iter().all(|edge| {
            let from = item_index(&edge.from);
            let to = item_index(&edge.to);
            matches!((from, to), (Some(left), Some(right)) if right > left)
        })
    }
}

/// Lightweight ChapterIR-shaped test document.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct GeneratedChapterIr {
    pub chapter_id: String,
    pub blocks: Vec<GeneratedBlock>,
}

impl GeneratedChapterIr {
    /// Count block and inline nodes.
    #[must_use]
    pub fn node_count(&self) -> usize {
        1 + self
            .blocks
            .iter()
            .map(GeneratedBlock::node_count)
            .sum::<usize>()
    }

    /// Count UTF-8 bytes in generated text payloads.
    #[must_use]
    pub fn text_len(&self) -> usize {
        self.blocks.iter().map(GeneratedBlock::text_len).sum()
    }
}

/// Generated block node.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum GeneratedBlock {
    Paragraph {
        inlines: Vec<GeneratedInline>,
    },
    Heading {
        level: u8,
        inlines: Vec<GeneratedInline>,
    },
    Image {
        resource_path: String,
        alt_text: String,
    },
    Footnote {
        id: String,
        inlines: Vec<GeneratedInline>,
    },
}

impl GeneratedBlock {
    fn node_count(&self) -> usize {
        match self {
            Self::Paragraph { inlines } | Self::Heading { inlines, .. } => 1 + inlines.len(),
            Self::Image { .. } => 1,
            Self::Footnote { inlines, .. } => 1 + inlines.len(),
        }
    }

    fn text_len(&self) -> usize {
        match self {
            Self::Paragraph { inlines } | Self::Heading { inlines, .. } => {
                inlines.iter().map(GeneratedInline::text_len).sum()
            }
            Self::Image { alt_text, .. } => alt_text.len(),
            Self::Footnote { inlines, .. } => inlines.iter().map(GeneratedInline::text_len).sum(),
        }
    }
}

/// Generated inline node.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum GeneratedInline {
    Text(String),
    Emphasis(String),
    Link { href: String, text: String },
}

impl GeneratedInline {
    fn text_len(&self) -> usize {
        match self {
            Self::Text(text) | Self::Emphasis(text) | Self::Link { text, .. } => text.len(),
        }
    }
}

/// Generated Unicode text bundle.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct GeneratedUnicodeText {
    pub text: String,
    pub graphemes: Vec<GeneratedGrapheme>,
    pub bidi_runs: Vec<GeneratedBidiRun>,
    pub line_breaks: Vec<usize>,
}

/// Byte range representing one generated grapheme-shaped cluster.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct GeneratedGrapheme {
    pub start: usize,
    pub end: usize,
}

/// Generated bidi run.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct GeneratedBidiRun {
    pub start: usize,
    pub end: usize,
    pub direction: GeneratedTextDirection,
    pub level: u8,
}

/// Generated text direction.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum GeneratedTextDirection {
    Ltr,
    Rtl,
}

/// Generated layout configuration and metrics.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct GeneratedLayoutConfig {
    pub viewport: GeneratedViewport,
    pub font_metrics: GeneratedFontMetrics,
    pub page_count_hint: u32,
}

/// Generated viewport in layout units.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct GeneratedViewport {
    pub width: LayoutUnit,
    pub height: LayoutUnit,
    pub margin_start: LayoutUnit,
    pub margin_end: LayoutUnit,
    pub margin_top: LayoutUnit,
    pub margin_bottom: LayoutUnit,
}

/// Generated deterministic font metrics.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct GeneratedFontMetrics {
    pub font_size: LayoutUnit,
    pub ascent: LayoutUnit,
    pub descent: LayoutUnit,
    pub leading: LayoutUnit,
    pub fingerprint: FontSetFingerprint,
}

fn is_valid_ocf_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && path
            .split('/')
            .all(|component| !component.is_empty() && component != "." && component != "..")
}

fn media_type_for_path(path: &str) -> &'static str {
    if path.ends_with(".xhtml") {
        "application/xhtml+xml"
    } else if path.ends_with(".css") {
        "text/css"
    } else if path.ends_with(".png") {
        "image/png"
    } else {
        "application/octet-stream"
    }
}

fn bytes_for_entry(path: &str, media_type: &str, index: usize) -> Vec<u8> {
    match media_type {
        "application/xhtml+xml" => {
            format!(r#"<html><body><p id="p{index}">{path}</p></body></html>"#).into_bytes()
        }
        "text/css" => format!("p {{ margin: {index}px; }}").into_bytes(),
        "image/png" => b"\x89PNG\r\n\x1a\npagelet-test-image".to_vec(),
        _ => format!("resource:{path}:{index}").into_bytes(),
    }
}

fn resource_href(index: usize, media_type: &str) -> String {
    match media_type {
        "application/xhtml+xml" => format!("chapter-{index}.xhtml"),
        "text/css" => format!("style-{index}.css"),
        "image/png" => format!("image-{index}.png"),
        _ => format!("resource-{index}.bin"),
    }
}

fn generated_properties(index: usize) -> Vec<String> {
    match index % 5 {
        0 => vec!["nav".into()],
        1 => vec!["cover-image".into()],
        2 => vec!["remote-resources".into()],
        _ => Vec::new(),
    }
}

fn item_index(id: &str) -> Option<usize> {
    id.strip_prefix("item")?.parse().ok()
}

const UNICODE_CLUSTERS: [&str; 10] = [
    "a",
    "b",
    "e\u{0301}",
    "\u{00e9}",
    "\u{4e2d}",
    "\u{6587}",
    "\u{05d0}",
    "\u{0645}",
    "\u{1f469}\u{200d}\u{1f4bb}",
    ".",
];

fn unicode_cluster(index: usize) -> &'static str {
    UNICODE_CLUSTERS[index % UNICODE_CLUSTERS.len()]
}

fn direction_for_cluster(cluster: &str) -> GeneratedTextDirection {
    if cluster
        .chars()
        .any(|ch| matches!(ch, '\u{0590}'..='\u{08ff}' | '\u{fb1d}'..='\u{fdff}'))
    {
        GeneratedTextDirection::Rtl
    } else {
        GeneratedTextDirection::Ltr
    }
}

fn push_bidi_run(
    runs: &mut Vec<GeneratedBidiRun>,
    start: usize,
    end: usize,
    direction: GeneratedTextDirection,
) {
    let level = match direction {
        GeneratedTextDirection::Ltr => 0,
        GeneratedTextDirection::Rtl => 1,
    };
    if let Some(last) = runs.last_mut() {
        if last.direction == direction && last.end == start {
            last.end = end;
            return;
        }
    }

    runs.push(GeneratedBidiRun {
        start,
        end,
        direction,
        level,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn property_generators_are_replayable() {
        let limits = GeneratorLimits::smoke();
        let mut first = PropertyGenerator::with_limits(0x5eed, limits);
        let mut second = PropertyGenerator::with_limits(0x5eed, limits);

        assert_eq!(first.ocf_entries(), second.ocf_entries());
        assert_eq!(first.xml_tree(), second.xml_tree());
        assert_eq!(first.manifest_graph(), second.manifest_graph());
        assert_eq!(first.chapter_ir(), second.chapter_ir());
        assert_eq!(first.unicode_text(), second.unicode_text());
        assert_eq!(first.layout_config(), second.layout_config());
    }

    #[test]
    fn property_generators_respect_limits_and_invariants() {
        let limits = GeneratorLimits::smoke();
        let mut generator = PropertyGenerator::with_limits(0xdecaf, limits);
        let context = generator.failure_context("property generator bounds");

        let archive = generator.ocf_entries();
        assert!(archive.entries.len() <= limits.max_resources, "{context}");
        assert!(archive.has_unique_valid_paths(), "{context}");

        let xml = generator.xml_tree();
        assert!(xml.node_count() <= limits.max_nodes, "{context}");
        assert!(xml.tree_depth() <= limits.max_depth, "{context}");
        assert!(xml.text_len() <= limits.max_text_len, "{context}");

        let manifest = generator.manifest_graph();
        assert!(manifest.manifest.len() <= limits.max_resources, "{context}");
        assert!(manifest.references_are_resolved(), "{context}");
        assert!(manifest.fallback_edges_are_acyclic(), "{context}");

        let chapter = generator.chapter_ir();
        assert!(chapter.node_count() <= limits.max_nodes, "{context}");
        assert!(chapter.text_len() <= limits.max_text_len, "{context}");

        let unicode = generator.unicode_text();
        assert!(unicode.text.len() <= limits.max_text_len, "{context}");
        assert!(!unicode.graphemes.is_empty(), "{context}");
        assert!(
            unicode
                .graphemes
                .iter()
                .all(|range| range.start < range.end && range.end <= unicode.text.len()),
            "{context}"
        );
        assert!(
            unicode
                .bidi_runs
                .iter()
                .all(|run| run.start < run.end && run.end <= unicode.text.len()),
            "{context}"
        );
        assert!(
            unicode
                .line_breaks
                .iter()
                .all(|offset| *offset <= unicode.text.len()),
            "{context}"
        );

        let layout = generator.layout_config();
        assert!(layout.viewport.width.raw() > 0, "{context}");
        assert!(layout.viewport.height.raw() > 0, "{context}");
        assert!(layout.font_metrics.font_size.raw() > 0, "{context}");
        assert!(layout.font_metrics.ascent.raw() > 0, "{context}");
        assert!(
            layout.page_count_hint <= limits.max_pages as u32,
            "{context}"
        );
    }
}
