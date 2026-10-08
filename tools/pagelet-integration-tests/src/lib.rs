#![forbid(unsafe_code)]
//! Private integration contracts for pagelet.

/// Return true when the integration crate is linked to the public pagelet crate.
#[must_use]
pub fn public_pagelet_facade_is_available() -> bool {
    pagelet::build_info().crate_name == "pagelet"
}

#[cfg(test)]
mod tests {
    use pagelet::{
        core::CancellationToken,
        epub::{open_book, resolve_resource_path, NavigationSource, OpenOptions},
        layout::{
            anchor_to_page, paginate_chapter_with_options, validate_layout_invariants,
            LayoutOptions,
        },
        text::{DefaultTextBackend, TextBackend},
    };
    use pagelet_testkit::{
        FixtureEntry, GeneratorLimits, GoldenDocument, GoldenSectionName, PropertyGenerator,
        ValidEpubBuilder,
    };

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
    fn chapter_text_includes_readable_descendants_of_unsupported_elements() {
        let fixture = ValidEpubBuilder::epub3("unsupported-text")
            .xhtml(
                "EPUB/chapter-1.xhtml",
                "Chapter",
                "<p>Before</p><pre><code><span>const x = 1;</span></code></pre><custom><p>Tail</p></custom><script>hidden script</script><style>p { color: red; }</style><template><p>hidden template</p></template><p>After</p>",
            )
            .build();
        let book = pagelet::engine::Engine::new()
            .open_bytes(fixture.bytes().to_vec())
            .expect("open");
        let chapter = book.open_spine_item(0).expect("chapter");
        assert_eq!(chapter.visible_text(), "Before\nconst x = 1;\nTail\nAfter");
    }

