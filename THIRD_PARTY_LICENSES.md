# Third-Party Licenses

This file tracks third-party code, generated assets, corpora, fixtures, and
runtime dependencies that require attribution or redistribution notes.

## Current State

Task 0.1.2 adds no third-party runtime dependencies. The workspace currently
contains only first-party crates licensed as `MIT OR Apache-2.0`.

## Maintenance Rules

- Add every non-trivial third-party source, fixture, and generated corpus entry
  before merging the change that introduces it.
- Record package name, version or commit, source URL, license, attribution
  requirement, and redistribution constraints.
- Keep `deny.toml` in sync with the license set allowed by this document.
- Release artifacts must include an updated license report.
