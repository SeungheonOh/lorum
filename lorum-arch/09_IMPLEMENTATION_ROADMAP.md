# 09 — Implementation Roadmap (TypeScript → Rust)

## 1) Program objective and execution constraints

This roadmap is the executable migration program for rebuilding the coding-agent runtime in Rust while preserving behavior contracts captured in:

- `03_TOOL_SYSTEM_AND_RENDERING.md`
- `04_EDITING_AND_PATCHING_ENGINE.md`
- `05_MCP_SKILLS_AND_EXTENSIBILITY.md`
- `06_SUBAGENTS_TASKS_AND_ORCHESTRATION.md`
- `07_TUI_AND_INTERACTION_LAYER.md`
- `08_RUST_TARGET_ARCHITECTURE.md`

### Non-negotiable migration rules

1. Parity-first: behavior/literals/protocol envelopes remain stable until cutover complete.
2. No semantic drift in mixed-runtime period: each session runs one runtime semantics contract, never blended within a session.
3. Contract tests and golden transcripts are release gates, not best-effort checks.
4. Any intentional behavior change requires explicit compatibility review after parity cutover.

---

## 1.1) Revision note — core/ui hardening before tools

Roadmap execution order is revised to insert **Phase 2B (agent-core/ui-core contract hardening)** between Phase 2A and Phase 3.

Authoritative rationale and acceptance details:
- `15_AGENTIC_LOOP_FIRST_REPLAN.md`
- `19_CORE_UI_FIRST_REPLAN.md`
- `20_PHASE2B_AGENT_UI_IMPLEMENTATION_BLUEPRINT.md`

> Rule: Tool runtime work (Phase 3) must not begin until Phase 2B hardening and sign-off gates are green.
---
## 2) Dependency-aware phase graph

```mermaid
graph TD
  P0[Phase 0<\br>Contract Freeze + Golden Baseline] --> P1[Phase 1<\br>Core Domain + Runtime Scaffolding]
  P1 --> P2[Phase 2<\br>AI/Auth/Model Migration]
  P1 --> P2A[Phase 2A<\br>Agentic Loop (Chat-Only) Runtime]
  P2 --> P2A
  P2A --> P2B[Phase 2B<\br>Agent-Core + UI-Core Hardening]
  P2B --> P3[Phase 3<\br>Tool Runtime + Render + Native Bridge]
  P3 --> P4[Phase 4<\br>Edit/Patch/Hashline Migration]
  P1 --> P5[Phase 5<\br>MCP/Capability/Extensibility Migration]
  P1 --> P6[Phase 6<\br>Task/Subagent/Isolation Migration]
  P1 --> P7[Phase 7<\br>TUI/Print/RPC Migration]
  P2 --> P8[Phase 8<\br>Integration + Hardening]
  P2A --> P8
  P2B --> P8
  P4 --> P8
  P5 --> P8
  P6 --> P8
  P7 --> P8
  P8 --> P9[Phase 9<\br>Staged Cutover + Rollbackable Launch]
```

---

## 3) Phase-by-phase execution plan

## Phase 0 — Contract freeze and golden baseline

### Objective
Freeze observable TS behavior and produce baseline artifacts that all Rust phases must match.

### Scope
- Freeze contract literals/protocols/ordering from docs 03–08.
- Build golden event/tool/output corpus from representative scenarios.
- Define parity blocker escalation policy.

### Dependencies
- None.

### Implementation tasks by subsystem
- **Contracts**: capture parity-critical strings and envelope schemas (MCP protocol/version, edit errors, submit warnings, RPC ready sentinel, internal URL errors).
- **Golden harness**: snapshot representative runs for AI streams, tools, edit modes, MCP flows, subagent enforcement, interactive/print/RPC outputs.
- **Baseline catalogs**: task ordering snapshots, capability precedence snapshots, skill ordering snapshots.

### Validation strategy
- Contract checks:
  - Schema diff checks for tool params/results and runtime events.
  - Literal checks for parser-dependent strings.
