# 10 — Migration Risk Register and Parity Checklist (TS → Rust Cutover)

This document is the operational release gate for parity and migration safety. It translates obligations from docs 03–09 into: (1) a risk register with detection and rollback controls, (2) a subsystem parity checklist with evidence capture fields, and (3) release sign-off and emergency response procedures.

## Scope and operating policy

- Parity-first enforcement: no intentional semantic drift during migration phases.
- Session-scoped runtime semantics: TS and Rust behavior MUST NOT mix within a session.
- Parser-dependent strings/literals/protocol methods are immutable during parity phase.
- Any open P0/P1 parity blocker is a hard no-go for cutover.
- Rollback authority is pre-approved for on-call release/security leads when trigger thresholds are breached.

---

## A) Risk Register

Severity scale: **Critical / High / Medium / Low**  
Likelihood scale: **High / Medium / Low**

| Risk ID | Subsystem | Description / Root Cause | Severity | Likelihood | Leading indicators / Detection methods | Preventive controls | Contingency / rollback action | Owner role |
|---|---|---|---|---|---|---|---|---|
| R-001 | Protocol + literals (MCP/tooling/task) | **Parser-dependent literal regression** (e.g., MCP method strings, protocol version `2025-03-26`, edit error literals, submit warning prefix) breaks parsers/tests/automation expecting exact byte sequences. Root cause: refactor or localization of constants in Rust path. | Critical | Medium | Contract literal tests fail; canary detects string mismatch; parser error spike in logs; missing warning-prefix extraction in task render output. | Central immutable constants module; pre-merge literal snapshot diff check; canary literal drift detector; block any string normalization on parity literals. | Immediate selector flip to TS default; invalidate Rust canary; open P0 incident with failing literal evidence; hotfix constants before re-entry at Stage A canary. | Runtime platform lead + Security reviewer |
| R-002 | Tool scheduler/event model | Ordering nondeterminism across shared/exclusive tool scheduling, lifecycle event sequencing, task result order, and branch merge order. Root cause: async race and non-deterministic collection iteration in Rust runtime. | Critical | Medium | Golden event sequence diffs; flaky ordering tests; mismatch in progress snapshot index order; merge/cherry-pick order divergence under load. | Deterministic sequence numbering; stable sort by input index; explicit scheduler barriers for exclusive tools; property tests for event ordering. | Disable Rust task/tool runtime via feature selector; route affected tools/tasks to TS path; rerun golden replay before re-enable. | Agent-core lead |
| R-003 | Edit engine (replace/patch/hashline) | Edit safety failure: ambiguous hunk/match accepted, stale hashline anchors applied, overlap not rejected, or no-partial-write guarantee violated. Root cause: incomplete parity in ambiguity detection / pre-validation. | Critical | Low | Mutation safety suite failure; any accepted stale anchor; production incident count for unsafe edit > 0; mismatch in hashline mismatch diagnostics. | Enforce full pre-validation before mutation; keep fail-fast ambiguity behavior; atomic write path with rollback; negative tests for stale/overlap/ambiguous cases. | Immediate rollback trigger (threshold > 0); force edit tool to TS engine; quarantine affected release and run incident triage on captured artifacts. | Edit engine lead |
| R-004 | Subagent orchestration | `submit_result` enforcement drift: missing forced tool injection layer, malformed success accepted, warning-prefix changed, or retry loop semantics changed. Root cause: simplification of redundant enforcement layers. | High | Medium | Missing `submit_result` in child toolset; increase in runs without valid submit payload; warning-prefix detector misses malformed exit output. | Preserve all enforcement layers (discovery + tool creation + subprocess setup + reminder loop); lock warning literals and prefixes in tests. | Route `task` tool to TS runtime; block rollout stage progression; patch enforcement and rerun task parity matrix. | Task/subagent lead |
| R-005 | MCP manager + transport | MCP degraded startup/cache/auth races: 250ms startup grace parity broken, deferred cached tools not available, auth discovery race, stale notification side-effects not epoch-gated. Root cause: race conditions in async connection/subscription flows. | High | Medium | MCP connect timeout/latency SLO regression; tool unavailability after startup; notification state drift; auth re-prompt loops. | Preserve `STARTUP_TIMEOUT_MS=250`, deferred tool bridge behavior, cache hash/TTL semantics, and `rollback|ignore|apply` subscription gate. | Flip MCP stack to TS bridge while Rust core remains; clear invalid cache entries; replay MCP race test suite before re-enable. | MCP lead |
| R-006 | Capability/discovery + tool assembly | Capability/discovery precedence drift: first-win dedupe order changes, tool auto-enrichment rules drift (`ast_*`, `resolve`, `submit_result`, `exit_plan_mode`), skill ordering changes. Root cause: different map/iteration semantics or policy refactor. | High | Medium | Capability snapshot diffs; missing auto-injected tools; shadow diagnostics mismatch; skill ordering diffs in discovery output. | Explicit priority vectors and stable ordering contracts; parity snapshots in CI; deny unordered map iteration at decision boundaries. | Revert capability/discovery module to TS path; freeze rollout and regenerate precedence snapshots post-fix. | Extensibility lead |
| R-007 | Extensions/plugins/internal URLs | Extension/plugin loading conflicts and boundary bypass (source precedence/collision drift), plus internal URL traversal (`skill://`, `memory://`, `local://`) or unauthorized extension execution context leakage. Root cause: insufficient path normalization and policy enforcement in Rust loaders. | Critical | Medium | Security tests detect traversal acceptance; plugin conflict diagnostics missing; extension executes in wrong UI/security context; unexplained file access in audit logs. | Canonical path normalization and traversal rejection; strict source precedence and conflict policy parity; execution sandbox/permission boundary checks; security test suite for URL protocols. | Trigger security rollback to TS extension + internal URL handlers; invalidate suspicious plugin bundles; incident response with forensic capture and patch before relaunch. | Security lead + Extensibility lead |
| R-008 | TUI/print/RPC | Sanitization/render failure regression: renderer panic escapes fallback path, unsanitized terminal control output rendered, print/RPC contract drift (exit codes, ready sentinel, unsupported API errors). Root cause: render exception isolation or sanitization pipeline gaps. | High | Medium | Renderer panic in logs with session abort; sanitization snapshot diffs; print/RPC compatibility test failures; increase in malformed JSONL outputs. | Keep renderer try/catch fallback semantics; sanitize all display/error paths; strict print/RPC contract tests and snapshot baselines. | Mode-level rollback (interactive/print/RPC adapter to TS); block release stage until sanitization and contract gates pass. | UI/runtime lead |
| R-009 | Mixed runtime persistence | Persistence/schema compatibility issues across TS/Rust (session logs, model cache, MCP cache, artifacts) causing unreadable state or semantic drift. Root cause: versioning mismatch or non-compatible serializers during mixed-runtime period. | Critical | Medium | Backward-read failure rates; cache parse errors; schema diff alerts; mixed-runtime replay mismatches on sampled traces. | Versioned schema compatibility checks; backward+forward read tests; migration guards; immutable session runtime selector. | Roll back default to TS; disable Rust writers for affected store; run repair/migration tool and compatibility regression suite. | Session/persistence lead |
| R-010 | Rollout operations | Canary blind spots: insufficient scenario coverage, telemetry gaps, or delayed trigger detection allow broad rollout despite parity drift. Root cause: incomplete stage gates and weak observability wiring. | High | Medium | Missing evidence for required parity categories; trigger detectors silent despite known injected faults; low confidence/noisy SLO dashboards. | Mandatory evidence checklist per stage; synthetic drift injection in canary; SLO alarm validation drills before Stage B/C. | Freeze rollout at current stage; revert selector to TS if trigger confidence is compromised; complete observability fixes then restart at Stage A. | Release manager + SRE |
| R-011 | Isolation and merge backend | Partially successful subagent isolation merges produce inconsistent repository state (subset cherry-picks applied, hidden conflicts, or cleanup failure). Root cause: non-atomic merge orchestration and weak conflict accounting in isolation backend. | Critical | Low | Merge reliability report failures; branch divergence after merge; unexpected working tree dirty state; missing conflict artifact logs. | Sequential merge by input order with explicit per-task commit accounting; transactional merge steps with rollback on first failure; cleanup assertions. | Disable isolated mode in Rust task runtime; route isolation backend to TS/worktree path; restore from pre-merge checkpoint and re-run with patched merge logic. | Task isolation lead |
| R-012 | Auth + credentials | Auth precedence/refresh drift across API key/OAuth/env/custom fallback causes auth loops or wrong credential selection under quota/expiry pressure. Root cause: ranking/backoff differences from TS behavior. | Medium | Medium | Auth refresh error spikes; higher 401/403 rates; credential blocklist churn anomalies; provider-specific failure regressions. | Contract tests for precedence chain and refresh backoff; deterministic fixture tests for least-exhausted credential selection. | Provider-level fallback to TS connector/auth path; keep Rust telemetry for incident analysis; hotfix precedence resolver. | AI/auth lead |

