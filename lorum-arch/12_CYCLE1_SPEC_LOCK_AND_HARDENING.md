# 12 — Cycle 1 Spec Lock and Hardening Record (Rust AI Stack)

## Scope

This document locks the implemented Cycle 1 backend surface for:

- `lorum-ai-contract`
- `lorum-ai-auth`
- `lorum-ai-models`
- `lorum-ai-connectors`
- `lorum-ai-testkit`

It defines compatibility surfaces, regression harness use, and controlled cleanup rules for M6.

---

## Locked compatibility surfaces

## A) Event and message contracts

From `lorum-ai-contract`:

- `ApiKind` serialized string values are contract-level and immutable during Cycle 1.
- `AssistantContent` tagged union tags are immutable:
  - `text`, `thinking`, `tool_call`
- `AssistantMessageEvent` tagged union tags are immutable:
  - `start`, `text_start`, `text_delta`, `text_end`,
  - `thinking_start`, `thinking_delta`, `thinking_end`,
  - `tool_call_start`, `tool_call_delta`, `tool_call_end`,
  - `done`, `error`
- `StopReason` values are immutable:
  - `stop`, `length`, `tool_use`, `error`, `aborted`

## B) Auth resolution precedence

From `lorum-ai-auth` resolver path:

1. runtime override
2. persisted API key credential
3. OAuth credential (with refresh)
4. env fallback
5. custom fallback resolver

Order changes are not allowed in Cycle 1.

## C) Model merge precedence

From `lorum-ai-models` manager path:

1. static
2. models.dev source
3. cache
4. dynamic source

Dynamic remains highest precedence for overlapping model IDs.

## D) Codex transport behavior

From `lorum-ai-connectors` Codex adapter:

- preferred transport: websocket
- fallback transport: sse
- provider session state persisted per `(session_id, provider_api)`
- websocket disable flag set on fallback path and respected on next call

---

## Regression harness glue

`lorum-ai-testkit` exposes:

- `run_regression_suite(fixtures: &[StreamFixture]) -> RegressionReport`
- fixture loaders and sequence validators
- deterministic snapshot hash generation

Regression report output fields:

- total/passed/failed counts
- per-fixture pass/fail
- per-fixture snapshot hash
- per-fixture error list

This is the required baseline harness for contract/golden checks in Cycle 1.

---

## Controlled cleanup policy

Any cleanup after lock must satisfy all conditions:

1. Design note included in PR description with:
   - current behavior
   - proposed behavior
   - reason
2. Regression proof:
   - full `run_regression_suite` pass for baseline fixtures
   - no contract test failures
3. Explicit sign-off in milestone gate review

No silent refactors that alter serialized tags, precedence order, or retry behavior are permitted.

---

## Mandatory validation commands (M6)

Run from repo root:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Optional focused checks:

```bash
cargo test -p lorum-ai-contract
cargo test -p lorum-ai-auth
cargo test -p lorum-ai-models
cargo test -p lorum-ai-connectors
cargo test -p lorum-ai-testkit
```

---

## Exit criteria

Cycle 1 hardening is complete only when all are true:

- Full workspace fmt/clippy/tests are green.
- Contract and regression harness tests are green.
- No open P0/P1 defects for AI/auth/models/connectors.
- This document remains aligned with implemented behavior.