- Golden checks:
  - Replay TS corpus and persist canonical JSONL/markdown artifacts.

### Risk controls and rollback
- **Risk**: incomplete baseline misses edge behavior.
- **Control**: block Phase 1 until all required scenario classes are represented.
- **Rollback**: if a missing contract is discovered later, add to baseline and re-run all completed phases before proceeding.

### Artifacts produced
- `golden/contracts/` literals and schema envelopes.
- `golden/transcripts/` per-scenario TS snapshots.
- `golden/parity-matrix.json` mapping scenarios to subsystem contracts.

### Exit gate
- All mandatory scenario classes present and reproducible.
- Contract inventory approved for parity lock.

---

## Phase 1 — Core domain/runtime scaffolding

### Objective
Establish Rust workspace, crate boundaries, event model, and runtime composition skeleton required by all downstream teams.

### Scope
- Implement `lorum-domain`, `lorum-agent-core`, `lorum-session`, `lorum-runtime` scaffolding.
- Define core traits from doc 08 and enforce crate layering.
- Add compatibility event/log formats.

### Dependencies
- Phase 0 complete.

### Implementation tasks by subsystem
- **Domain model**: IDs, canonical message blocks, tool lifecycle events, task progress snapshots.
- **Core runtime**: scheduler skeleton (`shared`/`exclusive`), cancellation token hierarchy, event sequencing.
- **Session**: append-only log model, restore/switch scaffolding, compaction boundary.
- **Runtime composition**: startup ordering, mode dispatch shell, config surface for runtime selection.

### Validation strategy
- Unit tests:
  - Event ordering determinism.
  - Scheduler barriers and synthetic tool-result pairing hooks.
- Contract tests:
  - Session event envelope compatibility with golden schemas.
- Static checks:
  - crate dependency graph checks preventing forbidden cross-imports.

### Risk controls and rollback
- **Risk**: leaky boundaries block parallelism.
- **Control**: enforce interface crates and deny direct implementation-crate imports in CI.
- **Rollback**: revert to previous scaffold baseline tag if trait/API churn breaks 2+ downstream teams.

### Artifacts produced
- Workspace crate skeleton and trait contracts.
- Architecture conformance report (actual dependency graph vs target).
- Event compatibility report vs Phase 0 envelopes.

### Exit gate
- All shared contracts compile and are consumed through stable trait seams.
- No unresolved layering violations.

---

## Phase 2 — AI/auth/model migration

### Objective
Port canonical provider streaming, auth resolution lifecycle, OAuth integration, and model discovery/cache semantics.

### Scope
- `lorum-ai-contract`, `lorum-ai-connectors`, `lorum-ai-auth`, `lorum-ai-models`.

### Dependencies
- Phase 1.

### Implementation tasks by subsystem
- **AI stream contract**: normalized events for text/thinking/toolcall lifecycle with terminal done/error.
- **Providers**: Anthropic/OpenAI/Google/Bedrock adapters with canonical stop-reason mapping.
- **Stateful transport**: provider session state store (Codex-style WS reuse/fallback).
- **Auth**: runtime override → persisted key → OAuth refresh path → env fallback → custom fallback resolver.
- **Usage-aware ranking**: least-exhausted credential selection and temporary block/retry behavior.
- **Models**: merge precedence `static -> models.dev -> cache -> dynamic`, authoritative cache semantics.

### Validation strategy
- Provider adapter contract tests against golden stream transcripts.
- Auth precedence and refresh/backoff tests with deterministic fixtures.
- Model merge/cache tests (including stale/non-authoritative behavior).

### Risk controls and rollback
- **Risk**: subtle stream/event mismatch causes downstream render drift.
- **Control**: lock adapter outputs to golden event sequence snapshots.
- **Rollback**: provider-level rollback flag to route affected provider to TS runtime path in mixed-runtime mode.

