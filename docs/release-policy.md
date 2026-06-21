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
