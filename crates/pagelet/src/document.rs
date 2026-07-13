//! Semantic book and chapter intermediate representations.

use std::{collections::BTreeMap, sync::Arc};

use crate::core::{
    make_stable_block_id, BlockFingerprint, ContentHash, Diagnostic, DiagnosticCode, DocumentId,
    NodeId, PageletError, ResourceId, ResourceLimitError, ResourceLimitKind, SourceRange, StyleId,
};

/// Package-level intermediate representation.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct BookIr {
    pub metadata: BookMetadata,
    pub package: PackageInfo,
    pub manifest: Vec<ManifestItem>,
    pub spine: Vec<SpineItem>,
    pub navigation: Navigation,
    pub resources: ResourceTable,
    pub capabilities: CapabilityReport,
}

impl BookIr {
    /// Serialize a stable subset suitable for golden tests and CLI inspect.
    #[must_use]
    pub fn to_golden_json(&self) -> String {
        let mut out = String::new();
        out.push_str("{\n");
        push_json_field(&mut out, 1, "title", self.metadata.title.as_deref(), true);
        push_json_field(
            &mut out,
            1,
            "language",
            self.metadata.language.as_deref(),
            true,
        );
        push_json_field(
            &mut out,
            1,
            "identifier",
            self.metadata.identifier.as_deref(),
            true,
        );
        push_json_string(&mut out, 1, "rootfile", &self.package.rootfile_path, true);
        push_json_string(&mut out, 1, "package_version", &self.package.version, true);
        push_manifest_json(&mut out, &self.manifest);
        out.push_str(",\n");
        push_spine_json(&mut out, &self.spine);
        out.push_str(",\n");
        push_resources_json(&mut out, &self.resources.resources);
        out.push('\n');
        out.push_str("}\n");
        out
    }
}

/// Book metadata copied from the package document.
#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct BookMetadata {
    pub package_version: Arc<str>,
    pub unique_identifier: Arc<str>,
    pub identifier: Option<Arc<str>>,
    pub title: Option<Arc<str>>,
    pub language: Option<Arc<str>>,
    pub cover_image: Option<Arc<str>>,
}

/// Package identity and reading-order metadata.
#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct PackageInfo {
    pub rootfile_path: Arc<str>,
    pub version: Arc<str>,
    pub spine_toc: Option<Arc<str>>,
    pub page_progression_direction: Option<Arc<str>>,
}

/// Manifest item inside [`BookIr`].
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ManifestItem {
    pub id: Arc<str>,
    pub href: Arc<str>,
    pub resolved_path: Arc<str>,
    pub media_type: Arc<str>,
    pub properties: Vec<Arc<str>>,
    pub fallback: Option<Arc<str>>,
    pub media_overlay: Option<Arc<str>>,
    pub resource_id: Option<ResourceId>,
}

/// Spine item inside [`BookIr`].
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SpineItem {
    pub idref: Arc<str>,
    pub linear: bool,
    pub properties: Vec<Arc<str>>,
    pub manifest_index: Option<u32>,
    pub href: Option<Arc<str>>,
}

/// Navigation model stored by [`BookIr`].
#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct Navigation {
    pub source: Arc<str>,
    pub toc: Vec<NavigationItem>,
    pub page_list: Vec<NavigationItem>,
    pub landmarks: Vec<NavigationItem>,
}

/// One navigation item.
#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct NavigationItem {
    pub label: Arc<str>,
    pub href: Arc<str>,
    pub children: Vec<NavigationItem>,
}

/// Resource index derived from the publication store and OPF manifest.
#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct ResourceTable {
    pub resources: Vec<ResourceInfo>,
    pub by_path: BTreeMap<Arc<str>, ResourceId>,
    pub images: Vec<ImageResource>,
    pub fonts: Vec<FontResource>,
}

impl ResourceTable {
    /// Create an empty resource table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add one resource to the path index.
    pub fn push(&mut self, resource: ResourceInfo) {
        self.by_path.insert(resource.path.clone(), resource.id);
        if resource.kind == ResourceKind::Image {
            self.images.push(ImageResource {
                id: resource.id,
                path: resource.path.clone(),
                media_type: resource.media_type.clone(),
                byte_length: resource.uncompressed_size,
                intrinsic_size: None,
                orientation: ImageOrientation::Unspecified,
            });
        }
        if resource.kind == ResourceKind::Font {
            self.fonts.push(FontResource {
                id: resource.id,
                path: resource.path.clone(),
                media_type: resource.media_type.clone(),
                fingerprint: ContentHash::from_bytes(resource.path.as_bytes()),
            });
        }
        self.resources.push(resource);
    }