### Artifacts produced
- Provider parity reports per adapter.
- Auth lifecycle compatibility report.
- Model discovery/cache compatibility report.

### Exit gate
- All targeted providers pass stream parity and auth/model contract suites.

---

## Phase 2A — Agentic loop runtime (chat-only, no tools)

### Objective
Establish an end-to-end agentic conversation loop (user ⇄ assistant turns) before introducing tool execution, so orchestration semantics are proven independently of tool complexity.

### Scope
- `lorum-agent-core` turn engine finalization for pure chat turns
- `lorum-session` persistence for turn append/restore/switch without tool events
- `lorum-runtime` composition path for model invocation + streamed assistant responses (no tool calls)
- mode integration for chat-only operation in interactive/print/RPC surfaces using shared runtime events

### Dependencies
- Phase 1 and Phase 2.

### Implementation tasks by subsystem
- **Turn loop**: deterministic request/response cycle with cancellation and abort semantics for assistant streaming.
- **Session semantics**: persist/reload conversation turns and model/thinking settings without relying on tool lifecycle side effects.
- **Runtime composition**: provider/model resolution wiring for chat-only flows using `lorum-ai-*` outputs from Phase 2.
- **Frontend contract alignment**: ensure interactive/print/RPC consume assistant/user events in stable order even with tools disabled.

### Validation strategy
- Chat-only golden transcript replay (multi-turn, abort, error, model switch).
- Session restore/switch tests proving deterministic replay of assistant content and stop reasons.
- Print/RPC contract tests for chat-only mode (exit codes, ready/event envelopes).

### Risk controls and rollback
- **Risk**: hidden coupling between chat loop and tool lifecycle bleeds into baseline runtime semantics.
- **Control**: explicit `tools-disabled` execution path and dedicated chat-only fixtures run in CI.
- **Rollback**: pin runtime selector to TS chat path for affected modes while preserving Rust Phase 2 artifacts.

### Artifacts produced
- Agentic loop parity report (chat-only).
- Session replay compatibility report (no-tool turn corpus).
- Mode contract report for chat-only operation.

### Exit gate
- Chat-only loop is parity-verified across interactive/print/RPC with no dependency on tool runtime.
- Tool runtime work (Phase 3) is unblocked only after this gate is green.

---
## Phase 2B — Agent-core/UI-core contract hardening

### Objective
Harden `lorum-agent-core` and `lorum-ui-core` semantics and freeze runtime↔UI contracts before adding tool lifecycle complexity.

### Scope
- `lorum-agent-core` cancellation/state-machine hardening
- `lorum-ui-core` reducer/state consistency hardening
- runtime↔ui event contract tightening and freeze artifacts

### Dependencies
- Phase 1, Phase 2, and Phase 2A.

### Implementation tasks by subsystem
- **Agent core hardening**: verify monotonic sequencing, single-terminal guarantees, cancellation propagation, and no post-terminal emissions.
- **UI core hardening**: verify deterministic reducer outcomes, replay consistency, and terminal-state handling under adverse event sequences.
- **Runtime↔UI freeze**: formalize event-order/field guarantees and lock additive-only extension rules for Phase 3 tool events.

### Validation strategy
- Hardening suites for loop cancellation/state-machine invariants.
- Reducer consistency suites under replay/interleaving/failure inputs.
- Contract freeze compatibility checks proving no baseline chat semantics drift.

### Risk controls and rollback
- **Risk**: unstable foundational semantics force rework once tool lifecycle events are added.
- **Control**: block Phase 3 until hardening reports and defect closure gates are complete.
- **Rollback**: keep runtime in chat-only validated path and defer tool integration until contracts are re-frozen.

### Artifacts produced
- Agent-core hardening report.
- UI-core hardening report.
- Runtime↔UI contract freeze note and consolidated defect ledger.

### Exit gate
- No open P0/P1 defects in loop/reducer/contract-hardening scope.
- Formal sign-off artifacts published and indexed.

