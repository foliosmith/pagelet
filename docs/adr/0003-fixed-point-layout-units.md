# ADR 0003: Fixed-Point Layout Units

- Status: Accepted
- Date: 2026-06-21
- Deciders: pagelet maintainers

## Context

Pagination must be deterministic across runs and platforms. Floating-point
rounding differences can change line breaks, page breaks, and hit testing near
boundary conditions. Page cache keys also need stable values that survive
serialization and comparison.

The plan defines `LayoutUnit` as 1/64 logical pixel.

## Decision

Internal layout uses a fixed-point `LayoutUnit` with 1/64 logical pixel
precision. FFI and public APIs may accept logical pixel values, but values are
quantized when they enter the layout kernel. Output may expose both fixed units
and logical pixel conversions where useful.

## Consequences

Break decisions, fingerprints, and cache keys can be deterministic. Tests can
compare exact layout units instead of tolerant float ranges for internal
results.

Precision is sufficient for reading layouts while keeping arithmetic and
serialization compact. The cost is that conversion boundaries must be explicit,
and public APIs must document quantization behavior.

## Alternatives Considered

Using `f32` or `f64` everywhere would be simpler but would make deterministic
pagination and cross-platform regression tests weaker.

Using a finer fixed-point scale would reduce quantization error but increase
the chance of overflow and unnecessary precision in cache keys.

## Follow-up

- Add `LayoutUnit` arithmetic and conversion tests.
- Require layout fingerprints and page cache keys to use quantized values.
- Document public API rounding behavior before the first crates.io release.