    /// Set an image size discovered from a bounded header read.
    pub fn set_image_size(&mut self, id: ResourceId, size: Option<ImageSize>) {
        if let Some(image) = self.images.iter_mut().find(|image| image.id == id) {
            image.intrinsic_size = size;
        }
    }

    /// Set a stable font fingerprint discovered from resource metadata/header bytes.
    pub fn set_font_fingerprint(&mut self, id: ResourceId, fingerprint: ContentHash) {
        if let Some(font) = self.fonts.iter_mut().find(|font| font.id == id) {
            font.fingerprint = fingerprint;
        }
    }

    /// Resolve a resource path to its typed identifier.
    #[must_use]
    pub fn id_for_path(&self, path: &str) -> Option<ResourceId> {
        self.by_path.get(path).copied()
    }
}

/// One indexed publication resource.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ResourceInfo {
    pub id: ResourceId,
    pub path: Arc<str>,
    pub media_type: Arc<str>,
    pub kind: ResourceKind,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
    pub compression_method: u16,
}

/// High-level resource category.
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum ResourceKind {
    Package,
    Xhtml,
    Css,
    Image,
    Font,
    Navigation,
    Xml,
    #[default]
    Other,
}

impl ResourceKind {
    /// Infer a resource kind from a media type.
    #[must_use]
    pub fn from_media_type(media_type: &str) -> Self {
        match media_type {
            "application/oebps-package+xml" => Self::Package,
            "application/xhtml+xml" | "text/html" => Self::Xhtml,
            "text/css" => Self::Css,
            "application/x-dtbncx+xml" => Self::Navigation,
            "application/xml" | "text/xml" => Self::Xml,
            value if value.starts_with("image/") => Self::Image,
            "font/otf"
            | "font/ttf"
            | "font/woff"
            | "font/woff2"
            | "application/font-sfnt"
            | "application/vnd.ms-opentype" => Self::Font,
            _ => Self::Other,
        }
    }

    /// Stable lowercase label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Package => "package",
            Self::Xhtml => "xhtml",
            Self::Css => "css",
            Self::Image => "image",
            Self::Font => "font",
            Self::Navigation => "navigation",
            Self::Xml => "xml",
            Self::Other => "other",
        }
    }
}

/// Lazy image resource metadata. Bytes are not stored in the IR.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ImageResource {
    pub id: ResourceId,
    pub path: Arc<str>,
    pub media_type: Arc<str>,
    pub byte_length: u64,
    pub intrinsic_size: Option<ImageSize>,
    pub orientation: ImageOrientation,
}

/// Intrinsic image dimensions when known from a header parser.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub struct ImageSize {
    pub width: u32,
    pub height: u32,
}

/// Image orientation metadata.
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq, Hash)]
pub enum ImageOrientation {
    #[default]
    Unspecified,
}

/// Font resource metadata.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct FontResource {
    pub id: ResourceId,
    pub path: Arc<str>,
    pub media_type: Arc<str>,
    pub fingerprint: ContentHash,
}

/// Compatibility/capability report copied into [`BookIr`].
#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct CapabilityReport {
    pub mode: Arc<str>,
    pub capabilities: Vec<Capability>,
}

/// One capability entry.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Capability {
    pub feature: Arc<str>,
    pub status: Arc<str>,
    pub message: Arc<str>,
}

/// Chapter-level intermediate representation.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ChapterIr {
    pub document_id: DocumentId,
    pub href: Arc<str>,
    pub title: Arc<str>,
    pub text_pool: TextPool,
    pub nodes: NodeArena,
    pub root: NodeId,
    pub styles: StyleTable,
    pub anchors: AnchorIndex,
    pub links: Vec<LinkTarget>,
    pub source_map: SourceMap,
    pub utf16_index: Utf16Index,
    pub diagnostics: Vec<Diagnostic>,
    pub content_hash: ContentHash,
    pub block_order: BTreeMap<NodeId, u32>,
    pub block_fingerprints: BTreeMap<NodeId, BlockFingerprint>,
}

impl ChapterIr {
    /// Create an empty chapter with a root set to node 0.
    #[must_use]
    pub fn empty(
        document_id: DocumentId,
        href: impl Into<Arc<str>>,
        title: impl Into<Arc<str>>,
        content_hash: ContentHash,
    ) -> Self {
        Self {
            document_id,
            href: href.into(),
            title: title.into(),
            text_pool: TextPool::new(),
            nodes: NodeArena::new(),
            root: NodeId::new(0),
            styles: StyleTable::new(),
            anchors: AnchorIndex::new(),
            links: Vec::new(),
            source_map: SourceMap::new(),
            utf16_index: Utf16Index::new(),
            diagnostics: Vec::new(),
            content_hash,
            block_order: BTreeMap::new(),
            block_fingerprints: BTreeMap::new(),
        }
    }

