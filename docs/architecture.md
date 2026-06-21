# Architecture

pagelet is a single public Rust crate with internal modules for EPUB parsing,
document IR, text measurement contracts, layout, pagination, wire DTOs, FFI
adapters, CLI tooling, and test support.

## Boundaries

The published crate is `pagelet`. Internal boundaries stay as modules until a
future ADR changes the release strategy. Hosts integrate through Rust APIs,
wire protocols, FFI, CLI, or future WASM adapters without owning engine state.

## Data Flow

1. Archive and container discovery locate the package document.
2. EPUB package, manifest, spine, navigation, and resources become document IR.
3. Text shaping and measurement are delegated to a host-provided backend.
4. Layout produces stable anchors, break tokens, diagnostics, and page scenes.
5. Wire and FFI layers expose versioned, ownership-safe transfer formats.

## Determinism

Parsing, pagination, cache keys, fixtures, and golden output must be stable for
the same input, configuration, text backend fingerprint, and resource limits.
