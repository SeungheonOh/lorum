# 20 — Phase 2B Detailed Implementation Blueprint (lorum-agent-core + lorum-ui-core)

## Goal

Provide a concrete, file-level implementation plan for Phase 2B so work can start immediately without ambiguity.

This blueprint converts `19_CORE_UI_FIRST_REPLAN.md` from policy to execution details.

---

## 1) Current implementation gaps (concrete)

## 1.1 `lorum-agent-core` gaps

Target file: `crates/lorum-agent-core/src/lib.rs`

Observed gaps:

1. **Cancellation semantics are not explicit in API**
   - `TurnRequest` has no cancellation token or policy.
   - `TurnEngine::run_turn` cannot express cooperative cancellation checkpoints.

2. **Stream event projection is lossy and overly permissive**
   - `AssistantToRuntimeEventAdapter` accepts all provider stream events but only maps delta events into `RuntimeEvent::AssistantStreamDelta`.
   - start/end boundaries are ignored, making invariant proofs weaker.

3. **Terminal-state safety is validated only post-hoc**
   - `validate_turn_event_order` is called at end; violations are not blocked incrementally as events are emitted.

4. **Error terminal path is too coarse**
   - provider failure path emits `RuntimeError` and returns provider error, but no structured internal classification for cancellation vs transport/auth/model failures.

## 1.2 `lorum-ui-core` gaps

Target file: `crates/lorum-ui-core/src/lib.rs`

Observed gaps:

1. **No sequence/order guard in reducer**
   - reducer applies events in received order but does not validate monotonic sequence per turn.

2. **Terminal-state bookkeeping is implicit**
   - completed turns are derived from buffer removal only.
   - no explicit tracking to reject post-terminal deltas for already closed turns.

3. **Session switch hygiene is incomplete**
   - active session changes, but no guardrails for stale-turn events crossing session boundaries.

4. **Error state model is too thin**
   - only stores `last_error` message string; no code/turn/session context for deterministic diagnostics.

## 1.3 Runtime↔UI contract tightening gaps

Target files:
- `crates/lorum-domain/src/lib.rs`
- `crates/lorum-runtime/src/lib.rs`

Observed gaps:

1. **`RuntimeEvent` lacks explicit contract version/freeze marker** for downstream compatibility checks.
2. **No runtime-side assertion for subscriber ordering guarantees** beyond current synchronous fan-out behavior.
3. **No dedicated contract test crate for runtime→ui ordering/terminal invariants** across interleaved sessions.

---

## 2) Phase 2B implementation workstreams

## M2B.1 — `lorum-agent-core` hardening

### Files to change

- `crates/lorum-agent-core/src/lib.rs`
- `crates/lorum-agent-core/tests/turn_state_machine_hardening.rs` (new)
- `crates/lorum-agent-core/tests/cancellation_contract.rs` (new)

### Required code changes

1. **Add cancellation contract to turn API**
   - extend `TurnRequest` with cancellation policy input (lightweight token abstraction or explicit canceled flag callback trait).
   - update `TurnEngine` docs + behavior contract in-code.

2. **Strengthen runtime emission gate**
   - add `TurnEventGuard` internal helper that enforces:
     - monotonic sequence
     - single terminal
     - no post-terminal emissions
   - enforce at emit time, not only end-of-turn.

3. **Structured terminal classification**
   - add `TurnFailureKind` (auth/transport/rate_limited/invalid_response/aborted/internal) for deterministic mapping.
   - preserve external parity behavior while improving internal diagnostics.

4. **Normalize stream projection policy**
   - document and enforce which assistant events are projected to runtime deltas.
   - explicitly ignore non-projected variants via named policy (not silent wildcard behavior).

### Required tests

- `turn_state_machine_hardening.rs`
  - rejects post-terminal emission attempt
  - rejects duplicate terminal attempt
  - preserves strict sequence monotonicity under interleaved provider updates

- `cancellation_contract.rs`
  - cancellation before first provider delta -> aborted terminal
  - cancellation mid-stream -> aborted terminal and no further events
  - cancellation after terminal -> no-op

### Exit criteria

- all new hardening tests pass
- existing agent-core tests pass unchanged
- no clippy warnings

---

## M2B.2 — `lorum-ui-core` reducer hardening

### Files to change

- `crates/lorum-ui-core/src/lib.rs`
- `crates/lorum-ui-core/tests/reducer_order_contract.rs` (new)
- `crates/lorum-ui-core/tests/reducer_terminal_contract.rs` (new)