    /// Rebuild block order and compute per-block text fingerprints.
    pub fn rebuild_blocks(&mut self) {
        self.block_order.clear();
        self.block_fingerprints.clear();
        let mut order = 0_u32;
        self.collect_blocks(self.root, &mut order);
    }

    fn collect_blocks(&mut self, node_id: NodeId, order: &mut u32) {
        let node = match self.nodes.get(node_id) {
            Some(node) => node,
            None => return,
        };
        match node {
            DocumentNode::Paragraph(_)
            | DocumentNode::Heading(_)
            | DocumentNode::Footnote(_)
            | DocumentNode::Divider
            | DocumentNode::ForcedBreak => {
                self.block_order.insert(node_id, *order);
                *order += 1;
                if let Some(text) = self.node_text(node_id) {
                    self.block_fingerprints
                        .insert(node_id, BlockFingerprint::from_text(&text));
                } else {
                    self.block_fingerprints
                        .insert(node_id, BlockFingerprint::from_text(""));
                }
            }
            _ => {
                let children: Vec<NodeId> = node.children().to_vec();
                for child in children {
                    self.collect_blocks(child, order);
                }
            }
        }
    }

    fn node_text(&self, node_id: NodeId) -> Option<String> {
        let node = self.nodes.get(node_id)?;
        match node {
            DocumentNode::Paragraph(text) => self.text_pool.get(text.text).map(|s| s.to_owned()),
            DocumentNode::Heading(heading) => self
                .text_pool
                .get(heading.content.text)
                .map(|s| s.to_owned()),
            DocumentNode::Footnote(note) => {
                let mut out = String::new();
                for child in &note.children {
                    if let Some(DocumentNode::Paragraph(text)) = self.nodes.get(*child) {
                        if let Some(s) = self.text_pool.get(text.text) {
                            out.push_str(s);
                        }
                    }
                }
                Some(out)
            }
            _ => None,
        }
    }

    /// Return all text-bearing blocks with stable identifiers.
    #[must_use]
    pub fn blocks(&self) -> Vec<ChapterBlock> {
        let mut blocks: Vec<_> = self
            .block_order
            .iter()
            .filter_map(|(node_id, order)| {
                let fingerprint = self.block_fingerprints.get(node_id)?;
                let node = self.nodes.get(*node_id)?;
                let text = self.node_text(*node_id).unwrap_or_default();
                let block_id = make_stable_block_id(&self.href, *order, *fingerprint);
                let kind = chapter_block_kind(node);
                Some(ChapterBlock {
                    node_id: *node_id,
                    order: *order,
                    block_id,
                    kind: kind.to_owned(),
                    text,
                    fingerprint: *fingerprint,
                })
            })
            .collect();
        blocks.sort_by_key(|block| block.order);
        blocks
    }

    /// Build EPUB CFI for a given node and optional character offset.
    #[must_use]
    pub fn node_cfi(
        &self,
        node_id: NodeId,
        spine_index: u32,
        character_offset: Option<u32>,
    ) -> Option<crate::core::EpubCfi> {
        let mut steps = Vec::new();
        self.cfi_steps(node_id, &mut steps);
        if steps.is_empty() {
            return None;
        }
        steps.reverse();
        Some(crate::core::EpubCfi::new(
            spine_index,
            steps,
            character_offset,
        ))
    }

    fn cfi_steps(&self, node_id: NodeId, steps: &mut Vec<(u32, Option<Arc<str>>)>) {
        let order = self.block_order.get(&node_id).copied();
        let element_id = self.element_id_for_node(node_id);
        if let Some(order) = order {
            steps.push((order, element_id));
        }
        if let Some(parent) = self.parent_node(node_id) {
            self.cfi_steps(parent, steps);
        }
    }

    fn parent_node(&self, node_id: NodeId) -> Option<NodeId> {
        for (candidate_id, candidate) in self.nodes.iter_with_ids() {
            if candidate.children().contains(&node_id) {
                return Some(candidate_id);
            }
        }
        None
    }

    fn element_id_for_node(&self, node_id: NodeId) -> Option<Arc<str>> {
        for anchor in self.anchors.anchors.values() {
            if anchor.node_id == node_id {
                return Some(anchor.key.clone());
            }
        }
        None
    }

    /// Rebuild derived UTF-16 lookup tables from text-bearing nodes.
    pub fn rebuild_utf16_index(&mut self) {
        self.utf16_index = Utf16Index::build(&self.nodes, &self.text_pool);
    }