---
## Phase 3 — Tool runtime + rendering + native bridge migration

### Objective
Port tool execution engine, metadata/wrapper semantics, renderer precedence/fallback behavior, and native-backed search/AST bridging.

### Scope
- `lorum-tool-contract`, `lorum-tool-runtime`, `lorum-tool-render`, `lorum-tool-deferred`, `lorum-native-bridge`.

### Dependencies
- Phase 1, Phase 2A, and Phase 2B.

### Implementation tasks by subsystem
- **Tool contract/runtime**: schema validation, lenient bypass policy, concurrency scheduler, lifecycle event emission.
- **Auto-enrichment**: implicit tool-set additions (`ast_*`, `resolve`, `submit_result`, `exit_plan_mode`).
- **Meta wrapper**: centralized output notices + normalized error rendering.
- **Deferred actions**: pending action store + resolve apply/discard flow.
- **Render pipeline**: tool renderer precedence (tool custom → registry → generic fallback), panic isolation.
- **Native bridge**: glob/grep/ast behavior parity (limits/offsets/sorting/parse errors).

### Validation strategy
- Scheduler parity tests for mixed shared/exclusive calls.
- Lifecycle event sequence parity tests.
- Renderer failure injection tests verifying safe fallback.
- Native behavior tests vs golden fixtures for find/grep/ast_*.

### Risk controls and rollback
- **Risk**: renderer or wrapper mismatch degrades operator trust.
- **Control**: golden render snapshots + explicit failure-injection tests.
- **Rollback**: per-tool runtime fallback map to TS implementations for high-risk tools.

### Artifacts produced
- Tool-event parity report.
- Renderer fallback resilience report.
- Native bridge compatibility report.

### Exit gate
- Tool lifecycle parity validated, including deferred and wrapper behavior.

---

## Phase 4 — Edit/patch/hashline migration

### Objective
Port high-risk mutation subsystem with strict ambiguity/staleness semantics and preview integration.

### Scope
- `lorum-edit-engine` (`replace`, `patch`, `hashline`, normalization/fuzzy/errors/fs adapter).

### Dependencies
- Phase 3 (tool integration + render preview hooks).

### Implementation tasks by subsystem
- **Mode routing**: preserve env/model/global precedence and runtime mode resolution.
- **Replace engine**: exact/fuzzy/ambiguity behavior, disambiguation diagnostics.
- **Patch engine**: diff normalization/parser, single-file guard, hunk strategy progression, overlap/ambiguity fail-fast.
- **Hashline engine**: anchor parse/validation, mismatch remap diagnostics, bottom-up mutation ordering.
- **I/O semantics**: BOM/newline/indentation preservation and fs invalidation hooks.
- **Writethrough boundary**: pluggable fs adapter for LSP-aware writes.

### Validation strategy
- Golden mutation corpus replay (success, ambiguity, stale-anchor, overlap, malformed diff).
- Literal error-string assertions for parser-dependent strings.
- Preview parity tests for replace/patch/hashline visual diffs.

### Risk controls and rollback
- **Risk**: unsafe mutation on ambiguity or stale anchors.
- **Control**: mandatory pre-mutation full validation and no-partial-write assertions.
- **Rollback**: immediate route of edit tool to TS path on any parity blocker in mutation safety class.

### Artifacts produced
- Edit safety compatibility report.
- Error-literal parity report.
- Preview parity snapshot bundle.

### Exit gate
- All mutation safety tests pass with zero semantic drift from golden outputs.

---

## Phase 5 — MCP/capability/extensibility migration

### Objective
Port MCP transports/manager/cache/auth-discovery and extensibility runtime (skills/plugins/custom tools/internal URLs).

### Scope
- `lorum-mcp-protocol`, `lorum-mcp-transport`, `lorum-mcp-client`, `lorum-mcp-manager`, `lorum-capability`, `lorum-extensibility`, `lorum-internal-urls`.

### Dependencies
- Phase 1.

