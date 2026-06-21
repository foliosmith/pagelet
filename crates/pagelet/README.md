# pagelet

pagelet is a deterministic, embeddable EPUB parsing and pagination engine
written in Rust.

This crate is the only Rust library package that pagelet publishes to
crates.io. Implementation boundaries such as core types, document IR, EPUB
parsing, text measurement, layout, engine orchestration, and wire DTOs live as
internal modules so the public API can evolve deliberately.

The project is pre-alpha. APIs and cache/wire details may change before 1.0.