    /// Return visible text in semantic reading order.
    #[must_use]
    pub fn visible_text(&self) -> String {
        let mut out = String::new();
        self.push_visible_text(self.root, &mut out);
        out.trim().to_owned()
    }

    /// Serialize a compact, deterministic ChapterIR summary.
    #[must_use]
    pub fn to_golden_json(&self) -> String {
        let mut out = String::new();
        out.push_str("{\n");
        push_json_u32(&mut out, 1, "document_id", self.document_id.get(), true);
        push_json_string(&mut out, 1, "href", &self.href, true);
        push_json_string(&mut out, 1, "title", &self.title, true);
        push_json_u32(&mut out, 1, "root", self.root.get(), true);
        push_json_usize(&mut out, 1, "node_count", self.nodes.len(), true);
        push_json_string(&mut out, 1, "visible_text", &self.visible_text(), true);
        push_anchors_json(&mut out, &self.anchors);
        out.push_str(",\n");
        push_links_json(&mut out, &self.links);
        out.push('\n');
        out.push_str("}\n");
        out
    }

    fn push_visible_text(&self, node_id: NodeId, out: &mut String) {
        let Some(node) = self.nodes.get(node_id) else {
            return;
        };
        match node {
            DocumentNode::Paragraph(text) => self.push_block_text(*text, out),
            DocumentNode::Heading(node) => self.push_block_text(node.content, out),
            DocumentNode::List(node) => {
                for child in &node.children {
                    self.push_visible_text(*child, out);
                }
            }
            DocumentNode::ListItem(node) => {
                for child in &node.children {
                    self.push_visible_text(*child, out);
                }
            }
            DocumentNode::BlockQuote(node)
            | DocumentNode::Figure(node)
            | DocumentNode::Table(node)
            | DocumentNode::Container(node) => {
                for child in &node.children {
                    self.push_visible_text(*child, out);
                }
            }
            DocumentNode::Footnote(node) => {
                for child in &node.children {
                    self.push_visible_text(*child, out);
                }
            }
            DocumentNode::ForcedBreak => out.push('\n'),
            DocumentNode::Image(_) | DocumentNode::Divider | DocumentNode::Unsupported(_) => {}
        }
    }

    fn push_block_text(&self, text: BlockText, out: &mut String) {
        if let Some(value) = self.text_pool.get(text.text) {
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(value.trim());
        }
    }
}

/// Contiguous UTF-8 text arena for chapter text.
#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct TextPool {
    text: String,
}

impl TextPool {
    /// Create an empty text pool.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append text and return its range inside the pool.
    pub fn push(&mut self, text: &str) -> Result<TextRange, PageletError> {
        let start = u32::try_from(self.text.len()).map_err(|_| dom_limit_error(u64::MAX))?;
        self.text.push_str(text);
        let end = u32::try_from(self.text.len()).map_err(|_| dom_limit_error(u64::MAX))?;
        Ok(TextRange { start, end })
    }

    /// Return all pooled text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Return one text range.
    #[must_use]
    pub fn get(&self, range: TextRange) -> Option<&str> {
        self.text
            .get(usize::try_from(range.start).ok()?..usize::try_from(range.end).ok()?)
    }

    /// Return the total UTF-8 byte length.
    #[must_use]
    pub fn len(&self) -> usize {
        self.text.len()
    }

    /// Return true when no text is stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}

/// UTF-8 range inside [`TextPool`].
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct TextRange {
    pub start: u32,
    pub end: u32,
}

impl TextRange {
    /// Return the byte length.
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

/// Arena for semantic document nodes.
#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct NodeArena {
    nodes: Vec<DocumentNode>,
}

impl NodeArena {
    /// Create an empty node arena.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Push one node and return its stable in-chapter identifier.
    pub fn push(&mut self, node: DocumentNode) -> Result<NodeId, PageletError> {
        let index = u32::try_from(self.nodes.len()).map_err(|_| dom_limit_error(u64::MAX))?;
        self.nodes.push(node);
        Ok(NodeId::new(index))
    }

    /// Get one node by id.
    #[must_use]
    pub fn get(&self, id: NodeId) -> Option<&DocumentNode> {
        self.nodes.get(usize::try_from(id.get()).ok()?)
    }

    /// Get one mutable node by id.
    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut DocumentNode> {
        self.nodes.get_mut(usize::try_from(id.get()).ok()?)
    }

    /// Iterate over typed node ids and nodes.
    pub fn iter_with_ids(&self) -> impl Iterator<Item = (NodeId, &DocumentNode)> {
        self.nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| Some((NodeId::new(u32::try_from(index).ok()?), node)))
    }

