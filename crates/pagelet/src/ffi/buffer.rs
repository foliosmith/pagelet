//! Explicitly-owned byte buffers for native hosts.

use std::{
    collections::BTreeMap,
    fmt, ptr,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, MutexGuard,
    },
};

static NEXT_BUFFER_ID: AtomicU64 = AtomicU64::new(1);

/// Rust-owned byte buffer descriptor returned across the C ABI.
///
/// `data` remains valid until this exact descriptor is released. Hosts may
/// either read it directly or copy it into host-owned memory.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
#[repr(C)]
pub struct NativeBuffer {
    pub id: u64,
    pub data: *const u8,
    pub len: usize,
}

impl NativeBuffer {
    /// Empty descriptor used when a call has no byte payload.
    pub const EMPTY: Self = Self {
        id: 0,
        data: ptr::null(),
        len: 0,
    };

    /// Return true when this descriptor carries no owned allocation.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.id == 0
    }
}

impl Default for NativeBuffer {
    fn default() -> Self {
        Self::EMPTY
    }
}

/// Validation or capacity failure for an owned byte buffer.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum BufferError {
    InvalidOrReleased,
    DestinationTooSmall { required: usize, available: usize },
    GenerationExhausted,
}

impl fmt::Display for BufferError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidOrReleased => formatter.write_str("invalid or released native buffer"),
            Self::DestinationTooSmall {
                required,
                available,
            } => write!(
                formatter,
                "native buffer needs {required} destination bytes, got {available}"
            ),
            Self::GenerationExhausted => {
                formatter.write_str("native buffer generation space exhausted")
            }
        }
    }
}

impl std::error::Error for BufferError {}

/// Owner table for Rust-allocated byte buffers.
///
/// Descriptor ids are process-unique and never reused. Releasing a descriptor
/// removes its allocation, and repeated release is a safe no-op.
#[derive(Default)]
pub struct BufferPool {
    entries: Mutex<BTreeMap<u64, Box<[u8]>>>,
}

impl fmt::Debug for BufferPool {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BufferPool")
            .field("live_buffers", &self.live_count())
            .finish()
    }
}

impl BufferPool {
    /// Create an empty buffer pool.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adopt a byte vector and return a stable native descriptor.
    pub fn allocate(&self, bytes: Vec<u8>) -> Result<NativeBuffer, BufferError> {
        let id = NEXT_BUFFER_ID
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .map_err(|_| BufferError::GenerationExhausted)?;
        let bytes = bytes.into_boxed_slice();
        let descriptor = NativeBuffer {
            id,
            data: bytes.as_ptr(),
            len: bytes.len(),
        };
        let previous = self.lock_entries().insert(id, bytes);
        debug_assert!(previous.is_none(), "native buffer id reused");
        Ok(descriptor)
    }

    /// Copy a live buffer into host-owned memory.
    pub fn copy_to(
        &self,
        buffer: NativeBuffer,
        destination: &mut [u8],
    ) -> Result<usize, BufferError> {
        let entries = self.lock_entries();
        let bytes = require_buffer(&entries, buffer)?;
        if destination.len() < bytes.len() {
            return Err(BufferError::DestinationTooSmall {
                required: bytes.len(),
                available: destination.len(),
            });
        }
        destination[..bytes.len()].copy_from_slice(bytes);
        Ok(bytes.len())
    }

    /// Copy a live buffer into a Rust vector.
    pub fn to_vec(&self, buffer: NativeBuffer) -> Result<Vec<u8>, BufferError> {
        let entries = self.lock_entries();
        Ok(require_buffer(&entries, buffer)?.to_vec())
    }

    /// Release an owned allocation. Repeated release is a safe no-op.
    pub fn release(&self, buffer: NativeBuffer) -> bool {
        let mut entries = self.lock_entries();
        if require_buffer(&entries, buffer).is_err() {
            return false;
        }
        entries.remove(&buffer.id).is_some()
    }

    /// Return the number of currently-owned native buffers.
    #[must_use]
    pub fn live_count(&self) -> usize {
        self.lock_entries().len()
    }

    fn lock_entries(&self) -> MutexGuard<'_, BTreeMap<u64, Box<[u8]>>> {
        match self.entries.lock() {
            Ok(entries) => entries,
            Err(poisoned) => {
                self.entries.clear_poison();
                poisoned.into_inner()
            }
        }
    }
}

impl Drop for BufferPool {
    fn drop(&mut self) {
        let entries = match self.entries.get_mut() {
            Ok(entries) => entries,
            Err(poisoned) => poisoned.into_inner(),
        };
        #[cfg(debug_assertions)]
        let leaked = entries.len();
        entries.clear();

        #[cfg(debug_assertions)]
        if leaked != 0 && !std::thread::panicking() {
            panic!("leaked native buffers at pool drop: {leaked}");
        }
    }
}

fn require_buffer(
    entries: &BTreeMap<u64, Box<[u8]>>,
    buffer: NativeBuffer,
) -> Result<&[u8], BufferError> {
    let bytes = entries
        .get(&buffer.id)
        .ok_or(BufferError::InvalidOrReleased)?;
    if bytes.as_ptr() != buffer.data || bytes.len() != buffer.len {
        return Err(BufferError::InvalidOrReleased);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_can_be_read_copied_and_released_once() {
        let pool = BufferPool::new();
        let buffer = pool.allocate(vec![1, 2, 3, 4]).expect("allocate");
        assert_eq!(pool.live_count(), 1);
        assert_eq!(pool.to_vec(buffer).expect("read"), vec![1, 2, 3, 4]);

        let mut short = [0_u8; 3];
        assert_eq!(
            pool.copy_to(buffer, &mut short),
            Err(BufferError::DestinationTooSmall {
                required: 4,
                available: 3,
            })
        );
        let mut exact = [0_u8; 4];
        assert_eq!(pool.copy_to(buffer, &mut exact), Ok(4));
        assert_eq!(exact, [1, 2, 3, 4]);

        assert!(pool.release(buffer));
        assert!(!pool.release(buffer));
        assert_eq!(pool.to_vec(buffer), Err(BufferError::InvalidOrReleased));
        assert_eq!(pool.live_count(), 0);
    }

    #[test]
    fn altered_descriptor_cannot_read_or_release_allocation() {
        let pool = BufferPool::new();
        let buffer = pool.allocate(vec![7, 8]).expect("allocate");
        let altered = NativeBuffer { len: 1, ..buffer };
        assert_eq!(pool.to_vec(altered), Err(BufferError::InvalidOrReleased));
        assert!(!pool.release(altered));
        assert!(pool.release(buffer));
    }

    #[cfg(debug_assertions)]
    #[test]
    fn debug_drop_detects_and_releases_leaked_buffers() {
        let leaked = std::panic::catch_unwind(|| {
            let pool = BufferPool::new();
            pool.allocate(vec![1]).expect("allocate");
            drop(pool);
        });
        assert!(leaked.is_err());
    }
}
