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
