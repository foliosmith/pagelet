# ADR 0007: Cache Versioning And Invalidation

- Status: Accepted
- Date: 2026-06-21
- Deciders: pagelet maintainers

## Context

pagelet will cache chapter parsing, computed styles, shaped text, lines, pages,
and possibly serialized disk artifacts. These caches are derived data. Incorrect
reuse can produce wrong pages, stale hit maps, or broken reading anchors.

Cache validity cannot rely only on crate semver because parser, layout, wire,
and cache schema versions may change independently.

## Decision

Every cache layer has an explicit key and version. Cache keys include content
hashes and the algorithm/configuration inputs needed by that layer.

Disk cache is disposable derived data and is not part of user backup state.
When a migration is not provided, cache schema or algorithm version changes
must invalidate affected entries.

## Consequences

Wrong-cache reuse is treated as a correctness bug. The engine can selectively
reuse expensive layers while invalidating only what changed.

Tests and diagnostics must expose cache hit/miss behavior, version components,
and debug paths to disable caches.

The cost is more key construction code and more explicit invalidation logic.

## Alternatives Considered

Using a single global engine version would be simple but would invalidate too
much or too little.

Letting the host own page caches would blur ownership and make backup semantics
unsafe.

## Follow-up

- Define key inputs for ChapterIR, style, shape, line, and page caches.
- Add cache version fields to engine version metadata.
- Add tests for invalidation on content, font, viewport, and policy changes.
