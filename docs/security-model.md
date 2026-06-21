# Security Model

pagelet treats EPUB files as untrusted input.

## Threats

The engine defends against path traversal, malformed archives, decompression
bombs, excessive resource sizes, deeply nested XML, oversized data URIs, CSS
import abuse, diagnostic floods, and layout fragmentation attacks.

## Resource Limits

Resource limits are part of the public engine configuration. Exceeding a limit
must produce a stable error or diagnostic and must not continue expensive work.

## Unsafe Policy

The crate forbids unsafe code by default. Any future exception requires an ADR,
localized encapsulation, safety comments, tests, and independent review.