    /// Number of nodes in the arena.
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Return true when no node is present.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

/// Semantic document node.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum DocumentNode {
    Paragraph(BlockText),
    Heading(HeadingNode),
    List(ListNode),
    ListItem(ListItemNode),
    BlockQuote(ContainerNode),
    Image(ImageNode),
    Figure(ContainerNode),
    Table(ContainerNode),
    Divider,
    ForcedBreak,
    Footnote(FootnoteNode),
    Container(ContainerNode),
    Unsupported(UnsupportedNode),
}

impl DocumentNode {
    /// Return the node's direct children, if any.
    #[must_use]
    pub fn children(&self) -> &[NodeId] {
        match self {
            Self::List(node) => &node.children,
            Self::ListItem(node) => &node.children,
            Self::BlockQuote(node)
            | Self::Figure(node)
            | Self::Table(node)
            | Self::Container(node) => &node.children,
            Self::Footnote(node) => &node.children,
            Self::Unsupported(node) => &node.children,
            Self::Paragraph(_)
            | Self::Heading(_)
            | Self::Image(_)
            | Self::Divider
            | Self::ForcedBreak => &[],
        }
    }

    fn text_range(&self) -> Option<TextRange> {
        match self {
            Self::Paragraph(text) => Some(text.text),
            Self::Heading(node) => Some(node.content.text),
            _ => None,
        }
    }
}

/// Text-bearing block node payload.
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq)]
pub struct BlockText {
    pub text: TextRange,
    pub style: StyleId,
}

/// Heading payload.
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq)]
pub struct HeadingNode {
    pub level: u8,
    pub content: BlockText,
}

/// List payload.
#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct ListNode {
    pub ordered: bool,
    pub children: Vec<NodeId>,
    pub style: StyleId,
}

/// List item payload.
#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct ListItemNode {
    pub children: Vec<NodeId>,
    pub style: StyleId,
}

/// Generic container payload.
#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct ContainerNode {
    pub children: Vec<NodeId>,
    pub style: StyleId,
}

/// Image payload.
#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct ImageNode {
    pub src: Arc<str>,
    pub resolved_path: Option<Arc<str>>,
    pub resource_id: Option<ResourceId>,
    pub alt: Arc<str>,
    pub title: Option<Arc<str>>,
    pub style: StyleId,
}

/// Footnote payload.
#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct FootnoteNode {
    pub note_id: Option<Arc<str>>,
    pub children: Vec<NodeId>,
    pub backlink: Option<LinkTarget>,
    pub style: StyleId,
}

/// Unsupported semantic payload.
#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct UnsupportedNode {
    pub element: Arc<str>,
    pub children: Vec<NodeId>,
    pub style: StyleId,
}

/// One text-bearing block exported for codexia / Book IR consumers.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ChapterBlock {
    pub node_id: NodeId,
    pub order: u32,
    pub block_id: String,
    pub kind: String,
    pub text: String,
    pub fingerprint: BlockFingerprint,
}

/// Return a stable label for a document node's block kind.
#[must_use]
pub fn chapter_block_kind(node: &DocumentNode) -> &'static str {
    match node {
        DocumentNode::Paragraph(_) => "paragraph",
        DocumentNode::Heading(heading) => match heading.level {
            1 => "heading-1",
            2 => "heading-2",
            3 => "heading-3",
            _ => "heading",
        },
        DocumentNode::Footnote(_) => "footnote",
        DocumentNode::Divider => "divider",
        DocumentNode::ForcedBreak => "forced-break",
        _ => "other",
    }
}

/// Anchor index keyed by `resolved-document-href#fragment`.
#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct AnchorIndex {
    pub anchors: BTreeMap<Arc<str>, Anchor>,
}

impl AnchorIndex {
    /// Create an empty anchor index.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert an anchor.
    pub fn insert(&mut self, anchor: Anchor) {
        self.anchors.insert(anchor.key.clone(), anchor);
    }

    /// Return an anchor by fully resolved key.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Anchor> {
        self.anchors.get(key)
    }

    /// Return true when no anchors are present.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.anchors.is_empty()
    }
}

/// One resolved source anchor.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Anchor {
    pub key: Arc<str>,
    pub document_href: Arc<str>,
    pub fragment: Arc<str>,
    pub node_id: NodeId,
    pub source_range: Option<SourceRange>,
}

/// Link target emitted by the parser.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct LinkTarget {
    pub source_node: NodeId,
    pub source_range: Option<SourceRange>,
    pub href: Arc<str>,
    pub resolved_document: Option<Arc<str>>,
    pub fragment: Option<Arc<str>>,
    pub kind: LinkKind,
}

/// Link category.
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum LinkKind {
    Internal,
    External,
    Resource,
    Footnote,
    #[default]
    Unknown,
}

