# FFI Contract

This document defines the native host boundary for pagelet adapters.

## Handles

FFI handles are opaque identifiers owned by pagelet. Hosts must not infer memory
layout, reuse released handles, or share handles across incompatible runtimes.

## Ownership

Inputs crossing FFI are borrowed only for the duration of the call unless the
API explicitly copies or adopts them. Outputs use versioned wire buffers or
opaque handles with explicit release functions.

## Threading

Foreground work must be cancellable. Background work must respect engine worker
configuration and must not call host text measurement APIs from unmanaged
threads unless the backend contract allows it.

## Wire Versioning

Every wire payload carries a schema version. Incompatible cache, wire, or handle
changes require a documented version bump and migration or invalidation policy.

The active binary contract is documented in
[`schemas/pageletScene/v1.md`](../schemas/pageletScene/v1.md). It uses a fixed
little-endian envelope with an exact payload length and CRC-32 checksum.

## Host-Measured Text Round Trip

Host adapters paginate in two phases:

1. pagelet prepares one `MeasureBatch` containing every paragraph/run request
   needed by the chapter layout;
2. the host measures that batch with its rendering stack and submits one
   `MeasuredBatch` carrying backend, font-set, request, line, and cluster
   identities;
3. pagelet validates the complete response and resumes layout to `PageScene`.

Adapters must not cross FFI once per line or glyph. Missing, duplicate, unknown,
stale, invalid UTF-8, or geometrically invalid results are protocol errors and
must not reach layout or caches.
