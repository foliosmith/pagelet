//! CLI command support boundary.

use std::{fs, path::Path};

use crate::{
    core::{Diagnostic, LayoutUnit, Severity, SourceRange},
    document::{ChapterIr, DocumentNode, LinkKind},
    epub::{
        BookSummary, CapabilityStatus, CompatibilityMode, NavigationItem, NavigationSource,
        OpenOptions,
    },
    layout::{self, HostMeasuredLayout, LayoutConstraints, LayoutOptions},
    text::{DefaultTextBackend, TextBackend},
    wire::{MeasureBatch as WireMeasureBatch, MeasuredBatch as WireMeasuredBatch},
};

/// Inspect EPUB bytes and return a deterministic JSON document.
pub fn inspect_bytes_json(bytes: impl Into<Vec<u8>>) -> Result<String, crate::core::PageletError> {
    inspect_bytes_json_with_options(bytes, OpenOptions::default())
}

/// Inspect an EPUB path and return a deterministic JSON document.
pub fn inspect_path_json(path: impl AsRef<Path>) -> Result<String, crate::core::PageletError> {
    inspect_bytes_json(fs::read(path)?)
}

/// Paginate EPUB bytes and return deterministic PageScene JSON for the first spine item.
pub fn paginate_bytes_json(bytes: impl Into<Vec<u8>>) -> Result<String, crate::core::PageletError> {
    paginate_bytes_json_with_options(bytes, OpenOptions::default(), LayoutOptions::default())
}

/// Paginate an EPUB path and return deterministic PageScene JSON for the first spine item.
pub fn paginate_path_json(path: impl AsRef<Path>) -> Result<String, crate::core::PageletError> {
    paginate_bytes_json(fs::read(path)?)
}

/// Paginate EPUB bytes with explicit EPUB and layout options.
pub fn paginate_bytes_json_with_options(
    bytes: impl Into<Vec<u8>>,
    open_options: OpenOptions,
    layout_options: LayoutOptions,
) -> Result<String, crate::core::PageletError> {
    paginate_spine_item_bytes_json_with_options(bytes, 0, open_options, layout_options)
}

/// Paginate one EPUB spine item and return deterministic PageScene JSON.
pub fn paginate_spine_item_bytes_json(
    bytes: impl Into<Vec<u8>>,
    spine_index: usize,
) -> Result<String, crate::core::PageletError> {
    paginate_spine_item_bytes_json_with_options(
        bytes,
        spine_index,
        OpenOptions::default(),
        LayoutOptions::default(),
    )
}

/// Paginate one EPUB spine item with explicit EPUB and layout options.
pub fn paginate_spine_item_bytes_json_with_options(
    bytes: impl Into<Vec<u8>>,
    spine_index: usize,
    open_options: OpenOptions,
    layout_options: LayoutOptions,
) -> Result<String, crate::core::PageletError> {
    let chapter =
        crate::epub::open_spine_item_chapter_ir_with_options(bytes, spine_index, open_options)?;
    let backend = DefaultTextBackend::new();
    let pages = layout::paginate_chapter_with_options(&chapter, &backend, layout_options)?;
    Ok(paginated_chapter_json(&chapter, &pages))
}

/// Prepare one spine item and encode every host text request as one wire batch.
pub fn prepare_spine_item_measure_batch_with_options(
    bytes: impl Into<Vec<u8>>,
    spine_index: usize,
    open_options: OpenOptions,
    layout_options: LayoutOptions,
) -> Result<Vec<u8>, crate::core::PageletError> {
    let chapter =
        crate::epub::open_spine_item_chapter_ir_with_options(bytes, spine_index, open_options)?;
    let layout = HostMeasuredLayout::prepare(chapter, layout_options);
    WireMeasureBatch::from(layout.measure_batch().clone())
        .encode()
        .map_err(wire_protocol_error)
}

