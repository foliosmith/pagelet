#![forbid(unsafe_code)]
//! Private integration contracts for pagelet.

/// Return true when the integration crate is linked to the public pagelet crate.
#[must_use]
pub fn public_pagelet_facade_is_available() -> bool {
    pagelet::build_info().crate_name == "pagelet"
}

#[cfg(test)]
mod tests {
    use pagelet_testkit::{GoldenDocument, GoldenSectionName, ValidEpubBuilder};

    use super::*;

    #[test]
    fn integration_crate_uses_public_pagelet_facade() {
        assert!(public_pagelet_facade_is_available());
    }

    #[test]
    fn integration_crate_uses_private_testkit() {
        let fixture = ValidEpubBuilder::epub3("contract-smoke")
            .xhtml("EPUB/chapter-1.xhtml", "Chapter 1", "<p>smoke</p>")
            .build();
        let golden = GoldenDocument::empty()
            .entry(GoldenSectionName::BookSummary, "id", fixture.id.clone())
            .entry(
                GoldenSectionName::Manifest,
                "entry_count",
                fixture.entries.len().to_string(),
            );

        assert!(fixture.bytes().starts_with(b"PK\x03\x04"));
        assert!(golden.to_json().contains(r#""entry_count""#));
    }
}
