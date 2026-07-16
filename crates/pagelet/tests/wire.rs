use std::sync::Arc;

use pagelet::{
    core::{
        ContentHash, Diagnostic, DiagnosticCode, DocumentId, FontId, LayoutUnit, NodeId,
        ResourceId, Severity, SourceRange, TextAffinity, TextAnchor,
    },
    document::LinkKind,
    layout::{
        AnchorRegion, BreakToken, LinkRegion, PageFingerprint, PageScene, PageSize, Rect,
        SceneFragment, SceneFragmentKind, SelectionMap, SemanticNode, TextAnchorRange,
    },
    text::{
        FontDescriptor, FontFallbackChain, FontSetFingerprint, FontStyle, HeightBehavior,
        MeasureRequest, StrutStyle, TextBackendId, TextDirection, TextStyleRun,
    },
    wire::{MeasureBatch, PageBatch, SchemaVersion, WireError, CURRENT_SCHEMA_VERSION},
};

const HEADER_LEN: usize = 20;

#[test]
fn schema_version_is_independent_from_crate_semver() {
    assert_eq!(CURRENT_SCHEMA_VERSION, SchemaVersion::new(1));
    assert_eq!(CURRENT_SCHEMA_VERSION.get(), 1);
    assert_ne!(
        CURRENT_SCHEMA_VERSION.get().to_string(),
        env!("CARGO_PKG_VERSION")
    );
}

#[test]
fn measure_batch_round_trip_preserves_all_fields_and_is_canonical() {
    let batch = rich_measure_batch();

    let encoded = batch.encode().expect("encode measure batch");
    assert_envelope(&encoded, 2);
    let decoded = MeasureBatch::decode(&encoded).expect("decode measure batch");

    assert_eq!(decoded, batch);
    assert_eq!(decoded.encode().expect("re-encode measure batch"), encoded);
    assert_eq!(decoded.clone().into_text_batch().requests, decoded.requests);
}

#[test]
fn page_batch_round_trip_preserves_every_page_scene_field_and_is_canonical() {
    let batch = PageBatch::new(vec![rich_page_scene()]);

    let encoded = batch.encode().expect("encode page batch");
    assert_envelope(&encoded, 1);
    let decoded = PageBatch::decode(&encoded).expect("decode page batch");

    assert_eq!(decoded, batch);
    assert_eq!(decoded.encode().expect("re-encode page batch"), encoded);
}

#[test]
fn empty_page_batch_has_a_fixed_v1_encoding() {
    let encoded = PageBatch::new(Vec::new())
        .encode()
        .expect("encode empty page batch");

    assert_eq!(
        encoded,
        [
            0x50, 0x47, 0x4c, 0x54, 0x53, 0x43, 0x4e, 0x00, // magic
            0x01, 0x00, // schema v1
            0x01, 0x00, // PageBatch
            0x04, 0x00, 0x00, 0x00, // payload length
            0x1c, 0xdf, 0x44, 0x21, // CRC-32 of four zero bytes
            0x00, 0x00, 0x00, 0x00, // zero pages
        ]
    );
}

#[test]
fn empty_batches_and_fixed_point_boundaries_round_trip() {
    let pages = PageBatch::new(Vec::new());
    assert_eq!(
        PageBatch::decode(&pages.encode().expect("encode empty pages"))
            .expect("decode empty pages"),
        pages
    );

    let mut request = MeasureRequest::new(
        0,
        "",
        LayoutUnit::from_raw(i64::MIN),
        LayoutUnit::from_raw(i64::MAX),
    );
    request.style_runs.clear();
    request.available_width = LayoutUnit::from_raw(i64::MIN);
    request.text_scale = LayoutUnit::from_raw(i64::MAX);
    request.strut = StrutStyle {
        ascent: LayoutUnit::from_raw(i64::MIN),
        descent: LayoutUnit::ZERO,
        leading: LayoutUnit::from_raw(i64::MAX),
    };
    let measurements = MeasureBatch::new(vec![request]);
    assert_eq!(
        MeasureBatch::decode(&measurements.encode().expect("encode boundary measurements"))
            .expect("decode boundary measurements"),
        measurements
    );
}