/// Source range map keyed by semantic node.
#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct SourceMap {
    pub ranges: BTreeMap<NodeId, SourceRange>,
}

impl SourceMap {
    /// Create an empty source map.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a source range when it is present.
    pub fn insert(&mut self, node_id: NodeId, source_range: Option<SourceRange>) {
        if let Some(range) = source_range {
            self.ranges.insert(node_id, range);
        }
    }

    /// Return a source range.
    #[must_use]
    pub fn get(&self, node_id: NodeId) -> Option<SourceRange> {
        self.ranges.get(&node_id).copied()
    }
}

/// UTF-16 lookup index for text-bearing nodes.
#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct Utf16Index {
    pub nodes: BTreeMap<NodeId, Utf16NodeIndex>,
}

impl Utf16Index {
    /// Create an empty UTF-16 index.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Build lookup tables for every text-bearing node.
    #[must_use]
    pub fn build(nodes: &NodeArena, text_pool: &TextPool) -> Self {
        let mut index = Self::new();
        for (node_id, node) in nodes.iter_with_ids() {
            if let Some(range) = node.text_range() {
                let Some(text) = text_pool.get(range) else {
                    continue;
                };
                index.nodes.insert(node_id, Utf16NodeIndex::from_text(text));
            }
        }
        index
    }

    /// Convert a UTF-8 byte offset to UTF-16 code units for a text node.
    #[must_use]
    pub fn utf8_to_utf16(&self, node_id: NodeId, utf8_offset: u32) -> Option<u32> {
        self.nodes.get(&node_id)?.utf8_to_utf16(utf8_offset)
    }

    /// Convert a UTF-16 code unit offset to UTF-8 bytes for a text node.
    #[must_use]
    pub fn utf16_to_utf8(&self, node_id: NodeId, utf16_offset: u32) -> Option<u32> {
        self.nodes.get(&node_id)?.utf16_to_utf8(utf16_offset)
    }
}

/// UTF-8/UTF-16 boundaries for one text node.
#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct Utf16NodeIndex {
    pub checkpoints: Vec<Utf16Checkpoint>,
}

impl Utf16NodeIndex {
    /// Build checkpoints at every char boundary.
    #[must_use]
    pub fn from_text(text: &str) -> Self {
        let mut checkpoints = Vec::with_capacity(text.chars().count().saturating_add(1));
        checkpoints.push(Utf16Checkpoint {
            utf8_offset: 0,
            utf16_offset: 0,
        });
        let mut utf16_offset = 0_u32;
        for (_, ch) in text.char_indices() {
            let next_utf8 = checkpoints
                .last()
                .map_or(0_u32, |checkpoint| checkpoint.utf8_offset)
                .saturating_add(u32::try_from(ch.len_utf8()).unwrap_or(u32::MAX));
            checkpoints.push(Utf16Checkpoint {
                utf8_offset: next_utf8,
                utf16_offset: utf16_offset
                    .saturating_add(u32::try_from(ch.len_utf16()).unwrap_or(u32::MAX)),
            });
            utf16_offset =
                utf16_offset.saturating_add(u32::try_from(ch.len_utf16()).unwrap_or(u32::MAX));
        }
        checkpoints.dedup();
        Self { checkpoints }
    }

    fn utf8_to_utf16(&self, utf8_offset: u32) -> Option<u32> {
        self.checkpoints
            .iter()
            .find(|checkpoint| checkpoint.utf8_offset == utf8_offset)
            .map(|checkpoint| checkpoint.utf16_offset)
    }

    fn utf16_to_utf8(&self, utf16_offset: u32) -> Option<u32> {
        self.checkpoints
            .iter()
            .find(|checkpoint| checkpoint.utf16_offset == utf16_offset)
            .map(|checkpoint| checkpoint.utf8_offset)
    }
}

/// One UTF-8/UTF-16 boundary.
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Utf16Checkpoint {
    pub utf8_offset: u32,
    pub utf16_offset: u32,
}

/// Interned computed style table.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct StyleTable {
    styles: Vec<ComputedStyle>,
    by_value: BTreeMap<ComputedStyle, StyleId>,
}

impl StyleTable {
    /// Create a style table with the default style at id 0.
    #[must_use]
    pub fn new() -> Self {
        let mut table = Self {
            styles: Vec::new(),
            by_value: BTreeMap::new(),
        };
        let _ = table.intern(ComputedStyle::default());
        table
    }

    /// Intern a computed style, returning an existing id for equal values.
    pub fn intern(&mut self, style: ComputedStyle) -> Result<StyleId, PageletError> {
        if let Some(id) = self.by_value.get(&style) {
            return Ok(*id);
        }
        let id =
            StyleId::new(u32::try_from(self.styles.len()).map_err(|_| dom_limit_error(u64::MAX))?);
        self.styles.push(style.clone());
        self.by_value.insert(style, id);
        Ok(id)
    }