/// Resume one spine item with a host `MeasuredBatch` and return PageScene JSON.
pub fn paginate_spine_item_bytes_json_with_host_measurements(
    bytes: impl Into<Vec<u8>>,
    spine_index: usize,
    open_options: OpenOptions,
    layout_options: LayoutOptions,
    measured_batch: &[u8],
) -> Result<String, crate::core::PageletError> {
    let chapter =
        crate::epub::open_spine_item_chapter_ir_with_options(bytes, spine_index, open_options)?;
    let layout = HostMeasuredLayout::prepare(chapter.clone(), layout_options);
    let measured = WireMeasuredBatch::decode(measured_batch)
        .map_err(wire_protocol_error)?
        .into_text_batch();
    let pages = layout.resume(measured)?;
    Ok(paginated_chapter_json(&chapter, &pages))
}

fn wire_protocol_error(error: crate::wire::WireError) -> crate::core::PageletError {
    crate::core::PageletError::Protocol(crate::core::ProtocolError::new(error.to_string()))
}

/// Paginate EPUB bytes and return debug SVG for the first page.
pub fn paginate_bytes_debug_svg(
    bytes: impl Into<Vec<u8>>,
) -> Result<String, crate::core::PageletError> {
    paginate_bytes_debug_svg_with_options(bytes, OpenOptions::default(), LayoutOptions::default())
}

/// Paginate an EPUB path and return debug SVG for the first page.
pub fn paginate_path_debug_svg(
    path: impl AsRef<Path>,
) -> Result<String, crate::core::PageletError> {
    paginate_bytes_debug_svg(fs::read(path)?)
}

/// Paginate EPUB bytes with explicit options and return debug SVG for the first page.
pub fn paginate_bytes_debug_svg_with_options(
    bytes: impl Into<Vec<u8>>,
    open_options: OpenOptions,
    layout_options: LayoutOptions,
) -> Result<String, crate::core::PageletError> {
    let chapter = crate::epub::open_spine_item_chapter_ir_with_options(bytes, 0, open_options)?;
    let backend = DefaultTextBackend::new();
    let pages = layout::paginate_chapter_with_options(&chapter, &backend, layout_options)?;
    Ok(pages
        .pages
        .first()
        .map(layout::page_debug_svg)
        .unwrap_or_else(|| {
            let page = layout::PageScene {
                page_index: 0,
                size: layout::PageSize {
                    width: layout_options.constraints.viewport_width,
                    height: layout_options.constraints.viewport_height,
                },
                start_anchor: None,
                end_anchor: None,
                text_backend_id: backend.backend_id(),
                font_fingerprint: backend.font_fingerprint(),
                paragraphs: Vec::new(),
                text_paints: Vec::new(),
                fragments: Vec::new(),
                links: Vec::new(),
                anchors: Vec::new(),
                selections: Vec::new(),
                semantics: Vec::new(),
                fingerprint: layout::PageFingerprint(crate::core::ContentHash::from_bytes(&[])),
                next_break_token: None,
                diagnostics: Vec::new(),
            };
            layout::page_debug_svg(&page)
        }))
}

/// Create layout options from whole-pixel viewport dimensions.
#[must_use]
pub fn layout_options_from_px(width: i64, height: i64) -> LayoutOptions {
    LayoutOptions::new(
        LayoutConstraints::new(LayoutUnit::from_px(width), LayoutUnit::from_px(height))
            .with_margin(LayoutUnit::from_px(24)),
    )
}

/// Parse one spine item and return renderable ChapterIR JSON for Web/WASM consumers.
pub fn spine_chapter_ir_json(
    bytes: impl Into<Vec<u8>>,
    spine_index: usize,
) -> Result<String, crate::core::PageletError> {
    spine_chapter_ir_json_with_options(bytes, spine_index, OpenOptions::default())
}

/// Parse one spine item with explicit options and return renderable ChapterIR JSON.
pub fn spine_chapter_ir_json_with_options(
    bytes: impl Into<Vec<u8>>,
    spine_index: usize,
    options: OpenOptions,
) -> Result<String, crate::core::PageletError> {
    let chapter =
        crate::epub::open_spine_item_chapter_ir_with_options(bytes, spine_index, options)?;
    Ok(chapter_ir_json(&chapter))
}

