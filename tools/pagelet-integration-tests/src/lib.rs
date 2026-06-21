#![forbid(unsafe_code)]
//! Private integration contracts for pagelet.

/// Return true when the integration crate is linked to the public pagelet crate.
#[must_use]
pub fn public_pagelet_facade_is_available() -> bool {
    pagelet::build_info().crate_name == "pagelet"
}

#[cfg(test)]
mod tests {
    use pagelet_testkit::{FixtureBuilder, GoldenDocument, GoldenSection};

    use super::*;

    #[test]
    fn integration_crate_uses_public_pagelet_facade() {
        assert!(public_pagelet_facade_is_available());
    }

    #[test]
    fn integration_crate_uses_private_testkit() {
        let fixture = FixtureBuilder::minimal_epub3("contract-smoke").build();
        let golden = GoldenDocument::empty().section(
            GoldenSection::new("fixture")
                .entry("id", fixture.id.clone())
                .entry("entry_count", fixture.entries.len().to_string()),
        );

        assert!(fixture.to_bytes().starts_with(b"PAGELET-FIXTURE"));
        assert!(golden.to_normalized_text().contains("entry_count=4"));
    }
}
