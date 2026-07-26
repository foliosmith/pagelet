# FFI Contract

This document defines the native host boundary for pagelet adapters.

## Handles

FFI handles are opaque identifiers owned by pagelet. Hosts must not infer memory
layout, reuse released handles, or share handles across incompatible runtimes.

The Rust registry issues process-unique, type-tagged generation tokens for
`EngineHandle`, `BookHandle`, `ChapterHandle`, `LayoutSessionHandle`, and
`RequestHandle`. The encoding is not part of the ABI contract and contains
neither a pointer nor a collection index. Released tokens are never reused
during the process lifetime, so stale handles cannot alias new objects.

Handle release is idempotent. Releasing a book also releases its chapter and
layout-session handles; releasing a layout session also releases and cancels
its request handles. Releasing an engine closes its complete ownership tree.
Use-after-release is rejected as an invalid handle. Debug builds report a
non-empty registry on drop after first releasing its resources.

Every exported control-plane function must use the shared panic boundary.
Existing typed errors pass through unchanged, while a Rust panic becomes an
opaque `InternalError` and its payload is never returned to the host.

## Control Plane

`ControlPlane` is the transport-neutral implementation behind the C and
generated host adapters. It owns one handle registry and exposes:

- `engine_create`, `book_open_path`, and borrowed `book_open_fd`;
- `book_summary`, `book_navigation`, and `chapter_open`;
- `layout_session_create`, `layout_request`, and
  `layout_submit_measurements`;
- `resource_read`, `hit_test`, and `anchor_to_page`;
- `request_cancel` and generic `handle_dispose`.

Opening from a borrowed file descriptor never adopts or closes the descriptor
and restores its original position. A layout session accepts one in-flight
measurement request at a time. Cancelling or submitting that request retires
its handle, while the layout session remains reusable. Accepted page scenes
stay owned by the layout session so hit testing and anchor lookup do not require
the host to send scene data back to Rust.

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
[`schemas/pageletScene/v2.md`](../schemas/pageletScene/v2.md). The frozen v1
contract remains available for explicit capability fallback. Both use a fixed
little-endian envelope with an exact payload length and CRC-32 checksum.

## Host-Measured Text Round Trip

Host adapters paginate in two phases:

1. pagelet prepares one `MeasureBatch` containing every paragraph/run request
   needed by the chapter layout;
2. the host measures that batch with its rendering stack and submits one
   `MeasuredBatch` carrying backend, font-set, request, line, and cluster
   identities;
3. pagelet validates the complete response and resumes layout to a v2
   `PageScene` containing complete measured paragraphs plus clipped paint
   references.

Adapters must not cross FFI once per line or glyph. Missing, duplicate, unknown,
stale, invalid UTF-8, or geometrically invalid results are protocol errors and
must not reach layout or caches.

Renderers replay the complete paragraph using the same backend/font identity,
request parameters, line baselines, and measurement fingerprint. They clip only
to the page fragment clip rectangle; line advance width is not a glyph clip.
