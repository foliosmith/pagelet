#![no_main]

use libfuzzer_sys::fuzz_target;
use pagelet::{
    core::ResourceLimits,
    epub::{parse_xhtml_tree, tokenize_xhtml_with_limits, CompatibilityMode},
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

    let _ = tokenize_xhtml_with_limits(&input, limits);
    let _ = parse_xhtml_tree(&input, CompatibilityMode::Compatible, limits);
});
