#![forbid(unsafe_code)]
//! Private test fixtures, golden output, and generators for pagelet tooling.

pub mod fixtures;
pub mod generators;
pub mod golden;
pub mod random;
pub mod text;

pub use fixtures::{
    EpubMutation, EpubMutator, Fixture, FixtureEntry, FixtureKind, RawEpubBuilder, ValidEpubBuilder,
};
pub use generators::{
    GeneratedBidiRun, GeneratedBlock, GeneratedChapterIr, GeneratedFallbackEdge,
    GeneratedFontMetrics, GeneratedGrapheme, GeneratedInline, GeneratedLayoutConfig,
    GeneratedManifestGraph, GeneratedManifestItem, GeneratedOcfArchive, GeneratedOcfEntry,
    GeneratedOcfPath, GeneratedSpineItem, GeneratedTextDirection, GeneratedUnicodeText,
    GeneratedViewport, GeneratedXmlAttribute, GeneratedXmlElement, GeneratedXmlNamespace,
    GeneratorLimits, PropertyFailureContext, PropertyGenerator,
};
pub use golden::{GoldenDocument, GoldenEntry, GoldenSection, GoldenSectionName};
pub use random::DeterministicRng;
pub use text::{DeterministicTextBackend, DeterministicTextMetrics};

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
