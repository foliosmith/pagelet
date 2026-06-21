# Contributing

pagelet is an independent Rust EPUB parsing and pagination engine. Contributions
should preserve deterministic output, clear crate boundaries, and explicit
security behavior.

## Development Setup

Install the Rust toolchain declared in `rust-toolchain.toml`, then run:

```sh
cargo check --workspace
cargo fmt --all -- --check
```

When `cargo-deny` is installed, validate license policy with:

```sh
cargo deny check licenses
```

## Internal Boundaries

The repository publishes one Rust library crate, `pagelet`. Internal dependency
direction still follows the architecture plan, but those boundaries are modules
inside the `pagelet` crate:

```text
core
   ↑
document
   ↑             ↑
epub          text
          \       /
          layout
                 ↑
          engine
             ↑       ↑
          ffi     cli
```

Rules:

- `epub` does not depend on Flutter, Dart, FRB, or layout internals.
- `layout` does not parse OPF, NCX, ZIP, or XHTML containers.
- `ffi` owns adapter safety and does not implement EPUB or pagination
  algorithms.
- Cross-language data belongs in the wire boundary.
- Do not publish internal implementation crates without a new ADR.

## Tests and Fixtures

- Add focused unit tests for narrow behavior.
- Use generated fixtures for EPUB edge cases instead of checked-in opaque test
  files whenever possible.
- Real-world corpus entries must record source, hash, license, and expected
  feature coverage before use in CI.
- Golden output must be normalized and deterministic.

## Benchmarks

Performance work must identify the fixture, profile stage, target platform, and
budget being changed. Do not merge algorithm changes that improve one corpus by
silently regressing another stable corpus class.

## Pull Requests

- Keep PRs scoped to one task or one coherent behavior change.
- Update the task or roadmap document that defines the work.
- Include validation commands and relevant output in the PR description.
- Document wire, cache, parser, or pagination version changes explicitly.