### P0/P1 blocker mapping

- **P0**: R-001, R-002, R-003, R-007, R-009, R-011 when triggered.
- **P1**: R-004, R-005, R-006, R-008, R-010, R-012 unless safety/protocol break elevates to P0.

---

## B) Parity Checklist (Release Gate Evidence Matrix)

Instruction: every checkbox must be marked with evidence before stage promotion. “Evidence” must reference a concrete artifact path, test run ID, dashboard panel URL, or incident ticket.

### B1. Tool runtime, scheduler, and rendering (Doc 03)

- [ ] Shared/exclusive tool scheduling parity confirmed under mixed call batches.  
  Evidence: ____________________
- [ ] Tool lifecycle event sequence (`start/update/end`) matches golden transcripts.  
  Evidence: ____________________
- [ ] Auto-enrichment parity verified (`ast_*`, `resolve`, `submit_result`, `exit_plan_mode`).  
  Evidence: ____________________
- [ ] Deferred action (`deferrable` + `resolve`) proposal/apply/discard semantics match TS behavior.  
  Evidence: ____________________
- [ ] Renderer precedence and fallback isolation validated (tool custom → registry → generic fallback).  
  Evidence: ____________________
- [ ] Sanitized error rendering preserved through meta wrapper paths.  
  Evidence: ____________________

