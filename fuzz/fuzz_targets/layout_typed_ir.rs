#![no_main]

use libfuzzer_sys::fuzz_target;
use pagelet::{
    core::{ContentHash, DocumentId, LayoutUnit, NodeId, StyleId},
    document::{BlockText, ChapterIr, ContainerNode, DocumentNode, TextRange},
    layout::{self, LayoutConstraints, LayoutOptions},
    text::DefaultTextBackend,
};

const MAX_INPUT_LEN: usize = 4096;
const MAX_BLOCKS: usize = 64;

fuzz_target!(|data: &[u8]| {
    if data.is_empty() || data.len() > MAX_INPUT_LEN {
        return;
    }

    let mut chapter = ChapterIr::empty(
        DocumentId::new(1),
        "fuzz.xhtml",
        "fuzz",
        ContentHash::from_bytes(data),
    );
    let root = chapter
        .nodes
        .push(DocumentNode::Container(ContainerNode {
            children: Vec::new(),
            style: StyleId::new(0),
        }))
        .expect("root");
    chapter.root = root;

    let text = String::from_utf8_lossy(data);
    let mut children = Vec::<NodeId>::new();
    for chunk in text.as_bytes().chunks(64).take(MAX_BLOCKS) {
        let value = String::from_utf8_lossy(chunk);
        let range = chapter.text_pool.push(&value).expect("text");
        let node = chapter
            .nodes
            .push(DocumentNode::Paragraph(BlockText {
                text: TextRange {
                    start: range.start,
                    end: range.end,
                },
                style: StyleId::new(0),
            }))
            .expect("paragraph");
        children.push(node);
    }
    if let Some(DocumentNode::Container(container)) = chapter.nodes.get_mut(root) {
        container.children = children;
    }
    chapter.rebuild_utf16_index();

    let backend = DefaultTextBackend::new();
    let options = LayoutOptions {
        constraints: LayoutConstraints::new(LayoutUnit::from_px(120), LayoutUnit::from_px(160))
            .with_margin(LayoutUnit::from_px(8)),
        max_pages: 128,
        ..LayoutOptions::default()
    };

    let Ok(pages) = layout::paginate_chapter_with_options(&chapter, &backend, options) else {
        return;
    };
    let _ = layout::validate_layout_invariants(&chapter, &pages.pages);

    let mut token = None;
    let mut previous = None;
    for _ in 0..128 {
        let Ok(Some(page)) = layout::paginate_next_page(&chapter, &backend, options, token) else {
            break;
        };
        if let Some(encoded) = &previous {
            if page
                .next_break_token
                .as_ref()
                .map(layout::BreakToken::to_wire_string)
                .as_ref()
                == Some(encoded)
            {
                panic!("layout break token did not advance");
            }
        }
        previous = page
            .next_break_token
            .as_ref()
            .map(layout::BreakToken::to_wire_string);
        token = page.next_break_token;
        if token.is_none() {
            break;
        }
    }
});