#[test]
fn decoder_rejects_unknown_schema_version_and_payload_kind() {
    let mut pages = PageBatch::new(Vec::new())
        .encode()
        .expect("encode empty page batch");
    pages[8..10].copy_from_slice(&2_u16.to_le_bytes());
    assert_eq!(
        PageBatch::decode(&pages),
        Err(WireError::UnsupportedVersion {
            expected: 1,
            actual: 2,
        })
    );

    let measurements = MeasureBatch::new(Vec::new())
        .encode()
        .expect("encode empty measure batch");
    assert_eq!(
        PageBatch::decode(&measurements),
        Err(WireError::UnexpectedPayloadKind {
            expected: 1,
            actual: 2,
        })
    );
}

#[test]
fn decoder_rejects_length_checksum_and_trailing_data_corruption() {
    let encoded = PageBatch::new(vec![rich_page_scene()])
        .encode()
        .expect("encode page batch");

    let mut bad_length = encoded.clone();
    let declared = u32::from_le_bytes(bad_length[12..16].try_into().expect("length bytes"));
    bad_length[12..16].copy_from_slice(&declared.saturating_add(1).to_le_bytes());
    assert!(matches!(
        PageBatch::decode(&bad_length),
        Err(WireError::LengthMismatch { .. })
    ));

    let mut bad_checksum = encoded.clone();
    let last = bad_checksum.last_mut().expect("payload byte");
    *last ^= 1;
    assert!(matches!(
        PageBatch::decode(&bad_checksum),
        Err(WireError::ChecksumMismatch { .. })
    ));

    let mut trailing = PageBatch::new(Vec::new())
        .encode()
        .expect("encode empty page batch");
    trailing.push(0xff);
    refresh_envelope(&mut trailing);
    assert_eq!(
        PageBatch::decode(&trailing),
        Err(WireError::TrailingBytes { remaining: 1 })
    );
}

