#![no_main]

use libfuzzer_sys::fuzz_target;
use pagelet::{
    engine::Engine,
    ffi::{BufferPool, EngineHandle, HandleRegistry, NativeBuffer},
};

const MAX_INPUT_LEN: usize = 4096;

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_INPUT_LEN {
        return;
    }

    let registry = HandleRegistry::new();
    let buffers = BufferPool::new();
    let mut handles = Vec::<u64>::new();
    let mut descriptors = Vec::<NativeBuffer>::new();

    for operation in data.chunks(4) {
        let selector = usize::from(operation.get(1).copied().unwrap_or_default());
        match operation.first().copied().unwrap_or_default() % 8 {
            0 => {
                if let Ok(handle) = registry.insert_engine(Engine::new()) {
                    handles.push(handle.as_raw());
                }
            }
            1 => {
                if let Some(raw) = handles.get(selector % handles.len().max(1)).copied() {
                    let _ = registry.dispose(raw);
                }
            }
            2 => {
                if let Some(raw) = handles.get(selector % handles.len().max(1)).copied() {
                    if let Some(handle) = EngineHandle::from_raw(raw) {
                        let _ = registry.engine(handle);
                    }
                }
            }
            3 => {
                if let Ok(buffer) = buffers.allocate(operation.to_vec()) {
                    descriptors.push(buffer);
                }
            }
            4 => {
                if let Some(buffer) = descriptors
                    .get(selector % descriptors.len().max(1))
                    .copied()
                {
                    let mut destination =
                        vec![0_u8; usize::from(operation.get(2).copied().unwrap_or_default())];
                    let _ = buffers.copy_to(buffer, &mut destination);
                }
            }
            5 => {
                if let Some(buffer) = descriptors
                    .get(selector % descriptors.len().max(1))
                    .copied()
                {
                    let _ = buffers.release(buffer);
                }
            }
            6 => {
                if let Some(buffer) = descriptors
                    .get(selector % descriptors.len().max(1))
                    .copied()
                {
                    let altered = NativeBuffer {
                        len: buffer.len.wrapping_add(1),
                        ..buffer
                    };
                    let _ = buffers.release(altered);
                    let _ = buffers.to_vec(altered);
                }
            }
            7 => {
                let raw = u64::from_le_bytes(padded_u64(operation));
                let _ = registry.dispose(raw);
            }
            _ => unreachable!(),
        }
    }

    for raw in handles {
        let _ = registry.dispose(raw);
    }
    for buffer in descriptors {
        let _ = buffers.release(buffer);
    }
    assert!(registry.leak_report().is_empty());
    assert_eq!(buffers.live_count(), 0);
});

fn padded_u64(bytes: &[u8]) -> [u8; 8] {
    let mut value = [0_u8; 8];
    value[..bytes.len()].copy_from_slice(bytes);
    value
}