### Implementation tasks by subsystem
- **MCP protocol/transport**: preserve protocol version `2025-03-26`, initialized notification, timeout strings, HTTP session header semantics.
- **MCP manager**: 250ms startup grace, deferred cached-tool fallback, eventual live replacement, epoch-gated subscription post-actions.
- **OAuth discovery/flow**: auth error heuristics, well-known probing order, PKCE + dynamic registration fallback.
- **Capability system**: provider priority first-win dedupe and `_shadowed` diagnostics.
- **Skills/plugins/extensions**: source filtering, collision handling, ordering, override precedence, init guards, conflict skip diagnostics.
- **Internal URLs**: deterministic `mcp://` exact/template tie-break and `skill://` traversal protections.

### Validation strategy
- Protocol tests for initialize/notifications/timeouts.
- Race tests for notification epoch gate (`rollback|ignore|apply`).
- Capability precedence replay tests from golden snapshots.
- Internal URL security tests (absolute path, traversal, unknown skill/resource errors).

### Risk controls and rollback
- **Risk**: MCP startup/regression causes tool unavailability.
- **Control**: deferred cached-tool compatibility and staged reconnect stress tests.
- **Rollback**: runtime switch for MCP stack to TS bridge while keeping Rust core active.

### Artifacts produced
- MCP protocol compatibility report.
- Capability precedence report.
- Extensibility and internal URL parity report.

### Exit gate
- MCP + extensibility + internal URL parity suites pass under race and failure conditions.

---

## Phase 6 — Task/subagent/isolation migration

### Objective
Port task orchestration with submit enforcement, fallback completion semantics, async jobs, and isolation merge flows.

### Scope
- `lorum-task-schema`, `lorum-subagent-executor`, `lorum-task-finalize`, `lorum-task-render`, `lorum-task-isolation`, `lorum-async-jobs`.

### Dependencies
- Phase 1 and Phase 3 (tool runtime integration).

### Implementation tasks by subsystem
- **Schema/validation**: isolation-mode-dependent schema selection; output truncation env parse behavior.
- **Executor lifecycle**: child session creation, event subscription, reminder loop (`MAX_SUBMIT_RESULT_RETRIES=3`), deterministic ordering.
- **Submit enforcement**: multilayer submit_result injection and termination logic.
- **Finalization**: fallback acceptance paths and warning-prefix behavior.
- **Async jobs**: queue limits/retries/retention and progress state transitions.
- **Isolation**: none/worktree/fuse-overlay/fuse-projfs mode resolution with fallback warnings; patch/branch merge semantics.

### Validation strategy
- Submit-result contract tests (valid/malformed/missing/null cases).
- Ordering tests for concurrent task execution and merge order determinism.
- Async state transition and throttling tests (`PROGRESS_COALESCE_MS=150`).
- Isolation conflict/fallback/cleanup scenario tests.

### Risk controls and rollback
- **Risk**: silent acceptance of malformed subagent completion.
- **Control**: explicit enforcement + warning literal tests + reminder-loop assertions.
- **Rollback**: route `task` tool to TS path if submit enforcement or merge correctness parity fails.

### Artifacts produced
- Task orchestration parity report.
- Submit enforcement compliance report.
- Isolation merge reliability report.

### Exit gate
- Task/subagent parity matrix fully green across sync, async, and isolation modes.

---

## Phase 7 — TUI/print/RPC migration

### Objective
Port frontends with reducer-driven deterministic rendering while preserving interaction and non-interactive contracts.

### Scope
- `lorum-ui-core`, `lorum-ui-tui`, `lorum-ui-print`, `lorum-ui-rpc`.

### Dependencies
- Phase 1, plus integration hooks from Phases 2/2A/2B/3/5/6.