#[test]
fn decoder_limits_collection_allocations_before_allocating() {
    let mut encoded = PageBatch::new(Vec::new())
        .encode()
        .expect("encode empty page batch");
    encoded[HEADER_LEN..HEADER_LEN + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    refresh_envelope(&mut encoded);

    assert_eq!(
        PageBatch::decode(&encoded),
        Err(WireError::CollectionTooLarge {
            field: "pages",
            limit: 1_000_000,
            actual: u32::MAX as usize,
        })
    );
}

#[test]
fn encoder_rejects_invalid_utf8_boundaries_and_reversed_ranges() {
    let mut request =
        MeasureRequest::new(7, "é", LayoutUnit::from_px(16), LayoutUnit::from_px(300));
    request.text_range = 1..2;
    assert_eq!(
        MeasureBatch::new(vec![request]).encode(),
        Err(WireError::InvalidRange {
            field: "measure text range",
        })
    );

    let mut page = rich_page_scene();
    page.selections[0].start = 9;
    page.selections[0].end = 2;
    assert_eq!(
        PageBatch::new(vec![page]).encode(),
        Err(WireError::InvalidRange { field: "selection" })
    );

    let mut page = rich_page_scene();
    page.fragments[0].source_range = Some(SourceRange { start: 9, end: 2 });
    assert_eq!(
        PageBatch::new(vec![page]).encode(),
        Err(WireError::InvalidRange {
            field: "source range",
        })
    );
}

fn rich_measure_batch() -> MeasureBatch {
    let mut primary = FontDescriptor::new("Noto Serif", FontSetFingerprint(0x0102_0304));
    primary.font_id = Some(FontId::new(u32::MAX));
    primary.weight = 725;
    primary.style = FontStyle::Oblique;
    primary.stretch = 88;
    let mut fallback = FontDescriptor::new("Noto Sans CJK", FontSetFingerprint(u64::MAX));
    fallback.style = FontStyle::Italic;
    let fonts = FontFallbackChain {
        primary,
        fallbacks: vec![fallback],
    };
    let text: Arc<str> = Arc::from("A中🙂\n");
    let text_len = u32::try_from(text.len()).expect("fixture text length");
    let request = MeasureRequest {
        id: u32::MAX,
        paragraph_id: 42,
        text,
        text_range: 0..text_len,
        style_runs: vec![
            TextStyleRun::new(0, 1, LayoutUnit::from_raw(i64::MIN), fonts.clone()),
            TextStyleRun::new(1, text_len, LayoutUnit::from_raw(i64::MAX), fonts.clone()),
        ],
        font_size: LayoutUnit::from_px(17),
        max_width: LayoutUnit::from_raw(i64::MAX),
        available_width: LayoutUnit::from_raw(-64),
        locale: Arc::from("zh-Hans"),
        direction: TextDirection::Rtl,
        text_scale: LayoutUnit::from_raw(80),
        font_candidates: fonts,
        strut: StrutStyle {
            ascent: LayoutUnit::from_px(12),
            descent: LayoutUnit::from_px(4),
            leading: LayoutUnit::from_px(2),
        },
        height_behavior: HeightBehavior::Tight,
        request_fingerprint: u64::MAX,
    };
    MeasureBatch::new(vec![request])
}

fn rich_page_scene() -> PageScene {
    let start_anchor = TextAnchor::new(
        DocumentId::new(3),
        NodeId::new(7),
        2,
        TextAffinity::Upstream,
    );
    let end_anchor = TextAnchor::new(
        DocumentId::new(3),
        NodeId::new(7),
        19,
        TextAffinity::Downstream,
    );
    let kinds = [
        SceneFragmentKind::TextLine,
        SceneFragmentKind::Marker,
        SceneFragmentKind::Image,
        SceneFragmentKind::Divider,
        SceneFragmentKind::BackgroundBorder,
        SceneFragmentKind::DebugOverlay,
        SceneFragmentKind::UnsupportedPlaceholder,
    ];
    let fragments = kinds
        .into_iter()
        .enumerate()
        .map(|(index, kind)| SceneFragment {
            id: u32::try_from(index).expect("fragment id"),
            kind,
            node_id: NodeId::new(7),
            rect: Rect {
                x: LayoutUnit::from_raw(-64 + i64::try_from(index).expect("fragment x")),
                y: LayoutUnit::from_px(i64::try_from(index).expect("fragment y") * 20),
                width: LayoutUnit::from_px(300),
                height: LayoutUnit::from_px(20),
            },
            text: (index == 0).then(|| Arc::from("完整字段🙂")),
            source_range: (index == 0).then_some(SourceRange::new(10, 30).expect("source range")),
            anchor_range: (index == 0).then_some(TextAnchorRange {
                start: start_anchor,
                end: end_anchor,
            }),
            line_index: (index == 0).then_some(u32::MAX),
            overflow: index == 6,
        })
        .collect();
    let link_kinds = [
        LinkKind::Internal,
        LinkKind::External,
        LinkKind::Resource,
        LinkKind::Footnote,
        LinkKind::Unknown,
    ];
    let links = link_kinds
        .into_iter()
        .enumerate()
        .map(|(index, kind)| LinkRegion {
            rect: Rect {
                x: LayoutUnit::from_px(i64::try_from(index).expect("link x")),
                y: LayoutUnit::from_px(2),
                width: LayoutUnit::from_px(50),
                height: LayoutUnit::from_px(10),
            },
            node_id: NodeId::new(u32::try_from(index + 1).expect("link node id")),
            href: Arc::from(format!("chapter.xhtml#link-{index}")),
            resolved_document: (index == 0).then(|| Arc::from("OPS/chapter.xhtml")),
            fragment: (index == 0).then(|| Arc::from("link-0")),
            kind,
        })
        .collect();
    let diagnostic_codes = [
        DiagnosticCode::Io,
        DiagnosticCode::InvalidContainer,
        DiagnosticCode::InvalidPackage,
        DiagnosticCode::UnsupportedFeature,
        DiagnosticCode::ResourceLimitExceeded,
        DiagnosticCode::Parse,
        DiagnosticCode::Layout,
        DiagnosticCode::Cancelled,
        DiagnosticCode::Protocol,
        DiagnosticCode::Internal,
    ];
    let diagnostics = diagnostic_codes
        .into_iter()
        .enumerate()
        .map(|(index, code)| Diagnostic {
            code,
            severity: match index % 3 {
                0 => Severity::Info,
                1 => Severity::Warning,
                _ => Severity::Error,
            },
            message: Arc::from(format!("diagnostic-{index}")),
            resource: (index == 0).then_some(ResourceId::new(u32::MAX)),
            source_range: (index == 0)
                .then_some(SourceRange::new(0, u32::MAX).expect("diagnostic source range")),
        })
        .collect();

    PageScene {
        page_index: u32::MAX,
        size: PageSize {
            width: LayoutUnit::from_raw(i64::MAX),
            height: LayoutUnit::from_raw(i64::MIN),
        },
        start_anchor: Some(start_anchor),
        end_anchor: Some(end_anchor),
        fragments,
        links,
        anchors: vec![AnchorRegion {
            rect: Rect::default(),
            key: Arc::from("OPS/chapter.xhtml#start"),
            node_id: NodeId::new(7),
        }],
        selections: vec![SelectionMap {
            node_id: NodeId::new(7),
            start: 2,
            end: 19,
            rects: vec![Rect {
                x: LayoutUnit::from_px(1),
                y: LayoutUnit::from_px(2),
                width: LayoutUnit::from_px(3),
                height: LayoutUnit::from_px(4),
            }],
        }],
        semantics: vec![SemanticNode {
            node_id: NodeId::new(7),
            rect: Rect::default(),
            role: Arc::from("paragraph"),
            label: Arc::from("Accessible label"),
        }],
        fingerprint: PageFingerprint(ContentHash::from_bytes(b"page-v1")),
        next_break_token: Some(BreakToken {
            node_id: NodeId::new(u32::MAX),
            child_index: u32::MAX,
            text_offset: u32::MAX,
            continuation: true,
            page_index: u32::MAX,
            content_fingerprint: ContentHash::from_bytes(b"content"),
            config_fingerprint: u64::MAX,
            text_backend_id: TextBackendId(u64::MAX - 1),
            font_fingerprint: FontSetFingerprint(u64::MAX - 2),
        }),
        diagnostics,
    }
}

fn assert_envelope(bytes: &[u8], payload_kind: u16) {
    assert_eq!(&bytes[..8], b"PGLTSCN\0");
    assert_eq!(u16::from_le_bytes(bytes[8..10].try_into().unwrap()), 1);
    assert_eq!(
        u16::from_le_bytes(bytes[10..12].try_into().unwrap()),
        payload_kind
    );
    assert_eq!(
        u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize,
        bytes.len() - HEADER_LEN
    );
    assert_eq!(
        u32::from_le_bytes(bytes[16..20].try_into().unwrap()),
        crc32(&bytes[HEADER_LEN..])
    );
}

fn refresh_envelope(bytes: &mut [u8]) {
    let payload_length = u32::try_from(bytes.len() - HEADER_LEN).expect("payload length");
    bytes[12..16].copy_from_slice(&payload_length.to_le_bytes());
    let checksum = crc32(&bytes[HEADER_LEN..]);
    bytes[16..20].copy_from_slice(&checksum.to_le_bytes());
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}
