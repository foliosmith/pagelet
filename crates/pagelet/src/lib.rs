#![forbid(unsafe_code)]
//! Deterministic EPUB parsing and pagination engine.
//!
//! `pagelet` is the only Rust library crate published by this repository.
//! Implementation boundaries are kept as internal modules until their public
//! APIs are ready to be stabilized.

pub mod cli;
pub mod core;
mod document;
pub mod engine;
pub mod epub;
mod ffi;
mod layout;
#[cfg(test)]
mod testkit;
pub mod text;
mod wire;

/// Static build metadata for the pagelet crate.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct BuildInfo {
    /// Published crate name.
    pub crate_name: &'static str,
    /// Crate semantic version.
    pub version: &'static str,
}

/// Return build metadata for the linked pagelet crate.
#[must_use]
pub const fn build_info() -> BuildInfo {
    BuildInfo {
        crate_name: env!("CARGO_PKG_NAME"),
        version: env!("CARGO_PKG_VERSION"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_info_matches_manifest() {
        assert_eq!(
            build_info(),
            BuildInfo {
                crate_name: "pagelet",
                version: env!("CARGO_PKG_VERSION"),
            }
        );
    }
}
