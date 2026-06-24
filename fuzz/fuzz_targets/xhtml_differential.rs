#![no_main]

use libfuzzer_sys::fuzz_target;
use pagelet::{
    core::ResourceLimits,
    epub::{parse_xhtml_tree, CompatibilityMode, XhtmlDocument, XhtmlNodeKind},
};

const MAX_INPUT_LEN: usize = 16 * 1024;

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_INPUT_LEN {
        return;
    }

    let input = String::from_utf8_lossy(data);
    let mut limits = ResourceLimits::mobile_defaults();
    limits.max_dom_nodes = 4096;
    limits.max_xml_depth = 64;

    let strict = parse_xhtml_tree(&input, CompatibilityMode::Strict, limits);
    let compatible = parse_xhtml_tree(&input, CompatibilityMode::Compatible, limits);
    if let (Ok(strict), Ok(compatible)) = (strict, compatible) {
        assert_eq!(visible_text(&strict), visible_text(&compatible));
    }
});

fn visible_text(document: &XhtmlDocument) -> String {
    let mut out = String::new();
    collect_text(document, document.root, &mut out);
    out
}

fn collect_text(document: &XhtmlDocument, node_id: usize, out: &mut String) {
    let Some(node) = document.nodes.get(node_id) else {
        return;
    };
    match &node.kind {
        XhtmlNodeKind::Text(text) => out.push_str(text.trim()),
        XhtmlNodeKind::Element(element) => {
            for child in &element.children {
                collect_text(document, *child, out);
            }
        }
    }
}
