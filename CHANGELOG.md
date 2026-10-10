# Changelog

All notable user-visible changes to pagelet are documented here.

The project follows semantic versioning for crates and separately versions the
C ABI, wire protocol, cache schema, parser algorithm, and pagination algorithm.

## Unreleased

### Fixed

- Preserve images and links inside paragraphs, headings and inline containers in page scenes, including their source order and surrounding text.
- Advance parser compatibility to 8 so derived caches containing the previously missing images are rebuilt.

## 0.2.0 - 2026-10-09

### Changed

- Rust API: extend chapter, text measurement and layout contracts for inline style runs, authored geometry and host paragraph replay. This is a breaking upgrade from 0.1.0: update struct literals for the new fields, clone `BlockText` and `HeadingNode` where callers previously relied on `Copy`, and migrate `ComputedLayoutStyle` to its physical margin/padding fields.
- Wire and cache: emit PageScene schema v3 while retaining explicit v1/v2 decoding and projection. Derived caches must compare the complete `EngineVersions` tuple; parser/style/text/pagination/scene/cache versions are now `7/4/2/7/3/2`, and mismatched cached data must be rebuilt.

### Added

- Rust API: book and layout sessions, progressive page ranges, incremental repagination, typed opaque handles and a native C ABI with owned byte buffers.
- Rust text: optional OpenType font parsing through `native-shaping`; native bidi, shaping, font fallback and glyph rendering remain deferred.
- Flutter: version 0.2.0 of the host adapter, with batched paragraph measurement, PageScene v1-v3 decoding, on-demand resources, typed book summaries and a minimal interactive reader example.
- CLI and verification: chapter/layout inspection, mapped W3C and EPUBCheck gates, corpus dashboards, property/fuzz checks and release verification tooling.
- Performance: pinned incremental benchmarks and same-boundary Dart/Rust real-book parse observations; equivalent first-page and warm-repagination targets remain unverified.

### Fixed

- Include readable descendants of unsupported XHTML elements in chapter text while keeping non-content elements excluded.
- Decode XML decimal and hexadecimal character references in text and attributes without double-decoding metadata.
- Index XHTML parents once per chapter to avoid repeated whole-tree scans during style resolution.
- Exclude non-content elements before block, inline and external-footnote conversion so script, style and template text cannot reach page scenes.
- Advance the parser compatibility version to 7 so derived caches cannot reuse text parsed under the previous character-reference and non-content rules.
- Harden CSS at-rule parsing and normalize RTL trailing whitespace clusters in the Flutter measurement bridge.
- Preserve authored image geometry, inline anchors and page-scene link identity.

### Limitations

- Rust APIs remain pre-alpha. Full browser CSS, vertical writing, page-bottom footnotes, fixed-layout fidelity and native shaping are not complete.
- Flutter smoke builds validate compilation on Android, iOS Simulator, macOS and Windows; native library distribution and device runtime acceptance remain host responsibilities. The adapter is not published to pub.dev.

## 0.1.0

### Added

- Initial deterministic EPUB parsing and pagination engine, Cargo workspace and single public `pagelet` crate.
- Project license, security, contribution and license-policy documents.