### B2. Edit/patch/hashline safety (Doc 04)

- [ ] Replace mode exact/fuzzy/ambiguity behavior parity confirmed, including disambiguation errors.  
  Evidence: ____________________
- [ ] Patch mode single-file guard, hunk parsing, overlap rejection, and ambiguity fail-fast parity confirmed.  
  Evidence: ____________________
- [ ] Hashline anchor validation rejects stale/mismatched tags with remap diagnostics.  
  Evidence: ____________________
- [ ] No-partial-write guarantee validated for all safety-failure cases.  
  Evidence: ____________________
- [ ] BOM/newline/indentation restoration and FS invalidation hooks verified.  
  Evidence: ____________________
- [ ] **Parser-dependent edit literals unchanged** (including `old_text must not be empty.`, `Diff contains no hunks`, `File not found: ${path}`).  
  Evidence: ____________________

### B3. MCP, capability discovery, and extensibility (Doc 05)

- [ ] MCP initialize handshake uses exact protocol version `2025-03-26` and `notifications/initialized`.  
  Evidence: ____________________
- [ ] Transport timeout and HTTP 202 notification semantics unchanged.  
  Evidence: ____________________
- [ ] Manager startup grace/deferred cache behavior matches 250ms contract and deferred replacement path.  
  Evidence: ____________________
