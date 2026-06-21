#![forbid(unsafe_code)]
//! Private test fixtures, golden output, and generators for pagelet tooling.

pub mod fixtures;
pub mod golden;
pub mod random;

pub use fixtures::{Fixture, FixtureBuilder, FixtureEntry};
pub use golden::{GoldenDocument, GoldenEntry, GoldenSection};
pub use random::DeterministicRng;

/// Return the pagelet crate version this testkit was built against.
#[must_use]
pub fn pagelet_version() -> &'static str {
    pagelet::build_info().version
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn testkit_links_against_pagelet() {
        assert_eq!(pagelet::build_info().crate_name, "pagelet");
        assert_eq!(pagelet_version(), pagelet::build_info().version);
    }
}
