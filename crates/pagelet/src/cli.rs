//! CLI command support boundary.

use std::{fs, path::Path};

use crate::{
    core::{Diagnostic, Severity},
    document::{ChapterIr, DocumentNode},
    epub::{
        BookSummary, CapabilityStatus, CompatibilityMode, NavigationItem, NavigationSource,
        OpenOptions,
    },
};

/// Inspect EPUB bytes and return a deterministic JSON document.
pub fn inspect_bytes_json(bytes: impl Into<Vec<u8>>) -> Result<String, crate::core::PageletError> {
    inspect_bytes_json_with_options(bytes, OpenOptions::default())
}

/// Inspect an EPUB path and return a deterministic JSON document.
pub fn inspect_path_json(path: impl AsRef<Path>) -> Result<String, crate::core::PageletError> {
    inspect_bytes_json(fs::read(path)?)
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
        out.push_str("{");
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
        out.push_str("{");
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
        out.push_str("{");
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
    out.push_str("{\n");
    push_field(out, 2, "href", &chapter.href, true);
    push_field(out, 2, "title", &chapter.title, true);
    indent(out, 2);
    out.push_str("\"document_id\": ");
    out.push_str(&chapter.document_id.get().to_string());
    out.push_str(",\n");
    indent(out, 2);
    out.push_str("\"root\": ");
    out.push_str(&chapter.root.get().to_string());
    out.push_str(",\n");
    indent(out, 2);
    out.push_str("\"node_count\": ");
    out.push_str(&chapter.nodes.len().to_string());
    out.push_str(",\n");
    push_field(out, 2, "visible_text", &chapter.visible_text(), true);

    indent(out, 2);
    out.push_str("\"nodes\": [\n");
    for (index, (node_id, node)) in chapter.nodes.iter_with_ids().enumerate() {
        indent(out, 3);
        out.push('{');
        out.push_str("\"id\": ");
        out.push_str(&node_id.get().to_string());
        out.push_str(", ");
        push_inline_field(out, "kind", document_node_kind(node), false);
        out.push('}');
        if index + 1 < chapter.nodes.len() {
            out.push(',');
        }
        out.push('\n');
    }
    indent(out, 2);
    out.push_str("],\n");

    indent(out, 2);
    out.push_str("\"anchors\": [\n");
    for (index, anchor) in chapter.anchors.anchors.values().enumerate() {
        indent(out, 3);
        out.push('{');
        push_inline_field(out, "key", &anchor.key, true);
        out.push_str("\"node_id\": ");
        out.push_str(&anchor.node_id.get().to_string());
        out.push('}');
        if index + 1 < chapter.anchors.anchors.len() {
            out.push(',');
        }
        out.push('\n');
    }
    indent(out, 2);
    out.push_str("],\n");

    indent(out, 2);
    out.push_str("\"links\": [\n");
    for (index, link) in chapter.links.iter().enumerate() {
        indent(out, 3);
        out.push('{');
        push_inline_field(out, "href", &link.href, true);
        push_inline_field(
            out,
            "resolved_document",
            link.resolved_document.as_deref().unwrap_or(""),
            true,
        );
        push_inline_field(
            out,
            "fragment",
            link.fragment.as_deref().unwrap_or(""),
            false,
        );
        out.push('}');
        if index + 1 < chapter.links.len() {
            out.push(',');
        }
        out.push('\n');
    }
    indent(out, 2);
    out.push_str("]\n");
    indent(out, 1);
    out.push('}');
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
        out.push_str("{");
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