- [ ] Notification epoch post-action gate (`rollback|ignore|apply`) race tests pass.  
  Evidence: ____________________
- [ ] Cache version/hash/TTL compatibility validated against existing persisted entries.  
  Evidence: ____________________
- [ ] Capability precedence first-win ordering and shadow diagnostics match snapshots.  
  Evidence: ____________________
- [ ] Plugin/extension source precedence and conflict handling parity validated.  
  Evidence: ____________________
- [ ] **Internal URL security boundaries validated** (absolute path rejection, traversal rejection, unknown-scope errors) for `skill://`, `memory://`, `local://`, `mcp://`.  
  Evidence: ____________________

### B4. Task/subagent/orchestration/isolation (Doc 06)

- [ ] Schema selection parity (`taskSchema` vs `taskSchemaNoIsolation`) by isolation mode confirmed.  
  Evidence: ____________________
- [ ] Output truncation env parsing/fallback behavior parity confirmed (`PI_TASK_MAX_OUTPUT_*`).  
  Evidence: ____________________
- [ ] Deterministic ordering validated (result order, progress sort, merge/cherry-pick order).  
  Evidence: ____________________
- [ ] Multi-layer `submit_result` injection preserved (discovery, tool creation, subprocess).  
  Evidence: ____________________
- [ ] Reminder loop (`MAX_SUBMIT_RESULT_RETRIES=3`) and termination behavior parity confirmed.  
  Evidence: ____________________
- [ ] Warning literal + prefix contracts unchanged for missing/null submit paths.  
  Evidence: ____________________
- [ ] Fallback completion rules (schema-compatible JSON/raw output cases) match TS decision table.  
  Evidence: ____________________
- [ ] **Isolation merge reliability validated including partial-success failure handling and cleanup**.  
  Evidence: ____________________

### B5. TUI, print, RPC interaction layer (Doc 07)

- [ ] Interactive init ordering parity validated (UI ready before event subscription).  
  Evidence: ____________________
- [ ] Tool rendering grouping/inlining/image fallback semantics match TS snapshots.  
  Evidence: ____________________
- [ ] Terminal sanitization and truncation coverage verified for normal + error paths.  
  Evidence: ____________________
- [ ] Renderer exception isolation verified (no session crash, safe fallback text shown).  
  Evidence: ____________________
- [ ] Print mode contracts validated (text/json outputs and non-zero exit semantics on error/abort).  
  Evidence: ____________________
- [ ] RPC mode contracts validated (`ready` sentinel, event envelopes, unsupported UI API explicit errors).  
  Evidence: ____________________

### B6. Architecture, persistence, and integration hardening (Docs 08–09)

- [ ] Parity-critical invariant suite passes (literals, ordering, protocol methods).  
  Evidence: ____________________
- [ ] Mixed-runtime persistence compatibility validated (backward/forward read-write).  
  Evidence: ____________________
- [ ] Full golden replay TS vs Rust passes for required scenario corpus.  
  Evidence: ____________________
- [ ] Fault injection suite passes (timeouts, cache corruption, renderer panic, abort storms).  
  Evidence: ____________________
- [ ] Canary SLOs stable per stage (task success, tool failure ratio, MCP latency/timeouts, edit safety incidents).  
  Evidence: ____________________
- [ ] Rollback automation validated in drill (selector flip + artifact preservation + incident template).  
  Evidence: ____________________

### B7. Stage-gate decision checklist

- [ ] Stage A (internal canary) complete with no open P0/P1 blockers.  
  Evidence: ____________________
- [ ] Stage B (low-risk cohort) complete and trigger alarms validated.  
  Evidence: ____________________
- [ ] Stage C (progressive broad rollout) complete with stable SLO window.  
  Evidence: ____________________
- [ ] Stage D (full cutover) approved; TS remains emergency-only path.  
  Evidence: ____________________

---

## C) Release Sign-Off and Blockers

### Required approvals (all mandatory)