### Implementation tasks by subsystem
- **Interactive mode**: controller split, init ordering, event-to-state reducer flow.
- **Tool UX parity**: tool rendering precedence/fallback, read grouping, image fallback behavior.
- **Input handling**: keybindings, submit-path tiers, background mode transitions.
- **Print mode**: text/json output contracts and exit-code semantics.
- **RPC mode**: ready sentinel, event stream, unsupported UI API explicit errors.
- **Safety rendering**: sanitization/truncation/tab/width policy parity.

### Validation strategy
- Interaction matrix tests for keybindings and submit paths.
- Render snapshot tests for streaming and tool lifecycle states.
- Non-interactive contract tests for print/RPC outputs and exit codes.

### Risk controls and rollback
- **Risk**: frontend regressions obscure runtime correctness.
- **Control**: reducer snapshot tests + failure-injection for renderer exceptions.
- **Rollback**: runtime mode-level fallback (interactive only, print only, or rpc only) to TS frontend adapters.

### Artifacts produced
- UI parity snapshot suite.
- Print/RPC contract compatibility report.
- Sanitization and truncation safety report.

### Exit gate
- Frontend parity gates pass for interactive, print, and RPC paths.

---

## Phase 8 — Integration, hardening, and parity closure

### Objective
Integrate all subsystems into `lorum-runtime`, run full parity campaign, and close blockers.

### Scope
- Cross-subsystem integration, stress/fault testing, migration hardening.

### Dependencies
- Phases 2, 2A, 2B, and 3–7 complete.

### Implementation tasks by subsystem
- **End-to-end integration**: wire all crates through runtime composition root.
- **Golden replay**: run full TS-vs-Rust corpus replay and diff outputs/events.
- **Fault injection**: transport timeouts, partial failures, abort storms, cache corruption, renderer panics.
- **Performance baselining**: startup latency, tool throughput, memory footprint vs TS baseline.
- **Migration tooling**: compatibility check command for pre-cutover validation.

### Validation strategy
- Full regression suite: contract + golden + property/fuzz tests for parsers and mutation engines.
- Soak tests on long sessions with MCP/task/async activity.
- Mixed-runtime conformance checks (see policy section below).

### Risk controls and rollback
- **Risk**: hidden parity drift appears only in integrated flows.
- **Control**: mandatory full-corpus replay and must-fix blocker triage before launch.
- **Rollback**: freeze release candidate and revert runtime selector default to TS until blocker closure.

### Artifacts produced
- Integrated parity closure report.
- Performance and stability benchmark report.
- Cutover readiness checklist signed by subsystem owners.

### Exit gate
- Zero open P0/P1 parity blockers.
- Cutover readiness checklist fully signed.

---

## Phase 9 — Staged cutover and rollbackable launch

### Objective
Move production runtime from TS default to Rust default through canary and staged rollout while preserving rollback safety.

### Scope
- Runtime selector rollout, production telemetry, rollback automation.

### Dependencies
- Phase 8.

### Implementation tasks by subsystem
- **Runtime selection**: process-entry runtime flag with explicit TS/Rust selector.
- **Canary cohorts**: internal agents → low-risk users → broad rollout.
- **Observability**: parity drift detectors, error literal drift detector, protocol mismatch alarms, tool failure rate alarms.
- **Operational playbooks**: rollback commands, incident routing, data capture templates.

### Validation strategy
- Canary-specific golden replay on live traces (sampled).
- SLO tracking:
  - task completion success rate
  - tool failure ratio by tool name
  - MCP connect latency and timeout rate
  - edit safety incident count

### Risk controls and rollback
- **Risk**: production-only divergence under real workloads.
- **Control**: staged exposure gates with automatic rollback triggers.
- **Rollback**: selector flip to TS default, preserve Rust telemetry for postmortem, reopen blocked phase gate.

### Artifacts produced
- Canary reports per stage.
- Cutover decision logs.
- Rollback postmortem templates and executed incident records (if triggered).

### Exit gate
- Broad rollout SLOs stable for agreed observation window.
- TS runtime no longer default; Rust is primary.

---

## 4) Parallelization matrix (teams, concurrency, mandatory sequencing)

