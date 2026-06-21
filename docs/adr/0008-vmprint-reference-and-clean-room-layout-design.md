# ADR 0008: VMPrint Reference And Clean-Room Layout Design

- Status: Accepted
- Date: 2026-06-21
- Deciders: pagelet maintainers

## Context

VMPrint is a relevant layout engine reference for the product boundary between
layout output and rendering. Its repository also states that parts of its core
execution architecture are patent-pending. pagelet needs a deterministic
fragmentation and pagination kernel without copying protected or project-
specific implementation mechanisms.

The project must be able to explain what was learned from references and what
was independently designed.

## Decision

pagelet may use VMPrint as a high-level reference for output boundaries, such
as producing positioned boxes or renderer-independent page data. pagelet must
not copy VMPrint's claimed execution mechanisms, including actor architecture,
simulation clock, transaction signal model, or single-pass loop dependencies.

pagelet layout design is documented and implemented independently around
semantic IR, text measurement batches, fixed layout units, break tokens,
deterministic page building, and explicit cache invalidation.

## Consequences

The project can still learn from public design ideas while preserving a clear
clean-room boundary for the implementation. Commercial release review has a
specific ADR to inspect.

The cost is stricter documentation discipline. Contributors must not port code
or detailed algorithms from VMPrint into pagelet.

## Alternatives Considered

Ignoring VMPrint entirely would reduce legal concern but would discard useful
public product-boundary lessons.

Porting VMPrint concepts directly would increase intellectual-property risk
and conflict with pagelet's independent engine goals.

## Follow-up

- Keep layout implementation notes focused on pagelet's own data flow.
- Review this ADR before commercial release.
- Add contributor guidance that reference material must not be copied into the
  codebase.
