#![no_main]

use libfuzzer_sys::fuzz_target;
use pagelet::{
    core::ResourceLimits,
    epub::{OpenOptions, ZipPublicationStore},
};

const MAX_INPUT_LEN: usize = 256 * 1024;

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_INPUT_LEN {
        return;
    }

    let mut limits = ResourceLimits::mobile_defaults();
    limits.max_zip_entries = 256;
    limits.max_total_decompressed_bytes = 4 * 1024 * 1024;
    limits.max_single_resource_bytes = 1024 * 1024;

    let _ = ZipPublicationStore::from_bytes(
        data.to_vec(),
        OpenOptions {
            limits,
            ..OpenOptions::compatible()
        },
    );
});