/// Inspect EPUB bytes with explicit open options.
pub fn inspect_bytes_json_with_options(
    bytes: impl Into<Vec<u8>>,
    options: OpenOptions,
) -> Result<String, crate::core::PageletError> {
    let bytes = bytes.into();
    let book = crate::epub::open_book_with_options(bytes.clone(), options)?;
    let chapter = crate::epub::open_spine_item_chapter_ir_with_options(bytes, 0, options).ok();
    Ok(book_summary_json_with_chapter(&book, chapter.as_ref()))
}

/// Serialize a book summary for `pagelet inspect`.
#[must_use]
pub fn book_summary_json(book: &BookSummary) -> String {
    book_summary_json_with_chapter(book, None)
}

/// Serialize renderable ChapterIR JSON for Web/WASM consumers.
#[must_use]
pub fn chapter_ir_json(chapter: &ChapterIr) -> String {
    let mut out = String::new();
    push_chapter_ir_value(&mut out, chapter, 0);
    out.push('\n');
    out
}

fn paginated_chapter_json(chapter: &ChapterIr, pages: &layout::PaginatedDocument) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    push_field(&mut out, 1, "href", &chapter.href, true);
    push_field(&mut out, 1, "title", &chapter.title, true);
    indent(&mut out, 1);
    out.push_str("\"document_id\": ");
    out.push_str(&chapter.document_id.get().to_string());
    out.push_str(",\n");
    indent(&mut out, 1);
    out.push_str("\"page_count\": ");
    out.push_str(&pages.pages.len().to_string());
    out.push_str(",\n");
    let pages_json = pages.to_normalized_json();
    let mut lines = pages_json.lines();
    if let Some(first) = lines.next() {
        indent(&mut out, 1);
        out.push_str("\"pagination\": ");
        out.push_str(first);
        out.push('\n');
    }
    for line in lines {
        indent(&mut out, 1);
        out.push_str(line);
        out.push('\n');
    }
    out.push_str("}\n");
    out
}

/// Serialize a book summary and optional first ChapterIR for `pagelet inspect`.
#[must_use]
pub fn book_summary_json_with_chapter(book: &BookSummary, chapter: Option<&ChapterIr>) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    push_field(&mut out, 1, "rootfile", &book.package.rootfile_path, true);
    push_field(
        &mut out,
        1,
        "package_version",
        &book.package.metadata.package_version,
        true,
    );
    push_field_opt(
        &mut out,
        1,
        "identifier",
        book.package.metadata.identifier.as_deref(),
        true,
    );
    push_field_opt(
        &mut out,
        1,
        "title",
        book.package.metadata.title.as_deref(),
        true,
    );
    push_field_opt(
        &mut out,
        1,
        "language",
        book.package.metadata.language.as_deref(),
        true,
    );

    indent(&mut out, 1);
    out.push_str("\"manifest\": [\n");
    for (index, item) in book.package.manifest.iter().enumerate() {
        indent(&mut out, 2);
        out.push('{');
        push_inline_field(&mut out, "id", &item.id, true);
        push_inline_field(&mut out, "href", &item.href, true);
        push_inline_field(&mut out, "resolved_path", &item.resolved_path, true);
        push_inline_field(&mut out, "media_type", &item.media_type, false);
        out.push('}');
        if index + 1 < book.package.manifest.len() {
            out.push(',');
        }
        out.push('\n');
    }
    indent(&mut out, 1);
    out.push_str("],\n");

    indent(&mut out, 1);
    out.push_str("\"spine\": [\n");
    for (index, item) in book.package.spine.iter().enumerate() {
        indent(&mut out, 2);
        out.push('{');
        push_inline_field(&mut out, "idref", &item.idref, true);
        out.push_str("\"linear\": ");
        out.push_str(if item.linear { "true" } else { "false" });
        out.push('}');
        if index + 1 < book.package.spine.len() {
            out.push(',');
        }
        out.push('\n');
    }
    indent(&mut out, 1);
    out.push_str("],\n");

    indent(&mut out, 1);
    out.push_str("\"navigation\": {\n");
    push_field(
        &mut out,
        2,
        "source",
        navigation_source_name(book.navigation.source),
        true,
    );
    push_nav_array(&mut out, "toc", &book.navigation.toc, true);
    push_nav_array(&mut out, "page_list", &book.navigation.page_list, true);
    push_nav_array(&mut out, "landmarks", &book.navigation.landmarks, false);
    indent(&mut out, 1);
    out.push_str("},\n");

    push_chapter_ir(&mut out, chapter);
    out.push_str(",\n");
    push_diagnostics(&mut out, &book.diagnostics);
    out.push_str(",\n");
    push_capabilities(&mut out, book);
    out.push('\n');
    out.push_str("}\n");
    out
}

