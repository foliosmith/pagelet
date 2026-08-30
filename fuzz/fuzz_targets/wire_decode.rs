#![no_main]

use libfuzzer_sys::fuzz_target;
use pagelet::wire::{MeasureBatch, MeasuredBatch, PageBatch};

const MAX_INPUT_LEN: usize = 256 * 1024;

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_INPUT_LEN {
        return;
    }

    let _ = PageBatch::decode(data);
    let _ = MeasureBatch::decode(data);
    let _ = MeasuredBatch::decode(data);

    if data.len() >= 2 {
        let version = u16::from(data[0] % 3 + 1);
        let kind = u16::from(data[1] % 3 + 1);
        let encoded = envelope(version, kind, &data[2..]);
        match kind {
            1 => {
                let _ = PageBatch::decode(&encoded);
            }
            2 => {
                let _ = MeasureBatch::decode(&encoded);
            }
            3 => {
                let _ = MeasuredBatch::decode(&encoded);
            }
            _ => unreachable!(),
        }
    }
});

fn envelope(version: u16, kind: u16, payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(20 + payload.len());
    bytes.extend_from_slice(b"PGLTSCN\0");
    bytes.extend_from_slice(&version.to_le_bytes());
    bytes.extend_from_slice(&kind.to_le_bytes());
    bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&crc32(payload).to_le_bytes());
    bytes.extend_from_slice(payload);
    bytes
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
