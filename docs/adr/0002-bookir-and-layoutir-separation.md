# ADR 0002: BookIR And LayoutIR Separation

- Status: Accepted
- Date: 2026-06-21
- Deciders: pagelet maintainers

## Context

EPUB package parsing and pagination have different stability and invalidation
rules. Package metadata, manifest, spine, navigation, resources, semantic
nodes, source ranges, and links can be derived before layout. Page fragments,
line breaks, hit testing, and page scenes depend on viewport, typography, text
measurement, and pagination policy.

Combining these layers would make every layout setting change look like a book
parse change and would encourage hosts to copy large book objects across FFI.

## Decision

pagelet separates document IR from layout IR.

BookIR and ChapterIR represent package-level and chapter-level semantic data:
metadata, manifest, spine, navigation, resources, semantic nodes, text pools,
anchors, links, and source maps.

LayoutIR and page scene data represent computed style, boxes, fragments, line
breaks, page breaks, hit maps, and anchor maps produced for a specific layout
configuration.

## Consequences

Parsing can be lazy and chapter-scoped. Layout can be invalidated by viewport,
font, text metrics, and pagination settings without reparsing package data.

The host can store stable reading anchors rather than page numbers. Cache keys
can be precise because parser versions and layout versions are independent.

The cost is more explicit translation between semantic nodes, computed style,
and layout fragments.

## Alternatives Considered

A single tree carrying parse and layout data would be simpler initially but
would couple parsing, styling, and pagination invalidation.

Returning a full host-side book tree would make host UI code convenient but
would violate the boundary that large EPUB and layout structures stay in Rust.

## Follow-up

- Keep parser output free of viewport-dependent fields.
- Keep layout code independent from ZIP, OPF, and navigation parsing.
- Add golden serializers that can compare BookIR/ChapterIR separately from
  layout output.