fn push_diagnostics(out: &mut String, diagnostics: &[Diagnostic]) {
    indent(out, 1);
    out.push_str("\"diagnostics\": [\n");
    for (index, diagnostic) in diagnostics.iter().enumerate() {
        indent(out, 2);
        out.push('{');
        push_inline_field(out, "code", &format!("{:?}", diagnostic.code), true);
        push_inline_field(out, "severity", severity_name(diagnostic.severity), true);
        push_inline_field(out, "message", &diagnostic.message, false);
        out.push('}');
        if index + 1 < diagnostics.len() {
            out.push(',');
        }
        out.push('\n');
    }
    indent(out, 1);
    out.push(']');
}

fn push_chapter_ir(out: &mut String, chapter: Option<&ChapterIr>) {
    indent(out, 1);
    out.push_str("\"chapter_ir\": ");
    let Some(chapter) = chapter else {
        out.push_str("null");
        return;
    };
    push_chapter_ir_value(out, chapter, 1);
}

fn push_chapter_ir_value(out: &mut String, chapter: &ChapterIr, level: usize) {
    out.push_str("{\n");
    push_field(out, level + 1, "href", &chapter.href, true);
    push_field(out, level + 1, "title", &chapter.title, true);
    indent(out, level + 1);
    out.push_str("\"document_id\": ");
    out.push_str(&chapter.document_id.get().to_string());
    out.push_str(",\n");
    indent(out, level + 1);
    out.push_str("\"root\": ");
    out.push_str(&chapter.root.get().to_string());
    out.push_str(",\n");
    indent(out, level + 1);
    out.push_str("\"node_count\": ");
    out.push_str(&chapter.nodes.len().to_string());
    out.push_str(",\n");
    push_field(
        out,
        level + 1,
        "visible_text",
        &chapter.visible_text(),
        true,
    );

    indent(out, level + 1);
    out.push_str("\"nodes\": [\n");
    for (index, (node_id, node)) in chapter.nodes.iter_with_ids().enumerate() {
        push_chapter_node_json(out, chapter, node_id, node, level + 2);
        if index + 1 < chapter.nodes.len() {
            out.push(',');
        }
        out.push('\n');
    }
    indent(out, level + 1);
    out.push_str("],\n");

    indent(out, level + 1);
    out.push_str("\"blocks\": [\n");
    let blocks = chapter.blocks();
    for (index, block) in blocks.iter().enumerate() {
        indent(out, level + 2);
        out.push('{');
        let mut first = true;
        push_json_u32_prop(out, "node_id", block.node_id.get(), &mut first);
        push_json_u32_prop(out, "order", block.order, &mut first);
        push_json_str_prop(out, "block_id", &block.block_id, &mut first);
        push_json_str_prop(out, "kind", &block.kind, &mut first);
        push_json_str_prop(out, "text", &block.text, &mut first);
        out.push('}');
        if index + 1 < blocks.len() {
            out.push(',');
        }
        out.push('\n');
    }
    indent(out, level + 1);
    out.push_str("],\n");

    indent(out, level + 1);
    out.push_str("\"anchors\": [\n");
    for (index, anchor) in chapter.anchors.anchors.values().enumerate() {
        indent(out, level + 2);
        out.push('{');
        let mut first = true;
        push_json_str_prop(out, "key", &anchor.key, &mut first);
        push_json_str_prop(out, "document_href", &anchor.document_href, &mut first);
        push_json_str_prop(out, "fragment", &anchor.fragment, &mut first);
        push_json_u32_prop(out, "node_id", anchor.node_id.get(), &mut first);
        push_json_u32_prop(out, "utf8_byte_offset", anchor.utf8_byte_offset, &mut first);
        push_json_source_range_prop(out, "source_range", anchor.source_range, &mut first);
        out.push('}');
        if index + 1 < chapter.anchors.anchors.len() {
            out.push(',');
        }
        out.push('\n');
    }
    indent(out, level + 1);
    out.push_str("],\n");

    indent(out, level + 1);
    out.push_str("\"links\": [\n");
    for (index, link) in chapter.links.iter().enumerate() {
        indent(out, level + 2);
        out.push('{');
        let mut first = true;
        push_json_u32_prop(out, "source_node", link.source_node.get(), &mut first);
        push_json_str_prop(out, "href", &link.href, &mut first);
        push_json_opt_str_prop(
            out,
            "resolved_document",
            link.resolved_document.as_deref(),
            &mut first,
        );
        push_json_opt_str_prop(out, "fragment", link.fragment.as_deref(), &mut first);
        push_json_str_prop(out, "kind", link_kind_name(link.kind), &mut first);
        push_json_text_range_prop(out, "text_range", link.text_range.as_ref(), &mut first);
        push_json_source_range_prop(out, "source_range", link.source_range, &mut first);
        out.push('}');
        if index + 1 < chapter.links.len() {
            out.push(',');
        }
        out.push('\n');
    }
    indent(out, level + 1);
    out.push_str("]\n");
    indent(out, level);
    out.push('}');
}

