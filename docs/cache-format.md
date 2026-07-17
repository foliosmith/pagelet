# Cache Format

pagelet caches derived parsing and layout artifacts only when the cache key can
fully describe the inputs that affect the result.

## Keys

Cache keys include the pagelet version, cache schema version, publication
fingerprint, resource fingerprints, layout configuration, resource limits, text
backend ID, and font set fingerprint.

Cached v2 page scenes additionally bind every paragraph ID to its complete
request and measurement fingerprints. A mismatch in backend, font set,
request, or measurement identity rejects text replay and triggers remeasurement.

## Schema

Cache records use compact, versioned data. Records must be validated before use
and rejected without panic when a schema, fingerprint, or limit does not match.

## Invalidation

Cache entries are invalidated when publication bytes, parsing policy, layout
configuration, text metrics, wire schema, cache schema, or security limits
change.