| Team | Primary scope | Can start at | Runs in parallel with | Must wait for | Consumes contracts from |
|---|---|---:|---|---|---|
| Team A | AI/auth/models (`lorum-ai-*`) | Phase 2 | Teams H/B/C/E/F | Phase 1 | `lorum-domain`, runtime trait seams |
| Team H | Core/UI hardening (`lorum-agent-core`, `lorum-ui-core`, runtime↔ui boundary) | Phase 2B | Teams A/C/D | Phases 1, 2, and 2A | runtime + ui contract seams |
| Team B | Tool runtime/render + native bridge + edit (`lorum-tool-*`, `lorum-edit-engine`) | Phase 3 | Teams A/C/E/F | Phases 1, 2A, and 2B | tool/domain contracts |
| Team C | MCP/capability/internal URLs (`lorum-mcp-*`, `lorum-capability`, `lorum-internal-urls`) | Phase 5 | Teams A/B/E/F | Phase 1 | domain + transport traits |
| Team D | Extensibility (`lorum-extensibility`) | Phase 5 (with Team C) | Teams A/B/E/F | Phase 1 | capability contracts |
| Team E | Task/subagent/isolation (`lorum-task-*`, `lorum-subagent-*`, `lorum-async-jobs`) | Phase 6 | Teams A/B/C/F | Phases 1 and 3 | tool runtime + domain contracts |
| Team F | Frontends (`lorum-ui-*`) | Phase 7 | Teams A/B/C/E | Phases 1 and 2B | runtime event contracts |
| Team G | Integration/runtime/session/core (`lorum-runtime`, `lorum-session`, `lorum-agent-core`) | Phase 1 and Phase 8 | Coordinates all | Subsystem phase exits | all subsystem contracts |

### Mandatory sequencing edges

1. Phase 0 → all development.
2. Phase 1 → all subsystem migrations.
3. Phase 2 → Phase 2A (agentic loop foundation requires AI/auth/models).
4. Phase 2A → Phase 2B (core/ui hardening must freeze base contracts before tools).
5. Phase 2B → Phase 3 (tool runtime builds on frozen loop/reducer/runtime-ui contracts).
6. Phase 3 → Phase 4 and Phase 6 (tool integration dependency).
7. Phases 2/2A/2B/4/5/6/7 → Phase 8.
8. Phase 8 → Phase 9.

---

## 5) Parity blocker policy and edge-case controls

## 5.1 Mid-phase parity blocker handling

- Classify blocker severity immediately:
  - **P0**: safety/data-corruption/protocol break/parser break.
  - **P1**: behavior drift breaking golden contracts.
  - **P2**: non-critical UX/perf drift.
- **P0/P1 rule**: freeze downstream dependent phase work; create hotfix branch; require red→green proof against failing contract/golden case.
- No phase exit permitted with open P0/P1 blockers.

## 5.2 Parser-dependent literal regression policy

For parser-dependent literals (edit errors, submit warnings, MCP method names/errors, URL handler errors):

1. Literal changes are prohibited during parity phases.
2. Literal contract tests run in pre-merge CI and stage gates.
3. If a regression is detected in canary, automatic rollback trigger fires.

## 5.3 Mixed-runtime transition without semantic drift

- Runtime selector is session-scoped and immutable for session lifetime.
- Shared persistence formats must be backward-readable/writable with versioned compatibility checks.
- Golden replay compares TS and Rust outputs for sampled production traces.
- Any drift in parity-critical outputs escalates as P1 minimum.

---

## 6) Cutover strategy (canary, staged rollout, rollback triggers)

## 6.1 Rollout stages

1. **Stage A — Internal canary**
   - Scope: internal engineering sessions only.
   - Goal: validate telemetry, failure classification, rollback automation.
2. **Stage B — Low-risk external cohort**
   - Scope: non-critical workflows and opt-in users.
   - Goal: prove stable behavior under moderate load.