fn push_chapter_node_json(
    out: &mut String,
    chapter: &ChapterIr,
    node_id: crate::core::NodeId,
    node: &DocumentNode,
    level: usize,
) {
    indent(out, level);
    out.push('{');
    let mut first = true;
    push_json_u32_prop(out, "id", node_id.get(), &mut first);
    push_json_str_prop(out, "kind", document_node_kind(node), &mut first);
    push_json_source_range_prop(
        out,
        "source_range",
        chapter.source_map.get(node_id),
        &mut first,
    );

    match node {
        DocumentNode::Paragraph(text) => {
            push_json_str_prop(out, "text", block_text(chapter, text), &mut first);
            push_json_u32_prop(out, "style", text.style.get(), &mut first);
            push_json_inline_style_runs_prop(out, &text.style_runs, &mut first);
        }
        DocumentNode::Heading(heading) => {
            push_json_u8_prop(out, "level", heading.level, &mut first);
            push_json_str_prop(
                out,
                "text",
                block_text(chapter, &heading.content),
                &mut first,
            );
            push_json_u32_prop(out, "style", heading.content.style.get(), &mut first);
            push_json_inline_style_runs_prop(out, &heading.content.style_runs, &mut first);
        }
        DocumentNode::List(list) => {
            push_json_bool_prop(out, "ordered", list.ordered, &mut first);
            push_json_children_prop(out, &list.children, &mut first);
            push_json_u32_prop(out, "style", list.style.get(), &mut first);
        }
        DocumentNode::ListItem(item) => {
            push_json_children_prop(out, &item.children, &mut first);
            push_json_u32_prop(out, "style", item.style.get(), &mut first);
        }
        DocumentNode::BlockQuote(container)
        | DocumentNode::Figure(container)
        | DocumentNode::Table(container)
        | DocumentNode::Container(container) => {
            push_json_children_prop(out, &container.children, &mut first);
            push_json_u32_prop(out, "style", container.style.get(), &mut first);
        }
        DocumentNode::Image(image) => {
            push_json_str_prop(out, "src", &image.src, &mut first);
            push_json_opt_str_prop(
                out,
                "resolved_path",
                image.resolved_path.as_deref(),
                &mut first,
            );
            push_json_opt_u32_prop(
                out,
                "resource_id",
                image.resource_id.map(|resource| resource.get()),
                &mut first,
            );
            push_json_str_prop(out, "alt", &image.alt, &mut first);
            push_json_opt_str_prop(out, "title", image.title.as_deref(), &mut first);
            push_json_u32_prop(out, "style", image.style.get(), &mut first);
        }
        DocumentNode::Footnote(footnote) => {
            push_json_opt_str_prop(out, "note_id", footnote.note_id.as_deref(), &mut first);
            push_json_children_prop(out, &footnote.children, &mut first);
            push_json_u32_prop(out, "style", footnote.style.get(), &mut first);
        }
        DocumentNode::Unsupported(unsupported) => {
            push_json_str_prop(out, "element", &unsupported.element, &mut first);
            push_json_children_prop(out, &unsupported.children, &mut first);
            push_json_u32_prop(out, "style", unsupported.style.get(), &mut first);
        }
        DocumentNode::Divider | DocumentNode::ForcedBreak => {}
    }
    out.push('}');
}

