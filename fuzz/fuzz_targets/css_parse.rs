#![no_main]

use libfuzzer_sys::fuzz_target;
use pagelet::{core::ResourceLimits, epub::parse_css};

const MAX_INPUT_LEN: usize = 16 * 1024;

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_INPUT_LEN {
        return;
    }

    let input = String::from_utf8_lossy(data);
    let mut limits = ResourceLimits::mobile_defaults();
    limits.max_css_selectors = 1024;
    let _ = parse_css(&input, limits);
});
