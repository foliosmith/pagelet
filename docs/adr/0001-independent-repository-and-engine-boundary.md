# ADR 0001: Independent Repository And Engine Boundary

- Status: Accepted
- Date: 2026-06-21
- Deciders: pagelet maintainers

## Context

pagelet is intended to be a deterministic EPUB parsing and pagination engine,
not a Flutter-only feature implementation. The host application should own UI,
platform permissions, gestures, persistence, and integration behavior. The
engine should own EPUB parsing, semantic IR, layout, pagination, diagnostics,
and cache invalidation.

The project also needs a clear public distribution model. The current release
policy publishes one public Rust library crate, `pagelet`, while internal
modules remain implementation boundaries.

## Decision

pagelet is maintained as an independent Rust repository and engine. Flutter,
CLI, C ABI, and future WASM support are adapters around the engine boundary.

Only the `pagelet` Rust library crate is public on crates.io. Internal
boundaries are represented by modules, and the published `pagelet` package must
be self-contained and must not depend on unpublished path crates.

## Consequences

The engine can be tested, benchmarked, fuzzed, and released independently of a
specific host application. Host adapters remain thin and are not allowed to own
EPUB or pagination algorithms.

The single public crate keeps the user-facing API simple and preserves freedom
to reorganize internal implementation boundaries. The cost is that release
tooling must enforce that internal crates are not accidentally published and
that the public package is self-contained.

## Alternatives Considered

Keeping the engine embedded inside a Flutter package would reduce repository
overhead but would make non-Flutter hosts and deterministic native testing
secondary.

Publishing every internal crate would expose unstable implementation details
and force semver commitments before the engine architecture has settled.

## Follow-up

- Keep `crates/pagelet` as the public library crate.
- Keep internal implementation boundaries as modules unless a future ADR changes
  the release strategy.
- Keep FFI, CLI, and Flutter adapters outside the core engine logic.
