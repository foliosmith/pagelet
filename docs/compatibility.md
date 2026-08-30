# Compatibility Ledger

`tests/spec/support-matrix.toml` is the machine-readable feature inventory; this
ledger owns the corresponding behavior, diagnostics, tests and limitations.
`cargo xtask manifests lint` rejects inventory IDs missing here. Each supported,
limited, rejected, or intentionally deferred behavior gets a stable feature ID
and at least one test ID before it is treated as release behavior.

## Support Status

| Status | Meaning |
|---|---|
| `Supported` | Parsed, represented, laid out, and exposed through diagnostics or output contracts as expected. |
| `SupportedWithLimitations` | Works for the documented subset, with explicit known limitations. |
| `ParsedNotRendered` | Parsed into internal state or diagnostics, but not yet represented in layout output. |
| `UnsupportedDiagnosed` | Not supported, and inputs produce a stable diagnostic instead of silent loss. |
| `RejectedForSecurity` | Rejected by policy or resource limits before normal parsing or layout. |

## Ledger

| Feature ID | Specification section | Support status | Parser behavior | Layout behavior | Diagnostics | Test IDs | Known limitations |
|---|---|---|---|---|---|---|---|
| `EPUB-PKG-CONTAINER` | OCF 3.3 §2.5 | `SupportedWithLimitations` | Resolves the selected rootfile from `META-INF/container.xml`. | Establishes the package used for downstream reading order. | Invalid container and unsafe-path diagnostics. | `fixture:minimal-epub3`, `w3c:ocf-package_arbitrary` | Multiple rootfiles use the project selection policy rather than reading-system UI. |
| `EPUB-OCF-ZIP` | OCF 3.3 §4 | `SupportedWithLimitations` | Reads stored and deflate entries lazily with checked offsets and sizes. | Resources are loaded only when parsing or layout requests them. | Archive, compression and resource-limit diagnostics. | `w3c:ocf-zip-comp`, `fuzz:ocf_decode`, `fuzz:ocf_paths` | Encryption and split archives are not supported. |
| `EPUB-PKG-METADATA` | EPUB 3.3 §5.2 | `SupportedWithLimitations` | Exposes identifier, title, language and cover metadata. | Metadata does not affect page geometry except referenced resources. | Invalid package diagnostics. | `w3c:pkg-unique-id`, `fixture:minimal-epub3` | Full metadata refinement and collection semantics are deferred. |
| `EPUB-PKG-SPINE` | EPUB 3.3 §5.7 | `Supported` | Preserves manifest-backed spine order, linearity and page progression direction. | Linear spine items define pagination order. | Missing and invalid idref diagnostics. | `w3c:pkg-spine-order`, `property:layout-invariants` | Activation UI for non-linear items belongs to the host. |
| `EPUB-NAV-TOC` | EPUB 3.3 §7; EPUB 2 NCX | `SupportedWithLimitations` | Parses EPUB 3 nav hierarchy and EPUB 2 NCX fallback. | Navigation targets resolve to document paths and fragments. | Navigation parse and missing-target diagnostics. | `w3c:nav-spine_in-spine`, `fixture:epub2-with-ncx` | Reading-system presentation and activation styling are host concerns. |
| `EPUB-CONTENT-XHTML` | EPUB 3.3 §6.3 | `SupportedWithLimitations` | Maps supported XHTML semantics, source ranges, links and anchors into ChapterIR. | Supported block and inline nodes paginate through the deterministic layout model. | Malformed XHTML and unsupported-feature diagnostics. | `w3c:cnt-xhtml-support`, `fuzz:xhtml_fast`, `fuzz:xhtml_differential` | MathML, scripted behavior and full browser layout are not implemented. |
| `EPUB-CSS-CASCADE` | EPUB 3.3 §6.3.2 | `SupportedWithLimitations` | Applies linked, embedded and inline declarations with specificity and inheritance. | A bounded geometry/text subset affects layout. | Unsupported declaration diagnostics. | `fixture:css-cascade`, `fuzz:css_parse`, `fuzz:css_cascade` | Advanced selectors and browser-complete CSS are outside the engine contract. |
| `EPUB-CSS-WRITING-MODE` | CSS Writing Modes / EPUB 3.3 | `ParsedNotRendered` | Retains `direction` and `writing-mode` in computed style. | Horizontal RTL participates in layout; vertical writing does not. | Compatibility ledger entry; W3C case remains manual. | `w3c:css-epub-writing-mode`, `fixture:rtl` | Vertical writing is deferred to Milestone 7. |
| `EPUB-NOTES-FOOTNOTE` | EPUB 3 Structural Semantics | `SupportedWithLimitations` | Resolves referenced local and external notes and backlinks. | Notes remain semantic flow content. | Missing target and malformed reference diagnostics. | `fixture:footnote-collision`, `fuzz:footnote_resolver` | Page-bottom footnote placement is deferred. |
| `EPUB-RESOURCE-IMAGE` | EPUB 3.3 §6.3 | `SupportedWithLimitations` | Indexes PNG, GIF and JPEG metadata and safe bitmap resources. | Intrinsic and authored dimensions affect image fragments. | Invalid image and resource-limit diagnostics. | `fixture:huge-image`, `fixture:data-uri` | SVG content documents and advanced object fallback are deferred. |
| `EPUB-RESOURCE-LIMITS` | Project security model | `RejectedForSecurity` | Rejects unsafe paths, excessive entries, decompression and diagnostic growth. | Rejected inputs produce no page output. | Stable resource-limit and unsafe-path diagnostics. | `fixture:zip-bomb-like`, `property:path-security`, `fuzz:ocf_paths` | Limits are conservative mobile defaults and remain configurable at the API boundary. |
| `EPUB-SCRIPTING` | EPUB 3.3 §6.4 | `ParsedNotRendered` | Script elements are excluded from visible content. | JavaScript is never executed. | Security policy and W3C N/A mapping. | `w3c:scr-support` | This library is not a complete scripted reading system. |

## Entry Template

| Feature ID | Specification section | Support status | Parser behavior | Layout behavior | Diagnostics | Test IDs | Known limitations |
|---|---|---|---|---|---|---|---|
| `EPUB-AREA-NNN` | Spec name and section | `Supported` | Parser contract. | Layout contract. | Stable diagnostic codes. | Fixture, corpus, W3C, EPUBCheck, fuzz, or regression IDs. | Explicit limitations or `None`. |
