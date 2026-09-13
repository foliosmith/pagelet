# Changelog

All notable user-visible changes to pagelet are documented here.

The project follows semantic versioning for crates and separately versions the
C ABI, wire protocol, cache schema, parser algorithm, and pagination algorithm.

## Unreleased

### Added

- Bootstrapped the Cargo workspace and single public `pagelet` crate skeleton.
- Added project license, security, contribution, and license-policy documents.
- Added mapped W3C, EPUBCheck, corpus dashboard, property/fuzz, nightly, and release-gate automation.
- Added same-boundary Dart/Rust performance comparison tooling and an incremental M2 Pro baseline.
- Fixed generated EPUB fixtures to keep `mimetype` first and emit EPUBCheck-valid package/navigation metadata.
- Added feature-gated OpenType font parsing as the first NativeShaping boundary.
