# pagelet

pagelet is a deterministic, embeddable EPUB parsing and pagination engine
written in Rust.

The project is built as an independent engine. Flutter, CLI, C ABI, and future
WASM integrations are host adapters; EPUB parsing, document IR construction,
layout, pagination, diagnostics, and cache decisions stay in Rust.

## Status

pagelet is in the `0.x` architecture and implementation phase:

- Rust APIs may change;
- cache schemas are not stable across minor versions;
- wire protocols are explicitly versioned;
- reflowable EPUB is the primary target;
- DRM, JavaScript execution, fixed-layout fidelity, PDF generation, and full
  browser CSS compatibility are not first-version goals.

Unsupported features must be reported through diagnostics or capability
reports, not silently discarded.

## Workspace

```text
crates/pagelet           public Rust library crate
```

`pagelet` is the only Rust library package published to crates.io. Internal
engine boundaries such as core types, document IR, EPUB parsing, text
measurement, layout, engine orchestration, wire DTOs, and test helpers live as
modules inside that crate until a future ADR changes the release strategy.

## Quick Start

```sh
cargo check --workspace
cargo fmt --all -- --check
cargo deny check licenses
```

The minimum supported Rust version is declared in `Cargo.toml` and currently
set to Rust 1.80. The local development toolchain is pinned by
`rust-toolchain.toml`.

## External Standards Tools

Pinned W3C EPUB Tests and EPUBCheck distributions are described by
`tests/corpus-manifest.toml`. Download them explicitly, then keep validation
offline:

```sh
cargo xtask external sync --locked
cargo xtask external verify
```

Artifacts default to `target/pagelet-external` and are not committed. Set
`PAGELET_EXTERNAL_ROOT` when CI or a shared local cache uses another location.

## Licensing

pagelet is licensed under either of:

- MIT, see `LICENSE-MIT`;
- Apache License 2.0, see `LICENSE-APACHE`.

Third-party notices are tracked in `THIRD_PARTY_LICENSES.md`.
