#![no_main]

use libfuzzer_sys::fuzz_target;
use pagelet::epub::parse_image_header;

const MAX_INPUT_LEN: usize = 16 * 1024;

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_INPUT_LEN {
        return;
    }

    let media_type = match data.first().copied().unwrap_or_default() % 4 {
        0 => "image/png",
        1 => "image/jpeg",
        2 => "image/gif",
        _ => "application/octet-stream",
    };
    let _ = parse_image_header(data, media_type);
});