    #[test]
    fn xml_character_references_decode_once_in_text_attributes_and_metadata() {
        let fixture = ValidEpubBuilder::epub3("literal &lt;")
            .xhtml(
                "EPUB/chapter-1.xhtml",
                "Chapter",
                "<p id=\"a&#49;\">&#39;&#x4E2D;&#x1F642; &amp;#39; &#0; &#xD800; &#x110000; &unknown; &#xZZ;</p><p><a href=\"#a&#49;\">jump</a></p>",
            )
            .build();
        let book = pagelet::engine::Engine::new()
            .open_bytes(fixture.bytes().to_vec())
            .expect("open");
        let chapter = book.open_spine_item(0).expect("chapter");
        assert_eq!(
            chapter.visible_text(),
            "'中🙂 &#39; &#0; &#xD800; &#x110000; &unknown; &#xZZ;\njump"
        );
        assert!(chapter.anchors.get("EPUB/chapter-1.xhtml#a1").is_some());
        assert_eq!(chapter.links[0].fragment.as_deref(), Some("a1"));
        assert_eq!(
            book.summary().package.metadata.title.as_deref(),
            Some("literal &lt;")
        );
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

    #[test]
    fn generated_unicode_layout_properties_are_replayable() {
        for seed in 0..32 {
            let mut generator = PropertyGenerator::with_limits(seed, GeneratorLimits::smoke());
            let context = generator.failure_context("unicode layout invariants");
            let generated = generator.unicode_text();
            let fixture = ValidEpubBuilder::epub3(format!("property-{seed}"))
                .xhtml(
                    "EPUB/chapter-1.xhtml",
                    "Property",
                    &format!("<p>{}</p>", escape_xml(&generated.text)),
                )
                .build();
            let chapter = pagelet::epub::open_first_chapter_ir(fixture.bytes().to_vec())
                .unwrap_or_else(|error| panic!("{context}: {error}"));
            let backend = DefaultTextBackend::new();
            let options = LayoutOptions::default();
            let first = paginate_chapter_with_options(&chapter, &backend, options)
                .unwrap_or_else(|error| panic!("{context}: {error}"));
            let replay = paginate_chapter_with_options(&chapter, &backend, options)
                .unwrap_or_else(|error| panic!("{context}: {error}"));

            validate_layout_invariants(&chapter, &first.pages)
                .unwrap_or_else(|error| panic!("{context}: {error}"));
            assert_eq!(
                first
                    .pages
                    .iter()
                    .map(|page| page.fingerprint)
                    .collect::<Vec<_>>(),
                replay
                    .pages
                    .iter()
                    .map(|page| page.fingerprint)
                    .collect::<Vec<_>>(),
                "{context}"
            );
            for page in &first.pages {
                if let Some(anchor) = page.start_anchor {
                    assert!(anchor_to_page(&first.pages, anchor).is_some(), "{context}");
                }
            }
        }
    }

    #[test]
    fn wasm_chapter_ir_json_contains_renderable_node_contract() {
        let fixture = wasm_contract_fixture();

        let json =
            pagelet::cli::spine_chapter_ir_json(fixture.bytes().to_vec(), 0).expect("chapter json");

        assert!(json.contains(r#""href": "EPUB/chapter-1.xhtml""#));
        assert!(json.contains(r#""kind": "heading""#));
        assert!(json.contains(r#""level": 1"#));
        assert!(json.contains(r#""text": "Intro""#));
        assert!(json.contains(r#""kind": "list""#));
        assert!(json.contains(r#""children": ["#));
        assert!(json.contains(r#""kind": "image""#));
        assert!(json.contains(r#""resolved_path": "EPUB/images/cover.png""#));
        assert!(json.contains(r#""resource_id": "#));
        assert!(json.contains(r#""alt": "Cover""#));
        assert!(json.contains(r#""document_href": "EPUB/chapter-1.xhtml""#));
        assert!(json.contains(r#""kind": "internal""#));
        assert!(json.contains(r#""source_range": {"#));
    }

    #[test]
    fn wasm_pagination_json_supports_explicit_spine_item() {
        let fixture = wasm_contract_fixture();
        let options = pagelet::cli::layout_options_from_px(360, 480);

        let json = pagelet::cli::paginate_spine_item_bytes_json_with_options(
            fixture.bytes().to_vec(),
            0,
            OpenOptions::default(),
            options,
        )
        .expect("pagination json");

        assert!(json.contains(r#""href": "EPUB/chapter-1.xhtml""#));
        assert!(json.contains(r#""page_count": "#));
        assert!(json.contains(r#""pagination": {"#));
        assert!(json.contains(r#""complete": true"#));
        assert!(json.contains(r#""pages": ["#));
        assert!(json.contains(r#""page_index": 0"#));
        assert!(json.contains(r#""fragments": ["#));
    }

    #[test]
    fn host_measurement_wire_round_trip_resumes_public_pagination() {
        let fixture = wasm_contract_fixture();
        let options = pagelet::cli::layout_options_from_px(360, 480);
        let requested_wire = pagelet::cli::prepare_spine_item_measure_batch_with_options(
            fixture.bytes().to_vec(),
            0,
            OpenOptions::default(),
            options,
        )
        .expect("prepare host measurement batch");
        let requested = pagelet::wire::MeasureBatch::decode(&requested_wire)
            .expect("decode requested batch")
            .into_text_batch();
        assert!(
            requested.requests.len() > 1,
            "all paragraphs share one batch"
        );

        let measured = DefaultTextBackend::new()
            .measure_batch(&requested, &CancellationToken::new())
            .expect("host fixture measurements");
        let measured_wire = pagelet::wire::MeasuredBatch::from(measured)
            .encode()
            .expect("encode measured batch");
        let json = pagelet::cli::paginate_spine_item_bytes_json_with_host_measurements(
            fixture.bytes().to_vec(),
            0,
            OpenOptions::default(),
            options,
            &measured_wire,
        )
        .expect("resume host measured pagination");

        assert!(json.contains(r#""complete": true"#));
        assert!(json.contains(r#""fragments": ["#));
    }

    #[test]
    fn wasm_resource_read_returns_bytes_and_media_type() {
        let fixture = wasm_contract_fixture();
        let book = open_book(fixture.bytes().to_vec()).expect("open fixture");
        let image = book
            .resources
            .iter()
            .find(|resource| resource.path.as_ref() == "EPUB/images/cover.png")
            .expect("image resource");

        let payload =
            pagelet::epub::read_resource_bytes(fixture.bytes().to_vec(), image.id).expect("read");

        assert_eq!(payload.id, image.id);
        assert_eq!(payload.path.as_ref(), "EPUB/images/cover.png");
        assert_eq!(payload.media_type.as_str(), "image/png");
        assert_eq!(payload.bytes, b"png bytes".to_vec());
    }

    fn wasm_contract_fixture() -> pagelet_testkit::Fixture {
        ValidEpubBuilder::epub3("wasm-contract")
            .xhtml(
                "EPUB/chapter-1.xhtml",
                "Chapter 1",
                r##"<h1 id="intro">Intro</h1><p>Alpha <a href="#intro">again</a>.</p><ul><li>One</li></ul><figure><img src="images/cover.png" alt="Cover"/></figure>"##,
            )
            .entry(FixtureEntry::new(
                "EPUB/images/cover.png",
                "image/png",
                b"png bytes".to_vec(),
            ))
            .build()
    }

    fn escape_xml(value: &str) -> String {
        value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;")
    }
}
