use std::{env, fs, path::PathBuf, sync::Arc};

use pagelet::{
    core::{ContentHash, DocumentId, LayoutUnit, NodeId, SourceRange, TextAffinity, TextAnchor},
    document::LinkKind,
    layout::{
        LinkRegion, PageFingerprint, PageScene, PageSize, Point, Rect, SceneFragment,
        SceneFragmentKind, SceneParagraph, SceneParagraphLine, TextAnchorRange, TextPaintFragment,
    },
    text::{
        FontFallbackChain, FontSetFingerprint, HeightBehavior, LineMetrics, StrutStyle,
        TextBackendId, TextBounds, TextCluster, TextDirection, TextStyleRun,
    },
    wire::PageBatch,
};

fn main() {
    let path = PathBuf::from(env::args_os().nth(1).expect("output path"));
    fs::write(
        path,
        PageBatch::new(vec![fixture()])
            .encode()
            .expect("encode page"),
    )
    .expect("write page fixture");
}

fn fixture() -> PageScene {
    let text: Arc<str> = Arc::from("Hello 中🙂");
    let text_end = u32::try_from(text.len()).expect("text length");
    let start = TextAnchor::new(
        DocumentId::new(1),
        NodeId::new(7),
        0,
        TextAffinity::Downstream,
    );
    let end = TextAnchor::new(
        DocumentId::new(1),
        NodeId::new(7),
        text_end,
        TextAffinity::Upstream,
    );
    let mut style = TextStyleRun::new(
        0,
        text_end,
        LayoutUnit::from_px(16),
        FontFallbackChain::default(),
    );
    style.letter_spacing = LayoutUnit::from_raw(4);
    let ink = Rect {
        x: LayoutUnit::from_raw(-2),
        y: LayoutUnit::from_raw(-3),
        width: LayoutUnit::from_px(200),
        height: LayoutUnit::from_px(20),
    };
    let paragraph = SceneParagraph {
        paragraph_id: 77,
        request_fingerprint: 0x0102_0304,
        measurement_fingerprint: 0x0506_0708,
        text,
        text_range: 0..text_end,
        style_runs: vec![style],
        font_size: LayoutUnit::from_px(16),
        available_width: LayoutUnit::from_px(300),
        max_width: LayoutUnit::from_px(300),
        locale: Arc::from("en-US"),
        direction: TextDirection::Ltr,
        text_scale: LayoutUnit::from_raw(LayoutUnit::SCALE),
        font_candidates: FontFallbackChain::default(),
        strut: StrutStyle::default(),
        height_behavior: HeightBehavior::Natural,
        lines: vec![SceneParagraphLine {
            metrics: LineMetrics {
                text_start: 0,
                text_end,
                baseline: LayoutUnit::from_px(15),
                ascent: LayoutUnit::from_px(12),
                descent: LayoutUnit::from_px(4),
                line_height: LayoutUnit::from_px(20),
                width: LayoutUnit::from_px(200),
                ink_bounds: TextBounds {
                    x: ink.x,
                    y: ink.y,
                    width: ink.width,
                    height: ink.height,
                },
                hard_break: false,
            },
            layout_rect: Rect {
                x: LayoutUnit::ZERO,
                y: LayoutUnit::ZERO,
                width: LayoutUnit::from_px(200),
                height: LayoutUnit::from_px(20),
            },
            ink_bounds: ink,
        }],
        clusters: vec![TextCluster {
            text_start: 0,
            text_end,
            line_index: 0,
            x_start: LayoutUnit::ZERO,
            x_end: LayoutUnit::from_px(200),
        }],
    };
    let anchor_range = TextAnchorRange { start, end };
    PageScene {
        page_index: 2,
        size: PageSize {
            width: LayoutUnit::from_px(320),
            height: LayoutUnit::from_px(480),
        },
        start_anchor: Some(start),
        end_anchor: Some(end),
        text_backend_id: TextBackendId(9),
        font_fingerprint: FontSetFingerprint(10),
        paragraphs: vec![paragraph],
        text_paints: vec![TextPaintFragment {
            id: 88,
            node_id: NodeId::new(7),
            paragraph_id: 77,
            visible_text_range: 0..text_end,
            paint_origin: Point {
                x: LayoutUnit::from_px(12),
                y: LayoutUnit::from_px(-4),
            },
            layout_rect: Rect {
                x: LayoutUnit::from_px(12),
                y: LayoutUnit::from_px(8),
                width: LayoutUnit::from_px(200),
                height: LayoutUnit::from_px(20),
            },
            clip_rect: Rect {
                x: LayoutUnit::ZERO,
                y: LayoutUnit::from_px(8),
                width: LayoutUnit::from_px(320),
                height: LayoutUnit::from_px(100),
            },
            first_line: 0,
            line_count: 1,
            source_range: Some(SourceRange::new(10, 30).expect("source range")),
            anchor_range,
            overflow: false,
        }],
        fragments: vec![SceneFragment {
            id: 89,
            kind: SceneFragmentKind::Image,
            node_id: NodeId::new(8),
            rect: Rect {
                x: LayoutUnit::from_px(20),
                y: LayoutUnit::from_px(40),
                width: LayoutUnit::from_px(80),
                height: LayoutUnit::from_px(60),
            },
            text: None,
            source_range: Some(SourceRange::new(31, 40).expect("image source range")),
            anchor_range: None,
            line_index: None,
            overflow: false,
        }],
        links: vec![LinkRegion {
            rect: Rect {
                x: LayoutUnit::from_px(12),
                y: LayoutUnit::from_px(8),
                width: LayoutUnit::from_px(50),
                height: LayoutUnit::from_px(20),
            },
            node_id: NodeId::new(7),
            text_range: Some(0..5),
            href: Arc::from("chapter-2.xhtml#target"),
            resolved_document: Some(Arc::from("OPS/chapter-2.xhtml")),
            fragment: Some(Arc::from("target")),
            kind: LinkKind::Internal,
        }],
        anchors: Vec::new(),
        selections: Vec::new(),
        semantics: Vec::new(),
        fingerprint: PageFingerprint(ContentHash::from_bytes(b"flutter-page-scene")),
        next_break_token: None,
        diagnostics: Vec::new(),
    }
}
