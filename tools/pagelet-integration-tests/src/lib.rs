#![forbid(unsafe_code)]
//! Private integration contracts for pagelet.

/// Return true when the integration crate is linked to the public pagelet crate.
#[must_use]
pub fn public_pagelet_facade_is_available() -> bool {
    pagelet::build_info().crate_name == "pagelet"
}

#[cfg(test)]
mod tests {
    use pagelet::epub::{open_book, resolve_resource_path, NavigationSource, OpenOptions};
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

    #[test]
    fn generated_epub3_opens_to_metadata_and_navigation() {
        let fixture = ValidEpubBuilder::epub3("m1-epub3")
            .xhtml("EPUB/chapter-1.xhtml", "Chapter 1", "<p>smoke</p>")
            .build();

        let book = open_book(fixture.bytes().to_vec()).expect("open fixture");

        assert_eq!(book.package.metadata.title.as_deref(), Some("m1-epub3"));
        assert_eq!(book.navigation.source, NavigationSource::Epub3Nav);
        assert_eq!(book.navigation.toc[0].href, "chapter-1.xhtml");
        assert!(book.store_stats.read_count < book.resources.len() as u64);
    }

    #[test]
    fn generated_epub2_uses_ncx_navigation() {
        let fixture = ValidEpubBuilder::epub2("m1-epub2")
            .xhtml("EPUB/chapter-1.xhtml", "Chapter 1", "<p>legacy</p>")
            .build();

        let book = open_book(fixture.bytes().to_vec()).expect("open fixture");

        assert_eq!(book.package.metadata.package_version, "2.0");
        assert_eq!(book.navigation.source, NavigationSource::Ncx);
        assert_eq!(book.navigation.toc[0].label, "Start");
    }

    #[test]
    fn path_security_property_rejects_container_escape() {
        let cases = [
            ("", "../evil.xhtml"),
            ("EPUB", "../../evil.xhtml"),
            ("EPUB", r"..\evil.xhtml"),
            ("EPUB", "file:///tmp/evil.xhtml"),
            ("EPUB", "https://example.test/evil.xhtml"),
        ];

        for (base, href) in cases {
            assert!(resolve_resource_path(base, href).is_err(), "{base} {href}");
        }
        assert_eq!(
            resolve_resource_path("EPUB/Text", "../chapter.xhtml").expect("resolve"),
            "EPUB/chapter.xhtml"
        );
    }

    #[test]
    fn resource_limits_are_reported_for_zip_entry_count() {
        let fixture = ValidEpubBuilder::epub3("m1-limits")
            .xhtml("EPUB/chapter-1.xhtml", "Chapter 1", "<p>smoke</p>")
            .build();
        let mut options = OpenOptions::compatible();
        options.limits.max_zip_entries = 1;

        let error = pagelet::epub::open_book_with_options(fixture.bytes().to_vec(), options)
            .expect_err("limit");

        assert_eq!(
            error.code(),
            pagelet::core::DiagnosticCode::ResourceLimitExceeded
        );
    }
}
