#![no_main]

use libfuzzer_sys::fuzz_target;

const MAX_INPUT_LEN: usize = 4096;

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_INPUT_LEN {
        return;
    }

    let _engine = pagelet::engine::Engine::new();
    let _info = pagelet::build_info();

    if let Some(bytes) = data.get(..8) {
        let mut raw = [0_u8; 8];
        raw.copy_from_slice(bytes);
        let _book_id = pagelet::core::BookId::new(u64::from_le_bytes(raw));
    }
});
