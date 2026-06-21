//! Public engine facade.

/// Minimal engine handle for the pre-alpha crate skeleton.
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq)]
pub struct Engine;

impl Engine {
    /// Create a new engine facade.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}