3. **Stage C — Progressive broad rollout**
   - Scope: incremental percentage ramps to default Rust runtime.
   - Goal: reach stable default with no parity-critical incidents.
4. **Stage D — Full cutover**
   - Scope: Rust default for all supported modes, TS behind emergency flag only.

## 6.2 Rollback triggers

Immediate rollback to TS default if any trigger breaches threshold:

- Edit safety failure (ambiguous/stale application accepted incorrectly) > 0.
- Parser-dependent literal mismatch in production path > 0.
- MCP handshake/protocol mismatch rate above baseline tolerance.
- Task submit enforcement failures (missing warning contract or malformed acceptance drift) above tolerance.
- Critical mode contract failures:
  - print exit-code contract break
  - RPC ready/event envelope break

## 6.3 Rollback procedure

1. Flip runtime selector default to TS at process entry.
2. Preserve Rust trace artifacts and telemetry for incident triage.
3. Open parity incident with failing golden/contract evidence.
4. Patch Rust path, re-run Phase 8 gates, then re-enter staged rollout from Stage A.

---

## 7) Verification strategy by phase and release gate

| Gate | Required evidence | Blocking failures |
|---|---|---|
| Contract Gate | Literal and envelope tests from Phase 0 | Any parser/protocol/literal mismatch |
| Subsystem Gate | Phase-specific parity suites and reports | Any open P0/P1 parity blocker |
| Integration Gate | Full golden replay + fault injection + soak | Unexplained event/output drift |
| Cutover Gate | Canary SLO stability + no rollback triggers | Any trigger breach |

Minimum verification categories (commands may vary by repository layout):

- Rust unit/integration tests per crate (`cargo test -p <crate>`).
- Cross-runtime golden replay harness (`ts_baseline` vs `rust_candidate`).
- Contract literal/envelope checks.
- Mutation safety suites (replace/patch/hashline).
- Protocol and transport suites (MCP HTTP/stdio/OAuth discovery).
- End-to-end mode suites (interactive reducer snapshots, print, RPC).

---

## 8) Milestone tracker template (execution use)

Use this template during implementation; one row per milestone (phase or sub-phase).

| Milestone ID | Phase | Owner Team | Scope | Planned Start | Planned End | Actual End | Dependencies Cleared | Deliverables Completed | Verification Evidence Link | Open Blockers (P0/P1/P2) | Go/No-Go |
|---|---|---|---|---|---|---|---|---|---|---|---|
| M0.1 | 0 | Team G | Contract freeze + golden corpus |  |  |  |  |  |  |  |  |
| M1.1 | 1 | Team G | Core scaffolding + trait seams |  |  |  |  |  |  |  |  |
| M2.1 | 2 | Team A | AI/auth/model parity |  |  |  |  |  |  |  |  |
| M2A.1 | 2A | Team H | Agentic loop (chat-only) parity across modes |  |  |  |  |  |  |  |  |
| M2B.1 | 2B | Team H | Agent-core + UI-core hardening and contract freeze |  |  |  |  |  |  |  |  |
| M3.1 | 3 | Team B | Tool runtime/render/native bridge |  |  |  |  |  |  |  |  |
| M4.1 | 4 | Team B | Edit/patch/hashline parity |  |  |  |  |  |  |  |  |
| M5.1 | 5 | Team C/D | MCP/capability/extensibility parity |  |  |  |  |  |  |  |  |
| M6.1 | 6 | Team E | Task/subagent/isolation parity |  |  |  |  |  |  |  |  |
| M7.1 | 7 | Team F | TUI/print/RPC parity |  |  |  |  |  |  |  |  |
| M8.1 | 8 | Team G | Integration + hardening closure |  |  |  |  |  |  |  |  |
| M9.1 | 9 | Team G + Ops | Staged cutover + stable default |  |  |  |  |  |  |  |  |

Completion definition: all rows Go with linked evidence, and no open P0/P1 blockers.