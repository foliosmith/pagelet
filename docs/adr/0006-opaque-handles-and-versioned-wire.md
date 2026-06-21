# ADR 0006: Opaque Handles And Versioned Wire

- Status: Accepted
- Date: 2026-06-21
- Deciders: pagelet maintainers

## Context

Host adapters need access to books, sessions, pages, resources, diagnostics,
and measurement batches. Exposing Rust struct layouts across FFI would make
internal refactors unsafe and would couple host code to memory ownership rules.

The engine also needs compatibility checks for page scene bytes, measurement
batches, cache entries, and future host adapters.

## Decision

FFI and host adapters use opaque handles for engine-owned objects. Cross-host
data uses explicit versioned wire types. Wire schemas are versioned separately
from Rust crate semver, C ABI version, cache schema, parser algorithm version,
and pagination algorithm version.

All wire inputs validate lengths, versions, and enum values before use. FFI
boundaries catch panics and return structured errors.

## Consequences

The engine can reorganize internal structs without breaking host ABI. Hosts can
negotiate wire compatibility and reject unsupported versions deterministically.

The cost is that DTO conversion and schema tests are required. Debugging also
needs good tooling to inspect wire payloads without relying on internal memory
layout.

## Alternatives Considered

Returning Rust structs directly through generated bindings would be convenient
early but would expose unstable layouts and large object graphs.

Using unversioned JSON or ad hoc byte buffers would be easy to prototype but
would make compatibility failures hard to diagnose.

## Follow-up

- Define initial wire version constants.
- Add wire golden tests.
- Document ownership, threading, and handle lifecycle in the FFI contract.
