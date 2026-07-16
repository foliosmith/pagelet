# Third-Party Licenses

This file tracks third-party code, generated assets, corpora, fixtures, and
runtime dependencies that require attribution or redistribution notes.

## Current State

The public `pagelet` crate uses `miniz_oxide 0.8.9` under `MIT OR Apache-2.0`.
Private development automation uses `sha2 0.10.9` under `MIT OR Apache-2.0`;
its transitive Rust dependencies are pinned in `Cargo.lock` and checked by
`cargo deny`.

## External Standards Artifacts

- W3C EPUB Tests commit `d707d58cec8518d3cb7cbbe061c8be444cf1ed24`
  comes from <https://github.com/w3c/epub-tests>. Documents and EPUB files use
  the W3C Software and Document License; generator code uses the W3C Software
  Notice and License.
- EPUBCheck `5.3.0` comes from <https://github.com/w3c/epubcheck> and uses the
  BSD-3-Clause license.

These archives are downloaded into an ignored external cache for testing and
are not redistributed in the pagelet crate or repository.

## Maintenance Rules

- Add every non-trivial third-party source, fixture, and generated corpus entry
  before merging the change that introduces it.
- Record package name, version or commit, source URL, license, attribution
  requirement, and redistribution constraints.
- Keep `deny.toml` in sync with the license set allowed by this document.
- Release artifacts must include an updated license report.
