# Security Policy

## Supported Versions

pagelet is pre-1.0 software. Security fixes are applied to the active `main`
branch until the project publishes versioned release support rules.

## Reporting a Vulnerability

Do not open a public issue for suspected vulnerabilities. Contact the
maintainers privately through the repository security advisory flow or the
private contact channel published with the project release.

Please include:

- affected version or commit;
- platform and build profile;
- minimal reproducer or input file when safe to share;
- expected impact and whether the issue is already public.

## Security Boundaries

pagelet treats EPUBs and host-provided wire data as untrusted input. The
project security model requires:

- archive path traversal rejection;
- external XML entity rejection;
- remote resource loading disabled by default;
- JavaScript execution disabled;
- resource limits for archive entries, decompressed bytes, nesting depth, and
  diagnostics;
- validated wire lengths and opaque FFI handles;
- panic boundaries at host adapter edges.

## Unsafe Code

Core parsing and layout crates must use `#![forbid(unsafe_code)]`. Unsafe code
is currently limited to `crates/pagelet/src/ffi/native.rs`. Every unsafe block
must carry a nearby `SAFETY:` explanation, and every public unsafe C ABI
function must document its caller obligations under `# Safety`. The release
policy verifier rejects unsafe code outside that boundary.

## Dependency Checks

License and source policy is enforced with `cargo deny`; RustSec advisories are
checked with `cargo audit`. Both run in normal CI and the release gate. Selected
wire, handle and buffer tests also run under fuzzing, Miri and the nightly
address-sanitizer job.

Run the local policy checks with:

```sh
cargo deny check
cargo audit
cargo xtask release verify
```
