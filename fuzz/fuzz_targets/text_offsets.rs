#![no_main]

use libfuzzer_sys::fuzz_target;
use pagelet::{
    core::{ContentHash, DocumentId, StyleId},
    document::{BlockText, ChapterIr, DocumentNode, TextRange},
};

const MAX_INPUT_LEN: usize = 16 * 1024;

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_INPUT_LEN {
        return;
    }

    let text = String::from_utf8_lossy(data);
    let mut chapter = ChapterIr::empty(
        DocumentId::new(1),
        "offsets.xhtml",
        "offsets",
        ContentHash::from_bytes(data),
    );
    let range = chapter.text_pool.push(&text).expect("bounded text");
    let node = chapter
        .nodes
        .push(DocumentNode::Paragraph(BlockText {
            text: TextRange {
                start: range.start,
                end: range.end,
            },
            style_runs: Vec::new(),
            style: StyleId::new(0),
        }))
        .expect("single paragraph");
    chapter.root = node;
    chapter.rebuild_utf16_index();

    for utf8 in text
        .char_indices()
        .map(|(offset, _)| offset)
        .chain(std::iter::once(text.len()))
    {
        let utf8 = u32::try_from(utf8).expect("bounded offset");
        let utf16 = chapter
            .utf16_index
            .utf8_to_utf16(node, utf8)
            .expect("UTF-8 character boundary");
        assert_eq!(chapter.utf16_index.utf16_to_utf8(node, utf16), Some(utf8));
    }
});