    /// Get a computed style.
    #[must_use]
    pub fn get(&self, id: StyleId) -> Option<&ComputedStyle> {
        self.styles.get(usize::try_from(id.get()).ok()?)
    }

    /// Number of interned styles.
    #[must_use]
    pub fn len(&self) -> usize {
        self.styles.len()
    }

    /// Return true when no styles are present.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.styles.is_empty()
    }
}

impl Default for StyleTable {
    fn default() -> Self {
        Self::new()
    }
}

/// Supported computed CSS subset represented as stable name/value pairs.
#[derive(Debug, Default, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct ComputedStyle {
    pub properties: BTreeMap<Arc<str>, Arc<str>>,
}

impl ComputedStyle {
    /// Create an empty computed style.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add or replace a property.
    #[must_use]
    pub fn with_property(mut self, name: impl Into<Arc<str>>, value: impl Into<Arc<str>>) -> Self {
        self.properties.insert(name.into(), value.into());
        self
    }
}

fn dom_limit_error(observed: u64) -> PageletError {
    PageletError::ResourceLimitExceeded(ResourceLimitError::new(
        ResourceLimitKind::DomNodes,
        u64::from(u32::MAX),
        observed,
    ))
}

fn push_manifest_json(out: &mut String, items: &[ManifestItem]) {
    indent(out, 1);
    out.push_str("\"manifest\": [\n");
    for (index, item) in items.iter().enumerate() {
        indent(out, 2);
        out.push('{');
        push_inline_json(out, "id", &item.id, true);
        push_inline_json(out, "href", &item.href, true);
        push_inline_json(out, "resolved_path", &item.resolved_path, true);
        push_inline_json(out, "media_type", &item.media_type, false);
        out.push('}');
        if index + 1 < items.len() {
            out.push(',');
        }
        out.push('\n');
    }
    indent(out, 1);
    out.push(']');
}

fn push_spine_json(out: &mut String, items: &[SpineItem]) {
    indent(out, 1);
    out.push_str("\"spine\": [\n");
    for (index, item) in items.iter().enumerate() {
        indent(out, 2);
        out.push('{');
        push_inline_json(out, "idref", &item.idref, true);
        out.push_str("\"linear\": ");
        out.push_str(if item.linear { "true" } else { "false" });
        out.push('}');
        if index + 1 < items.len() {
            out.push(',');
        }
        out.push('\n');
    }
    indent(out, 1);
    out.push(']');
}

fn push_resources_json(out: &mut String, resources: &[ResourceInfo]) {
    indent(out, 1);
    out.push_str("\"resources\": [\n");
    for (index, resource) in resources.iter().enumerate() {
        indent(out, 2);
        out.push('{');
        push_inline_json(out, "path", &resource.path, true);
        push_inline_json(out, "media_type", &resource.media_type, true);
        push_inline_json(out, "kind", resource.kind.as_str(), false);
        out.push('}');
        if index + 1 < resources.len() {
            out.push(',');
        }
        out.push('\n');
    }
    indent(out, 1);
    out.push(']');
}

fn push_anchors_json(out: &mut String, anchors: &AnchorIndex) {
    indent(out, 1);
    out.push_str("\"anchors\": [\n");
    for (index, anchor) in anchors.anchors.values().enumerate() {
        indent(out, 2);
        out.push('{');
        push_inline_json(out, "key", &anchor.key, true);
        push_inline_json(out, "document_href", &anchor.document_href, true);
        push_inline_json(out, "fragment", &anchor.fragment, true);
        out.push_str("\"node_id\": ");
        out.push_str(&anchor.node_id.get().to_string());
        out.push('}');
        if index + 1 < anchors.anchors.len() {
            out.push(',');
        }
        out.push('\n');
    }
    indent(out, 1);
    out.push(']');
}

fn push_links_json(out: &mut String, links: &[LinkTarget]) {
    indent(out, 1);
    out.push_str("\"links\": [\n");
    for (index, link) in links.iter().enumerate() {
        indent(out, 2);
        out.push('{');
        push_inline_json(out, "href", &link.href, true);
        push_inline_json(
            out,
            "resolved_document",
            link.resolved_document.as_deref().unwrap_or(""),
            true,
        );
        push_inline_json(
            out,
            "fragment",
            link.fragment.as_deref().unwrap_or(""),
            false,
        );
        out.push('}');
        if index + 1 < links.len() {
            out.push(',');
        }
        out.push('\n');
    }
    indent(out, 1);
    out.push(']');
}

