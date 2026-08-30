use std::{env, fs, path::PathBuf, sync::Arc};

use pagelet::{
    core::{FontId, LayoutUnit},
    text::{
        FontDescriptor, FontFallbackChain, FontSetFingerprint, FontStyle, HeightBehavior,
        MeasureRequest, StrutStyle, TextDirection, TextStyleRun,
    },
    wire::{MeasureBatch, MeasuredBatch},
};

fn main() {
    let mut arguments = env::args_os().skip(1);
    let command = arguments.next().expect("command");
    let path = PathBuf::from(arguments.next().expect("path"));
    assert!(arguments.next().is_none(), "unexpected argument");

    match command.to_str().expect("UTF-8 command") {
        "write-request" => {
            let batch = MeasureBatch::new(vec![
                request(7, 70, "Hello 中🙂\nsecond line", TextDirection::Ltr, 120),
                request(8, 80, "مرحبا بالعالم", TextDirection::Rtl, 80),
            ]);
            fs::write(path, batch.encode().expect("encode request"))
                .expect("write request fixture");
        }
        "verify-response" => {
            let bytes = fs::read(path).expect("read response fixture");
            let batch = MeasuredBatch::decode(&bytes).expect("decode Flutter response");
            assert_eq!(batch.results.len(), 2);
            assert_eq!(batch.results[0].request_id, 7);
            assert_eq!(batch.results[0].request_fingerprint, 0x0707);
            assert_eq!(
                batch.results[0].utf8_len,
                "Hello 中🙂\nsecond line".len() as u32
            );
            assert!(batch.results[0].lines.iter().any(|line| line.hard_break));
            assert!(batch.results[0]
                .clusters
                .iter()
                .any(|cluster| cluster.text_start == 6 && cluster.text_end == 9));
            assert!(batch.results[0]
                .clusters
                .iter()
                .any(|cluster| cluster.text_start == 9 && cluster.text_end == 13));
            assert_eq!(batch.results[1].request_id, 8);
        }
        other => panic!("unsupported command {other}"),
    }
}

fn request(
    id: u32,
    paragraph_id: u32,
    text: &'static str,
    direction: TextDirection,
    width: i64,
) -> MeasureRequest {
    let mut primary = FontDescriptor::new("Ahem", FontSetFingerprint(0));
    primary.font_id = Some(FontId::new(id));
    primary.weight = 400;
    primary.style = FontStyle::Normal;
    primary.stretch = 100;
    let fonts = FontFallbackChain {
        primary,
        fallbacks: Vec::new(),
    };
    let text: Arc<str> = Arc::from(text);
    let text_end = u32::try_from(text.len()).expect("fixture text length");
    MeasureRequest {
        id,
        paragraph_id,
        text,
        text_range: 0..text_end,
        style_runs: vec![TextStyleRun::new(
            0,
            text_end,
            LayoutUnit::from_px(16),
            fonts.clone(),
        )],
        font_size: LayoutUnit::from_px(16),
        max_width: LayoutUnit::from_px(width),
        available_width: LayoutUnit::from_px(width),
        locale: Arc::from("en-US"),
        direction,
        text_scale: LayoutUnit::from_raw(LayoutUnit::SCALE),
        font_candidates: fonts,
        strut: StrutStyle::default(),
        height_behavior: HeightBehavior::Natural,
        request_fingerprint: u64::from(id) * 0x0101,
    }
}
