# Compatibility Ledger

This ledger is the source of truth for EPUB feature support. Each supported,
limited, rejected, or intentionally deferred behavior gets a stable feature ID
and at least one test ID before it is treated as release behavior.

## Support Status

| Status | Meaning |
|---|---|
| `Supported` | Parsed, represented, laid out, and exposed through diagnostics or output contracts as expected. |
| `SupportedWithLimitations` | Works for the documented subset, with explicit known limitations. |
| `ParsedNotRendered` | Parsed into internal state or diagnostics, but not yet represented in layout output. |
| `UnsupportedDiagnosed` | Not supported, and inputs produce a stable diagnostic instead of silent loss. |
| `RejectedForSecurity` | Rejected by policy or resource limits before normal parsing or layout. |

## Ledger

| Feature ID | Specification section | Support status | Parser behavior | Layout behavior | Diagnostics | Test IDs | Known limitations |
|---|---|---|---|---|---|---|---|
| `EPUB-PKG-001` | EPUB package document | `SupportedWithLimitations` | Reads the package entry identified by `META-INF/container.xml`. | Establishes spine order for downstream layout. | Emits package/container diagnostics for invalid inputs. | `fixture:minimal-epub3`, `fixture:epub2-with-ncx` | Full OPF metadata normalization is not implemented yet. |
| `EPUB-SEC-001` | Resource processing | `RejectedForSecurity` | Applies deterministic resource-limit checks before expensive processing. | No layout output is produced for rejected inputs. | Emits resource-limit diagnostics. | `fixture:zip-bomb-like`, `fixture:huge-image` | Current generated fixtures only simulate the policy surface. |

## Entry Template

| Feature ID | Specification section | Support status | Parser behavior | Layout behavior | Diagnostics | Test IDs | Known limitations |
|---|---|---|---|---|---|---|---|
| `EPUB-AREA-NNN` | Spec name and section | `Supported` | Parser contract. | Layout contract. | Stable diagnostic codes. | Fixture, corpus, W3C, EPUBCheck, fuzz, or regression IDs. | Explicit limitations or `None`. |