fn push_json_field(
    out: &mut String,
    level: usize,
    name: &str,
    value: Option<&str>,
    trailing: bool,
) {
    indent(out, level);
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
        out.push(',');
    }
    out.push('\n');
}

fn push_json_string(out: &mut String, level: usize, name: &str, value: &str, trailing: bool) {
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

fn push_json_u32(out: &mut String, level: usize, name: &str, value: u32, trailing: bool) {
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

fn push_json_usize(out: &mut String, level: usize, name: &str, value: usize, trailing: bool) {
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

fn push_inline_json(out: &mut String, name: &str, value: &str, trailing: bool) {
    out.push('"');
    out.push_str(name);
    out.push_str("\": \"");
    out.push_str(&escape_json(value));
    out.push('"');
    if trailing {
        out.push_str(", ");
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

#[allow(dead_code)]
fn diagnostic_code_name(code: DiagnosticCode) -> &'static str {
    match code {
        DiagnosticCode::Io => "io",
        DiagnosticCode::InvalidContainer => "invalid-container",
        DiagnosticCode::InvalidPackage => "invalid-package",
        DiagnosticCode::UnsupportedFeature => "unsupported-feature",
        DiagnosticCode::ResourceLimitExceeded => "resource-limit-exceeded",
        DiagnosticCode::Parse => "parse",
        DiagnosticCode::Layout => "layout",
        DiagnosticCode::Cancelled => "cancelled",
        DiagnosticCode::Protocol => "protocol",
        DiagnosticCode::Internal => "internal",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_table_interns_equal_styles() {
        let mut table = StyleTable::new();
        let style = ComputedStyle::new().with_property("font-weight", "bold");

        let first = table.intern(style.clone()).expect("first style");
        let second = table.intern(style).expect("second style");

        assert_eq!(first, second);
        assert_eq!(table.len(), 2);
    }

    #[test]
    fn all_semantic_node_variants_are_constructible() {
        let text = BlockText {
            text: TextRange { start: 0, end: 5 },
            style: StyleId::new(0),
        };
        let nodes = vec![
            DocumentNode::Paragraph(text),
            DocumentNode::Heading(HeadingNode {
                level: 1,
                content: text,
            }),
            DocumentNode::List(ListNode::default()),
            DocumentNode::ListItem(ListItemNode::default()),
            DocumentNode::BlockQuote(ContainerNode::default()),
            DocumentNode::Image(ImageNode::default()),
            DocumentNode::Figure(ContainerNode::default()),
            DocumentNode::Table(ContainerNode::default()),
            DocumentNode::Divider,
            DocumentNode::ForcedBreak,
            DocumentNode::Footnote(FootnoteNode::default()),
            DocumentNode::Container(ContainerNode::default()),
            DocumentNode::Unsupported(UnsupportedNode::default()),
        ];

        assert_eq!(nodes.len(), 13);
    }

    #[test]
    fn utf16_index_maps_text_boundaries() {
        let mut pool = TextPool::new();
        let range = pool.push("A😀B").expect("text");
        let mut nodes = NodeArena::new();
        let node_id = nodes
            .push(DocumentNode::Paragraph(BlockText {
                text: range,
                style: StyleId::new(0),
            }))
            .expect("node");

        let index = Utf16Index::build(&nodes, &pool);

        assert_eq!(index.utf8_to_utf16(node_id, 0), Some(0));
        assert_eq!(index.utf8_to_utf16(node_id, 1), Some(1));
        assert_eq!(index.utf8_to_utf16(node_id, 5), Some(3));
        assert_eq!(index.utf16_to_utf8(node_id, 4), Some(6));
    }

    #[test]
    fn chapter_visible_text_uses_semantic_order() {
        let mut chapter = ChapterIr::empty(
            DocumentId::new(0),
            "EPUB/chapter.xhtml",
            "Chapter",
            ContentHash::from_bytes(b"chapter"),
        );
        let root = chapter
            .nodes
            .push(DocumentNode::Container(ContainerNode::default()))
            .expect("root");
        chapter.root = root;
        let text = chapter.text_pool.push("Hello").expect("text");
        let paragraph = chapter
            .nodes
            .push(DocumentNode::Paragraph(BlockText {
                text,
                style: StyleId::new(0),
            }))
            .expect("paragraph");
        if let Some(DocumentNode::Container(node)) = chapter.nodes.nodes.get_mut(0) {
            node.children.push(paragraph);
        }

        chapter.rebuild_utf16_index();

        assert_eq!(chapter.visible_text(), "Hello");
        assert!(chapter
            .to_golden_json()
            .contains("\"visible_text\": \"Hello\""));
    }
}