fn block_text<'a>(chapter: &'a ChapterIr, text: &crate::document::BlockText) -> &'a str {
    chapter.text_pool.get(text.text).unwrap_or("")
}

fn push_json_inline_style_runs_prop(
    out: &mut String,
    runs: &[crate::document::InlineStyleRun],
    first: &mut bool,
) {
    push_json_prop_name(out, "style_runs", first);
    out.push('[');
    for (index, run) in runs.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        out.push('{');
        let mut first_run_field = true;
        push_json_u32_prop(out, "start", run.start, &mut first_run_field);
        push_json_u32_prop(out, "end", run.end, &mut first_run_field);
        push_json_u32_prop(out, "style", run.style.get(), &mut first_run_field);
        push_json_source_range_prop(out, "source_range", run.source_range, &mut first_run_field);
        out.push('}');
    }
    out.push(']');
}

fn push_json_prop_name(out: &mut String, name: &str, first: &mut bool) {
    if !*first {
        out.push_str(", ");
    }
    *first = false;
    out.push('"');
    out.push_str(name);
    out.push_str("\": ");
}

fn push_json_str_prop(out: &mut String, name: &str, value: &str, first: &mut bool) {
    push_json_prop_name(out, name, first);
    out.push('"');
    out.push_str(&escape_json(value));
    out.push('"');
}

fn push_json_opt_str_prop(out: &mut String, name: &str, value: Option<&str>, first: &mut bool) {
    push_json_prop_name(out, name, first);
    if let Some(value) = value {
        out.push('"');
        out.push_str(&escape_json(value));
        out.push('"');
    } else {
        out.push_str("null");
    }
}

fn push_json_u32_prop(out: &mut String, name: &str, value: u32, first: &mut bool) {
    push_json_prop_name(out, name, first);
    out.push_str(&value.to_string());
}

fn push_json_opt_u32_prop(out: &mut String, name: &str, value: Option<u32>, first: &mut bool) {
    push_json_prop_name(out, name, first);
    if let Some(value) = value {
        out.push_str(&value.to_string());
    } else {
        out.push_str("null");
    }
}

fn push_json_u8_prop(out: &mut String, name: &str, value: u8, first: &mut bool) {
    push_json_prop_name(out, name, first);
    out.push_str(&value.to_string());
}

fn push_json_bool_prop(out: &mut String, name: &str, value: bool, first: &mut bool) {
    push_json_prop_name(out, name, first);
    out.push_str(if value { "true" } else { "false" });
}

fn push_json_children_prop(out: &mut String, children: &[crate::core::NodeId], first: &mut bool) {
    push_json_prop_name(out, "children", first);
    out.push('[');
    for (index, child) in children.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        out.push_str(&child.get().to_string());
    }
    out.push(']');
}

fn push_json_source_range_prop(
    out: &mut String,
    name: &str,
    range: Option<SourceRange>,
    first: &mut bool,
) {
    push_json_prop_name(out, name, first);
    if let Some(range) = range {
        out.push('{');
        out.push_str("\"start\": ");
        out.push_str(&range.start.to_string());
        out.push_str(", \"end\": ");
        out.push_str(&range.end.to_string());
        out.push('}');
    } else {
        out.push_str("null");
    }
}

fn push_json_text_range_prop(
    out: &mut String,
    name: &str,
    range: Option<&std::ops::Range<u32>>,
    first: &mut bool,
) {
    push_json_prop_name(out, name, first);
    if let Some(range) = range {
        out.push('{');
        out.push_str("\"start\": ");
        out.push_str(&range.start.to_string());
        out.push_str(", \"end\": ");
        out.push_str(&range.end.to_string());
        out.push('}');
    } else {
        out.push_str("null");
    }
}

