# ADR 0005: Break Token Pagination

- Status: Accepted
- Date: 2026-06-21
- Deciders: pagelet maintainers

## Context

Reflowable EPUB pagination must support incremental work, cancellation,
prefetch, cache reuse, and stable reading anchors. Saving page numbers is not
stable because a page index changes when viewport, font, CSS, or parser output
changes.

The engine needs a compact representation of where pagination can resume.

## Decision

Pagination uses break tokens and page fingerprints. A page is derived from a
layout configuration, starting break token, content dependencies, and break
policy. The page output records ending break tokens and anchor ranges.

Reading position is stored as a stable semantic anchor, not a page number.

## Consequences

Pagination can be resumed, prefetched, cancelled, and partially invalidated.
If a new run produces matching fingerprints and compatible break tokens,
downstream page results can be reused.

Tests can assert that page fragments do not overlap, do not drop text, and can
round-trip anchors through page output.

The cost is that break tokens become part of the engine contract and must be
versioned with parser and pagination algorithms.

## Alternatives Considered

Saving page numbers is simple but breaks under nearly every layout setting
change.

Paginating the whole book eagerly would simplify navigation but would increase
latency, memory use, and cancellation complexity.

## Follow-up

- Define break token data structures and versioning.
- Add page fingerprint tests for deterministic output.
- Add cache invalidation rules based on break token compatibility.