### Required code changes

1. **Introduce explicit turn reducer state machine**
   - internal enum for turn state: `Open`, `Terminal`.
   - maintain per-turn metadata: last_sequence, session_id, terminal_reason, terminal_seen_at.

2. **Add sequence-order validation at reducer boundary**
   - reject out-of-order or regression events per turn with explicit `UiError` variant.

3. **Reject post-terminal deltas**
   - once terminal is recorded for a turn, any further event for that turn fails deterministically.

4. **Enrich error diagnostics**
   - replace `last_error: Option<String>` with structured error snapshot:
     - `turn_id`
     - `code`
     - `message`
     - `sequence_no`

5. **Session switch guardrails**
   - assert/track active session transitions and expose deterministic policy for events arriving from inactive sessions.

### Required tests

- `reducer_order_contract.rs`
  - sequence regression rejected
  - interleaved turns with valid ordering accepted
  - session switch and valid subsequent turn accepted

- `reducer_terminal_contract.rs`
  - delta after terminal rejected
  - duplicate terminal rejected
  - runtime error event captured as structured diagnostic

### Exit criteria

- reducer hardening tests green
- no regressions in existing ui-core tests
- deterministic state snapshots for repeated replay inputs

---

## M2B.3 — Runtime↔UI contract freeze implementation

### Files to change

- `crates/lorum-domain/src/lib.rs`
- `crates/lorum-runtime/src/lib.rs`
- `crates/lorum-runtime/tests/runtime_ui_contract_freeze.rs` (new)
- `crates/lorum-ui-core/tests/runtime_event_compatibility.rs` (new)

### Required code changes

1. **Add explicit runtime event contract metadata**
   - define contract version constant in domain (e.g., `RUNTIME_EVENT_CONTRACT_VERSION`).
   - include helper for compatibility assertions used by tests/docs.

2. **Add runtime fan-out determinism checks**
   - assert subscriber notification occurs in emitted order for each submitted turn.
   - codify behavior in comments/tests as frozen contract.

3. **Freeze additive-only extension rule**
   - document in code comments and tests that Phase 3 tool events are additive and must not alter chat baseline ordering semantics.

### Required tests

- `runtime_ui_contract_freeze.rs`
  - runtime emits baseline event set in stable order under normal/error/aborted paths
  - subscriber receives identical order to persisted session replay

- `runtime_event_compatibility.rs`
  - ui reducer accepts all frozen baseline events
  - contract version assertion present and stable

### Exit criteria

- contract freeze tests green
- runtime/ui contract behavior documented and machine-checked

---

## M2B.4 — Evidence and sign-off artifact production

### Files to create

- `lorum-arch/21_PHASE2B_AGENT_CORE_HARDENING_REPORT.md`
- `lorum-arch/22_PHASE2B_UI_CORE_HARDENING_REPORT.md`
- `lorum-arch/23_PHASE2B_RUNTIME_UI_CONTRACT_FREEZE.md`
- `lorum-arch/24_PHASE2B_SIGNOFF_AND_DEFECT_LEDGER.md`

### Required content

- failing->passing defect list with root cause and fix references
- test evidence with command outputs and totals
- explicit P0/P1 closure statement
- Phase 3 unblock decision

### Exit criteria

- all four docs exist and are cross-referenced in `00_INDEX.md`
- sign-off includes clear go/no-go statement for Phase 3 implementation start

---

## 3) Execution order (strict)

1. M2B.1 agent-core hardening
2. M2B.2 ui-core hardening
3. M2B.3 runtime↔ui freeze
4. M2B.4 reports/sign-off

Phase 3 remains blocked until step 4 is complete.

---

## 4) Verification commands per milestone

Mandatory for each milestone:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Focused suites to add and run:

```bash
cargo test -p lorum-agent-core --test turn_state_machine_hardening
cargo test -p lorum-agent-core --test cancellation_contract
cargo test -p lorum-ui-core --test reducer_order_contract
cargo test -p lorum-ui-core --test reducer_terminal_contract
cargo test -p lorum-runtime --test runtime_ui_contract_freeze
cargo test -p lorum-ui-core --test runtime_event_compatibility
```

---

## 5) Definition of done for this blueprint

This implementation blueprint is complete when:

- all listed file-level changes are implemented,
- all listed new suites are present and green,
- M2B reports/sign-off artifacts are published,
- Phase 3 unblock decision is explicitly recorded in `24_PHASE2B_SIGNOFF_AND_DEFECT_LEDGER.md`.