fn push_capabilities(out: &mut String, book: &BookSummary) {
    indent(out, 1);
    out.push_str("\"capability_report\": {\n");
    push_field(
        out,
        2,
        "mode",
        compatibility_mode_name(book.capability_report.mode),
        true,
    );
    indent(out, 2);
    out.push_str("\"capabilities\": [\n");
    for (index, capability) in book.capability_report.capabilities.iter().enumerate() {
        indent(out, 3);
        out.push('{');
        push_inline_field(out, "feature", &capability.feature, true);
        push_inline_field(
            out,
            "status",
            capability_status_name(capability.status),
            true,
        );
        push_inline_field(out, "message", &capability.message, false);
        out.push('}');
        if index + 1 < book.capability_report.capabilities.len() {
            out.push(',');
        }
        out.push('\n');
    }
    indent(out, 2);
    out.push_str("]\n");
    indent(out, 1);
    out.push('}');
}

fn document_node_kind(node: &DocumentNode) -> &'static str {
    match node {
        DocumentNode::Paragraph(_) => "paragraph",
        DocumentNode::Heading(_) => "heading",
        DocumentNode::List(_) => "list",
        DocumentNode::ListItem(_) => "list-item",
        DocumentNode::BlockQuote(_) => "blockquote",
        DocumentNode::Image(_) => "image",
        DocumentNode::Figure(_) => "figure",
        DocumentNode::Table(_) => "table",
        DocumentNode::Divider => "divider",
        DocumentNode::ForcedBreak => "forced-break",
        DocumentNode::Footnote(_) => "footnote",
        DocumentNode::Container(_) => "container",
        DocumentNode::Unsupported(_) => "unsupported",
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

fn push_nav_array(out: &mut String, name: &str, items: &[NavigationItem], trailing: bool) {
    indent(out, 2);
    out.push('"');
    out.push_str(name);
    out.push_str("\": [\n");
    for (index, item) in items.iter().enumerate() {
        indent(out, 3);
        out.push('{');
        push_inline_field(out, "label", &item.label, true);
        push_inline_field(out, "href", &item.href, false);
        out.push('}');
        if index + 1 < items.len() {
            out.push(',');
        }
        out.push('\n');
    }
    indent(out, 2);
    out.push(']');
    if trailing {
        out.push(',');
    }
    out.push('\n');
}

fn push_field(out: &mut String, level: usize, name: &str, value: &str, trailing: bool) {
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

fn push_field_opt(out: &mut String, level: usize, name: &str, value: Option<&str>, trailing: bool) {
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

fn push_inline_field(out: &mut String, name: &str, value: &str, trailing: bool) {
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

const fn navigation_source_name(source: NavigationSource) -> &'static str {
    match source {
        NavigationSource::Epub3Nav => "epub3-nav",
        NavigationSource::Ncx => "ncx",
        NavigationSource::Guide => "guide",
        NavigationSource::Spine => "spine",
    }
}

const fn compatibility_mode_name(mode: CompatibilityMode) -> &'static str {
    match mode {
        CompatibilityMode::Strict => "strict",
        CompatibilityMode::Compatible => "compatible",
        CompatibilityMode::Salvage => "salvage",
    }
}

const fn capability_status_name(status: CapabilityStatus) -> &'static str {
    match status {
        CapabilityStatus::Supported => "supported",
        CapabilityStatus::SupportedWithLimitations => "supported-with-limitations",
        CapabilityStatus::UnsupportedDiagnosed => "unsupported-diagnosed",
    }
}

const fn severity_name(severity: Severity) -> &'static str {
    match severity {
        Severity::Info => "info",
        Severity::Warning => "warning",
        Severity::Error => "error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspect_json_contains_m1_sections() {
        let fixture =
            crate::testkit::GeneratedEpubFixture::preset(crate::testkit::FixtureKind::MinimalEpub3);
        let json = inspect_bytes_json(fixture.bytes().to_vec()).expect("inspect");

        assert!(json.contains(r#""manifest""#));
        assert!(json.contains(r#""spine""#));
        assert!(json.contains(r#""navigation""#));
        assert!(json.contains(r#""chapter_ir""#));
        assert!(json.contains(r#""visible_text": "Hello pagelet.""#));
        assert!(json.contains(r#""capability_report""#));
    }
}
