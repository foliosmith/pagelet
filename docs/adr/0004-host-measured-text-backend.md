# ADR 0004: Host-Measured Text Backend

- Status: Accepted
- Date: 2026-06-21
- Deciders: pagelet maintainers

## Context

Text shaping and measurement are expensive to implement correctly across
platform fonts, scripts, fallbacks, and host rendering stacks. pagelet still
needs deterministic pagination, batched FFI, and a future path to native
shaping.

The initial host environment already has text measurement capabilities. The
engine should not cross FFI per glyph, per line, or per node.

## Decision

The first text backend is host-measured. pagelet defines a `TextBackend`
contract and batched measurement protocol. The host receives paragraph-level or
run-level measurement batches and returns deterministic metrics for a given
backend id, font set, locale, direction, and text input.

Native shaping remains a later optional backend and must satisfy the same
backend contract.

## Consequences

The engine can ship earlier while preserving the correct boundary: layout stays
in Rust, while platform-specific shaping can be delegated. Batching prevents
chatty FFI behavior.

Cache keys must include the text backend identity and relevant font/locale
inputs. Tests need a fake deterministic backend to make layout output stable.

The cost is that the host must provide a correct measurement adapter before
full pagination can run in that environment.

## Alternatives Considered

Building native shaping first would reduce host dependency but would increase
initial scope substantially.

Doing all text layout in Flutter would undermine the goal that pagination and
page state stay in Rust.

## Follow-up

- Define the `TextBackend` trait and measurement batch wire contract.
- Add a fake deterministic text backend in testkit.
- Add profiling to decide when native shaping is worth implementing.