1. Runtime Platform Lead (tool/runtime/event parity)
2. Edit Engine Lead (mutation safety)
3. MCP/Extensibility Lead (protocol, capability, plugin/internal URL parity)
4. Task/Subagent Lead (submit enforcement + isolation merge correctness)
5. UI/Mode Lead (interactive/print/RPC parity + sanitization)
6. Security Lead (boundary and traversal protections)
7. SRE/Release Manager (canary health, rollback readiness, operational controls)

### Hard blockers (automatic no-go)

- Any open P0/P1 parity blocker.
- Any unchecked item in sections B1–B7.
- Any rollback trigger breach in canary/staged rollout.
- Any unresolved security finding on internal URL traversal, extension execution boundaries, or plugin conflict policy.
- Any missing evidence artifact for a required check.

### Sign-off record

| Role | Name | Decision (Go/No-Go) | Timestamp (UTC) | Evidence bundle link |
|---|---|---|---|---|
| Runtime Platform Lead |  |  |  |  |
| Edit Engine Lead |  |  |  |  |
| MCP/Extensibility Lead |  |  |  |  |
| Task/Subagent Lead |  |  |  |  |
| UI/Mode Lead |  |  |  |  |
| Security Lead |  |  |  |  |
| SRE/Release Manager |  |  |  |  |

---

## D) Emergency Response Playbook Snippets (Top Critical Risks)

### D1. Parser-dependent literal/protocol regression (R-001)

**Trigger:** any literal mismatch in production path, parser break, or method-name/protocol drift alert.  
**Immediate actions (0–15 min):**
1. Freeze rollout progression and declare parity incident (P0).
2. Flip runtime selector default to TS.
3. Preserve Rust traces, request/response payloads, and failing artifact bundle.
4. Run literal contract suite against current candidate and identify first bad commit.

**Recovery criteria:**
- Literal contract tests green.
- Canary replay shows zero literal drift.
- Security/runtime leads approve re-entry at Stage A.

### D2. Unsafe edit acceptance (R-003)

**Trigger:** accepted stale hashline anchor, accepted ambiguous patch/replace, or any edit safety incident count > 0.  
**Immediate actions (0–15 min):**
1. Disable Rust edit engine path and route edit tool to TS engine.
2. Halt all stage promotion.
3. Capture offending request, file pre/post state, and decision logs.
4. Run focused mutation safety regression corpus with new counterexample added.

**Recovery criteria:**
- Counterexample reproduced and fixed in Rust.
- No-partial-write and ambiguity/staleness suites fully green.
- Edit engine lead signs off with evidence.

### D3. Internal URL traversal / extension boundary breach (R-007)

**Trigger:** traversal accepted, unauthorized file scope access, or extension executed outside allowed boundary.  
**Immediate actions (0–30 min):**
1. Activate security incident channel and classify severity.
2. Roll back Rust internal URL + extension loader handlers to TS path.
3. Disable newly loaded suspect plugins/extensions and rotate affected credentials if exposed.
4. Preserve forensic logs (resolved path inputs, normalized outputs, execution context, actor/session IDs).

**Recovery criteria:**
- Security regression tests for traversal/boundary controls green.
- Independent security review confirms patch efficacy.
- Controlled canary with enhanced audit logging passes for observation window.

### D4. Mixed-runtime persistence/schema incompatibility (R-009)

**Trigger:** backward/forward read failures, cache parse errors, or schema drift affecting active sessions.  
**Immediate actions (0–30 min):**
1. Flip default to TS and disable Rust writes to impacted store(s).
2. Snapshot corrupted/incompatible records and preserve migration metadata.
3. Run compatibility verifier and identify serializer/version gate mismatch.
4. Execute repair/migration procedure on staged copy before production application.

**Recovery criteria:**
- Compatibility suite passes for TS↔Rust read/write matrix.
- Restored records validated against golden replay sample.
- Session/persistence lead + SRE approve phased re-entry.
