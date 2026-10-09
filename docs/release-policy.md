# Release Policy

pagelet publishes one Rust library crate: `pagelet`.

## Versioning

Rust APIs follow SemVer. During `0.x`, breaking API changes are allowed between
minor versions, but user-visible changes must be documented in `CHANGELOG.md`.

## Wire And Cache Versions

Wire and cache schemas are versioned independently from the crate version.
Breaking schema changes require explicit migration or invalidation behavior.

## Publishing

Before publishing, run formatting, all-target checks, tests, documentation,
bench smoke, package verification, license checks, and compatibility ledger
updates. Do not publish internal module boundaries as separate crates.

`cargo xtask release verify` checks the single-publishable-crate rule, unsafe
boundary, manifests, pinned external artifacts, golden files and generated
corpus. `cargo xtask release dry-run` additionally runs
`cargo publish -p pagelet --dry-run --locked` without requiring a crates.io
token. Formal publishing is intentionally explicit:

```sh
cargo xtask release publish --version 0.2.0
```

It requires a clean checkout at the exact `v0.2.0` tag and a matching
`CHANGELOG.md` release heading. If crates.io accepted a bad release, follow the
tool's recovery message and yank that exact version; never reuse a version.
