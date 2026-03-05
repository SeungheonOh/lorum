# 14 — Cycle 1 RC Test Report and Defect Ledger

## Report metadata

- Cycle: 1 (AI connectors/auth/models backend foundation)
- Date: 2026-03-04
- Scope crates:
  - `lorum-ai-contract`
  - `lorum-ai-testkit`
  - `lorum-ai-auth`
  - `lorum-ai-models`
  - `lorum-ai-connectors`

---

## Executed quality gates

## Gate 1 — formatting

Command:

```bash
cargo fmt --all -- --check
```

Result: PASS

## Gate 2 — linting (warnings denied)

Command:

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

Result: PASS

## Gate 3 — full workspace tests

Command:

```bash
cargo test --workspace
```

Result: PASS

### Test totals (from latest workspace run)

- Unit tests passed: 103
- Unit tests failed: 0
- Ignored tests (secret-gated smoke scaffolding): 4
  - `lorum-ai-auth/tests/live_smoke.rs`: 1 ignored
  - `lorum-ai-connectors/tests/live_smoke.rs`: 3 ignored
- Doc tests: 0 failures

---

## Regression inventory status

- Contract surface tests: PASS (`lorum-ai-contract`)
- Fixture and sequence invariants: PASS (`lorum-ai-testkit`)
- Auth precedence/refresh/ranking/sqlite tests: PASS (`lorum-ai-auth`)
- Model merge/cache/discovery tests: PASS (`lorum-ai-models`)
- Connector streaming/parser/codex transport tests: PASS (`lorum-ai-connectors`)
- Live smoke scaffolding: PRESENT and gated behind env + ignore attributes

---

## Defect ledger (RC cut)

## Open defects by severity

| Severity | Count | Notes |
|---|---:|---|
| P0 | 0 | none |
| P1 | 0 | none |
| P2 | 0 | none currently tracked in-cycle |
| P3 | 2 | follow-up hardening items below |

## P3 follow-up items

1. **Live provider execution not run in this environment**
   - Status: open
   - Reason: smoke tests are intentionally secret-gated and ignored by default.
   - Path(s):
     - `crates/lorum-ai-auth/tests/live_smoke.rs`
     - `crates/lorum-ai-connectors/tests/live_smoke.rs`
   - Next action:
     - Run with `OMP_LIVE_SMOKE=1` and provider secrets in controlled environment.

2. **24h soak/fault campaign not executed in this run**
   - Status: open
   - Reason: this RC report covers deterministic gate run and smoke scaffolding; long-duration campaign remains operational follow-up.
   - Next action:
     - Schedule dedicated soak window with repeated streaming, refresh cycling, and cache invalidation loops.

---

## Controlled cleanup compliance (M6 policy)

For changes included in this RC window:

- design-note artifacts added:
  - `12_CYCLE1_SPEC_LOCK_AND_HARDENING.md`
  - `13_PROVIDER_ERROR_MAPPING_COMPAT.md`
- regression and contract tests: PASS
- no unauthorized compatibility-surface changes introduced during hardening window

---

## Cycle 1 RC gate summary

Go/No-Go recommendation: **GO for Cycle 1 backend handoff** with operational follow-ups tracked as P3.

Rationale:

- All mandatory build/test/lint gates are green.
- Contract and precedence semantics are explicitly locked.
- Auth/model/connector surfaces are reusable for Cycle 2 runtime integration.
