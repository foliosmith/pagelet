#![no_main]

use libfuzzer_sys::fuzz_target;
use pagelet::epub::resolve_resource_path;

const MAX_INPUT_LEN: usize = 4096;

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_INPUT_LEN {
        return;
    }

    let text = String::from_utf8_lossy(data);
    let mut parts = text.split('\n');
    let base = parts.next().unwrap_or_default();
    let href = parts.next().unwrap_or_default();
    let _ = resolve_resource_path(base, href);
});
