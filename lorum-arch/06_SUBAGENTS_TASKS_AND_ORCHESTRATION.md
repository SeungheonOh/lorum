# 06 — Subagents, Task Tool, and Orchestration Runtime

This document specifies parity-critical behavior for Rust migration of subagent orchestration: task schema, execution runtime, submit enforcement, subprocess event extraction/rendering, async background jobs, isolation/merge flows, nested tasks, and failure/cancellation edges.

Primary source surface:

- Task tool + task domain:
  - `packages/coding-agent/src/task/{index.ts,types.ts,template.ts,executor.ts,render.ts,subprocess-tool-registry.ts,parallel.ts,output-manager.ts}`
  - `packages/coding-agent/src/task/{isolation-backend.ts,worktree.ts}`
  - `packages/coding-agent/src/task/{agents.ts,discovery.ts}`
- Hidden completion/review tools:
  - `packages/coding-agent/src/tools/{submit-result.ts,review.ts}`
- Session + SDK integration:
  - `packages/coding-agent/src/sdk.ts`
  - `packages/coding-agent/src/session/agent-session.ts`
  - `packages/coding-agent/src/async/job-manager.ts`
  - `packages/coding-agent/src/discovery/helpers.ts`
- Prompt contracts used in enforcement:
  - `packages/coding-agent/src/prompts/system/{subagent-system-prompt.md,subagent-submit-reminder.md,subagent-user-prompt.md}`

Notes on file drift vs older architecture references:

- No standalone `schema.ts`, `compat.ts`, `run-subprocess.ts`, `async-job-manager.ts`, `subprocess-state.ts`, `isolated.ts`, `branch.ts`, or `merge.ts` exist in current tree.
- Their responsibilities are consolidated into `types.ts`, `index.ts`, `executor.ts`, `worktree.ts`, and `async/job-manager.ts`.
- Rust rewrite should follow current runtime behavior, not historical file names.

---

## 1) Task tool contract and schema model

## 1.1 TypeBox schema and parameter variants

`task/types.ts` defines two parameter schemas:

- `taskSchema` (with `isolated?: boolean`)
- `taskSchemaNoIsolation` (without `isolated`)

At runtime, `TaskTool` chooses schema based on setting `task.isolation.mode`:

- mode `none` => `taskSchemaNoIsolation`
- otherwise => `taskSchema`

Core payload:

- `agent: string`
- `context?: string`
- `schema?: Record<string, unknown>` (JTD schema for expected output)
- `tasks: TaskItem[]`
- `TaskItem = { id, description, assignment }`

`TaskItem.id` has hard max length `48` and is used in artifact IDs, branch names, and UI ordering.

## 1.2 Output truncation env overrides (parity-critical)

From `task/types.ts`:

- `MAX_OUTPUT_BYTES = parseNumber(PI_TASK_MAX_OUTPUT_BYTES, 500_000)`
- `MAX_OUTPUT_LINES = parseNumber(PI_TASK_MAX_OUTPUT_LINES, 5000)`

Behavior:

- only positive integer env values are accepted
- invalid/empty/non-positive values fall back to defaults

Rust must preserve this permissive parse+fallback behavior exactly.

## 1.3 Runtime details shape and sync/async return contracts

`TaskToolDetails` shape:

- `projectAgentsDir: string | null`
- `results: SingleResult[]`
- `totalDurationMs: number`
- optional `usage`
- optional `outputPaths`
- optional `progress: AgentProgress[]`
- optional `async: { state: "running"|"completed"|"failed"; jobId: string; type: "task" }`

Return modes:

- Sync path returns final `results[]` (possibly empty on top-level failure), no async object.
- Async path returns immediate acknowledgement text + `details.async.state = "running"`, and later emits updates through `onUpdate` and session follow-up messages.

---

## 2) TaskTool lifecycle: dispatch, validation, and deterministic ordering

## 2.1 Entry dispatch

`TaskTool.execute(...)` chooses path:

- Sync when:
  - `async.enabled` is false, or
  - selected agent has `blocking: true`
- Async when:
  - `async.enabled` true and agent not blocking and async manager exists

If async manager missing while async enabled, tool returns explicit error text with empty results.

## 2.2 Pre-flight validation in sync path

`#executeSync` enforces:

- Agent exists (`Unknown agent ...` otherwise)
- Agent not disabled via `task.disabledAgents`
- Task list non-empty
- Every task has non-empty `id`
- IDs are unique case-insensitively
- Self-recursion protection via `PI_BLOCKED_AGENT`
- Parent spawn allowlist (`session.getSessionSpawns()`):
  - `""` deny all
  - `"*"` allow all
  - CSV allowlist otherwise

Task IDs drive ordering and summaries; duplicate detection prevents nondeterministic overwrite/collision.

## 2.3 Deterministic ordering constraints

- Progress snapshots are sorted by `index`.
- Final result order preserves original task order (`mapWithConcurrencyLimit` stores by input index).
- Branch merges are sequential in input order (`mergeTaskBranches` cherry-picks one-by-one).
- Nested rendering also follows result order.

Rust parity must preserve these order guarantees.

---

## 3) Subagent session construction and executor lifecycle

## 3.1 Prompt composition and assignment shaping

Per task:

1. `renderTemplate(context, task)` builds subagent user payload:
   - wraps shared context in `<context>...</context>` via `subagent-user-prompt.md`
   - wraps assignment in `<goal>...</goal>`
2. `runSubprocess` creates child `AgentSession` with:
   - base system prompt + subagent wrapper prompt (`subagent-system-prompt.md`)
   - agent-specific system prompt inserted into wrapper
   - output schema inserted for typed completion guidance

`subagent-system-prompt.md` includes hard requirement to call `submit_result` exactly once.

## 3.2 Tool set resolution and recursion cutoff

In `runSubprocess`:

- start with agent tool allowlist (`agent.tools`) when present
- auto-add `task` if `spawns` defined and not at max depth
- remove `task` when recursion limit hit
- expand legacy alias `exec` -> `python`/`bash` based on `python.toolMode`

Recursion rules:

- `maxDepth = settings.get("task.maxRecursionDepth") ?? 2`
- child depth = `parentDepth + 1`
- `atMaxDepth` disables nested task tool injection

Also enforced globally in `createTools` (`tools/index.ts`) for direct task tool availability.

## 3.3 Required submit_result injection on multiple layers

Submit enforcement is intentionally redundant:

1. Agent parsing (`discovery/helpers.ts`): explicit agent tool lists auto-append `submit_result`.
2. Tool creation (`tools/index.ts`): `requireSubmitResultTool` forces hidden tool inclusion.
3. Subagent session creation (`runSubprocess`): always passes `requireSubmitResultTool: true`.
4. Runtime reminders (see section 5) if subagent still fails to call it.

Rust rewrite must preserve all layers; removing one weakens safety.

## 3.4 Executor event loop and lifecycle phases

`runSubprocess` phases:

1. Initialize progress state (`status=running`, counters zeroed).
2. Create child session + subscribe to AgentEvents.
3. Prompt once with full task.
4. Wait idle.
5. If no submit_result yet, run reminder loop up to `MAX_SUBMIT_RESULT_RETRIES=3`.
6. Finalize output (submit_result-aware rewrite/fallback/warnings).
7. Truncate output by byte/line limits.
8. Persist artifact markdown output.
9. Emit final progress and return `SingleResult`.

Abort paths can short-circuit at multiple checkpoints; abort reason propagation is explicit.

---

## 4) Subprocess tool extraction/termination/render split

## 4.1 Registry contract

`subprocess-tool-registry.ts` defines per-tool hooks:

- `extractData(event)`
- `shouldTerminate(event)`
- `renderInline(data, theme)`
- `renderFinal(allData, theme, expanded)`

Executor only knows registry; rendering/termination policy is tool-defined.

## 4.2 submit_result handler semantics

`tools/submit-result.ts` registers:

- `extractData` => `{ data, status, error }` when details shape valid
- `shouldTerminate` => `!event.isError`

Effect:

- valid non-error `submit_result` ends subagent run (`requestAbort("terminate")`)
- malformed/error submit_result does not trigger termination

## 4.3 report_finding handler semantics

`tools/review.ts` registers `report_finding`:

- extraction only when event non-error and payload parses
- inline/final rendering grouped by priority
- no termination behavior

Executor also deduplicates findings by key `(file_path,line_start,line_end,priority,title)`.

## 4.4 nested task handler semantics

`task/render.ts` registers handler for tool name `task`:

- `extractData` accepts only `TaskToolDetails`-shaped details
- `renderFinal` recursively renders nested `results`

Parent render defers nested task lines (`deferredToolLines`) so they appear after primary output block.

---

## 5) submit_result contract enforcement and fallback behavior

## 5.1 SubmitResultTool payload contract

`submit_result` parameters:

- `{ result: { data: <schema> } }` success path
- `{ result: { error: string } }` abort/failure path

Hard errors:

- `result must be an object containing either data or error`
- `result cannot contain both data and error`
- `result must contain either data or error`
- `data is required when submit_result indicates success`

Schema behavior:

- `lenientArgValidation = true`
- first schema mismatch throws
- subsequent mismatches allowed with override message

## 5.2 Reminder forcing logic

`executor.ts` reminder loop:

- max retries: `3`
- reminder prompt from `subagent-submit-reminder.md`
- tool choice targeting tries provider-specific shape via `buildSubmitResultToolChoice(...)`:
  - OpenAI-style: `{ type: "function", name: "submit_result" }`
  - Anthropic/Bedrock-style: `{ type: "tool", name: "submit_result" }`

If still missing after retries and not already aborted:

- mark run aborted+failed
- set `error` and `abortReason` to missing-submit warning string

## 5.3 Parity-critical warning literals

Must remain byte-for-byte unchanged where consumed by parser/render logic:

- `SYSTEM WARNING: Subagent called submit_result with null data.`
- `SYSTEM WARNING: Subagent exited without calling submit_result tool after 3 reminders.`
- Renderer detection prefix (`task/render.ts`):
  - `SYSTEM WARNING: Subagent exited without calling submit_result tool`

Renderer strips first line when it starts with prefix and re-renders as warning badge/line. Changing prefix breaks extraction.

## 5.4 finalizeSubprocessOutput decision table

If submit_result exists:

- `status=aborted` => force `exitCode=0`, output JSON `{aborted:true,error}`
- `status=success` and `data=null|undefined` => prepend null-warning string, keep raw output
- `status=success` with data => serialize normalized data to pretty JSON, `exitCode=0`, clear stderr

If submit_result missing:

- when `exitCode=0` and schema-compatible JSON output exists => fallback completion accepted
- when no output schema and non-empty raw output => accept raw output as success (`exitCode=0`)
- else when `exitCode=0` => prepend missing-submit warning string

Normalization step injects report findings into output object when findings exist and output object lacks `findings` key.

---

## 6) Event processing, usage accumulation, and progress cadence

## 6.1 Event channels and forwarding

Executor emits onto event bus when provided:

- raw events: `task:subagent:event`
- aggregated progress: `task:subagent:progress`

Only explicit agent event set is processed (`agent_start/end`, `turn_start/end`, message/tool start/update/end).

## 6.2 Progress model updates

During run:

- tracks `currentTool`, args preview, start timestamp
- stores `recentTools` capped at 5
- tracks `recentOutput` tail (last ~8KB, up to 8 lines shown)
- aggregates tokens from usage-like payloads with flexible field normalization

Final status resolution:

- `aborted` when submit_result-aborted OR caller abort without submit_result
- `completed` when exitCode 0 and not aborted
- otherwise `failed`

## 6.3 Coalescing cadence (parity-critical)

Progress emission throttle:

- `PROGRESS_COALESCE_MS = 150`
- tool end/agent end flushes immediately
- otherwise emits at most once per 150ms window

Rust rewrite should preserve cadence to avoid UI churn and event storms.

## 6.4 Usage accumulation semantics

Executor accumulates usage incrementally from `message_end` assistant events only:

- sums input/output/cache/token/cost fields when present
- tolerant to field naming variants
- attaches usage only when at least one usage object observed

Top-level task sync path aggregates `SingleResult.usage` across child runs.

---

## 7) Async background task flow

## 7.1 Async path in TaskTool

When async enabled and task list non-empty:

- each task item is registered as async job (`type: "task"`)
- each job internally calls sync execution for single task payload
- per-job concurrency is additionally gated by `Semaphore(task.maxConcurrency)`

Immediate tool return:

- content text: started jobs summary
- details:
  - empty `results`
  - `progress` snapshot for all tasks
  - `async: { state: "running", jobId: <first job>, type: "task" }`

## 7.2 Async state transitions and update shapes

`details.async.state` transitions:

- `running` during launch/progress
- `completed` when all finished and no failures/schedule failures
- `failed` when any failed job or scheduling failure

Batch status messages (literal patterns) include:

- `Launching N background task(s)...`
- `Background task batch progress: X/Y finished (R running).`
- `Background task batch complete: X/Y finished.`
- failure variants with `complete with failures`

## 7.3 AsyncJobManager queue/concurrency behavior

`async/job-manager.ts`:

- max running cap default 15 (overridden in sdk by `async.maxJobs`, clamped 1..100)
- register rejects when running count >= max
- job IDs deduplicated (`id`, `id-2`, ...)
- completion delivery queue retries with exponential backoff+jitter
- delivery suppression/ack APIs for job IDs
- retention eviction timer default 5 minutes

SDK wiring:

- `onJobComplete` sends follow-up custom message with formatted result
- long async result handling:
  - inline limit: 12,000 chars
  - preview: 4,000 chars
  - full result persisted to `artifact://` when available

---

## 8) Isolation modes, merge flows, and failure handling

## 8.1 Mode resolution and fallback

Settings-driven:

- `task.isolation.mode`: `none|worktree|fuse-overlay|fuse-projfs`
- `task.isolation.merge`: `patch|branch`
- `task.isolation.commits`: `generic|ai`

`resolveIsolationBackendForTaskExecution(...)` may downgrade mode with `<system-notification>` warnings:

- Windows + fuse-overlay => worktree fallback
- non-Windows + fuse-projfs => worktree fallback
- ProjFS host/repo prerequisites fail => worktree fallback with reason

Hard error remains for initialization failures not classified as prerequisite-unavailable.

## 8.2 Worktree/fuse/projfs setup semantics

- `worktree`: detached `git worktree add --detach ... HEAD`, plus baseline replay
- `fuse-overlay`: mounts `fuse-overlayfs` (`lowerdir=repoRoot, upper/work/merged`)
- `fuse-projfs`: `projfsOverlayStart/Stop` on Windows

Cleanup always attempts unmount/stop + recursive remove.

## 8.3 Patch merge mode

For successful isolated results:

- capture per-task root patch + nested repo patches
- combine root patches in task order
- `git apply --check --binary` then `git apply --binary`
- on failure: mark manual handling required and include patch artifact paths

Notification strings include:

- `<system-notification>Patches were not applied and must be handled manually.</system-notification>`
- nested patch non-fatal warning when applicable

## 8.4 Branch merge mode

Per task success:

- `commitToBranch` creates `lorum/task/<taskId>` branch
- applies root patch in temp worktree, commits (optional AI commit message)
- returns nested patches separately

Final merge:

- `mergeTaskBranches` cherry-picks branches sequentially
- on first conflict:
  - abort cherry-pick
  - returns merged list + failed list including remaining branches
  - conflict string includes branch+stderr
- summary includes `<system-notification>Branch merge failed...Unmerged branches remain for manual resolution.</system-notification>`

Cleanup:

- merged branches deleted only when overall branch merge succeeded
- failed/unmerged branches retained for manual resolution

---

## 9) Renderer behavior: warnings, nested aggregation, and review overlays

## 9.1 Missing-submit warning extraction

`task/render.ts`:

- checks first line for `MISSING_SUBMIT_RESULT_WARNING_PREFIX`
- strips warning from output body
- shows warning in dedicated warning style
- leaves remaining output for JSON/raw rendering

This parsing is prefix-based and intentionally brittle to string change.

## 9.2 Review-specific rendering precedence

If extracted `submit_result` contains review summary shape (`overall_correctness`) and findings exist:

- render combined verdict + confidence + summary + priority-sorted findings
- suppress generic output rendering path

If findings exist without valid review verdict:

- render warning:
  - `Review verdict missing expected fields` OR
  - `Review incomplete (submit_result not called)`

## 9.3 Nested task rendering aggregation

- nested `task` extracted details are rendered via registered `task` subprocess handler
- nested lines are deferred and appended after primary output/custom sections
- enables recursive task result surfacing while keeping parent status/output readable

---

## 10) Adversarial and edge-case behavior

## 10.1 Malformed submit_result payloads

Handled in tool execute with explicit errors:

- non-object `result`
- both `data` and `error`
- neither field present
- success with null/undefined data

In subprocess extraction, malformed details are ignored (`extractData` returns undefined), so enforcement falls back to reminder/missing-submit path.

## 10.2 Null data submit and parser-dependent warnings

If submit_result called with null/undefined success data:

- does not count as valid completion payload
- warning string prepended
- renderer exposes warning

Do not alter warning literal.

## 10.3 Missing submit_result with successful raw output

Two permissive escape hatches exist:

- schema-validated fallback parse from raw output JSON
- no-schema mode accepts non-empty raw output as success

Otherwise successful exit with missing submit_result is downgraded via warning prefix.

## 10.4 Cancellation and abort races

Observed race controls:

- shared abort controller + session abort on signal
- repeated abort requests coalesce, with `signal` reason taking precedence
- subscribe handler guarded by `resolved` flag
- progress timer cleared on finalize
- pending async workers may complete after global abort; skipped tasks get deterministic placeholder result (`Cancelled before start`)

## 10.5 Task recursion and spawn boundaries

Two independent boundaries:

- depth-based disable (`task.maxRecursionDepth`)
- parent spawn allowlist via `spawns` string

Both must pass for nested task delegation.

## 10.6 Isolation conflict/failure surfaces

- non-git repo for isolated mode hard-fails with explicit message
- branch commit failure on successful agent run yields `error: Merge failed: ...`
- patch capture failure yields `error: Patch capture failed: ...`
- merge conflict returns notification and leaves branches

---

## 11) Rust architecture mapping

Recommended module split (Rust side):

1. `task_schema`
   - TaskItem/task payload schema validation
   - env-driven output truncation config
2. `task_runtime`
   - sync/async dispatch
   - validation, spawn policy, recursion policy
3. `subagent_executor`
   - child session lifecycle
   - event subscription + progress coalescing
   - submit enforcement/reminder loop
4. `subprocess_registry`
   - extract/terminate/render hook registry
5. `task_finalize`
   - submit_result normalization, fallback completion, warning prefixing
6. `task_render`
   - warning extraction, result tree rendering, nested aggregation
7. `task_isolation`
   - backend resolver + worktree/fuse/projfs adapters
   - patch/branch merge engines
8. `async_jobs`
   - queue, retries, retention, delivery state, cancellation
9. `sdk_bridge`
   - tool session wiring (`requireSubmitResultTool`, `taskDepth`, `parentTaskPrefix`, async manager)

Non-negotiable constants/strings to preserve:

- `MCP_CALL_TIMEOUT_MS = 60000`
- `PROGRESS_COALESCE_MS = 150`
- submit warnings and missing-submit prefix strings
- async summary text patterns consumed by UI/ops workflows

---

## 12) Parity test matrix (task orchestration)

## 12.1 Schema and validation

- [ ] Reject duplicate task IDs case-insensitively
- [ ] Reject missing/blank task IDs
- [ ] Enforce id max length and shape parity
- [ ] `isolated` field accepted only when isolation-enabled schema active

## 12.2 submit_result enforcement

- [ ] Success path with valid `result.data` returns exit 0 and pretty JSON output
- [ ] Error path with `result.error` marks aborted with normalized JSON `{aborted:true,error}`
- [ ] Null data triggers exact null-warning string
- [ ] No submit_result after 3 reminders triggers exact missing-warning string
- [ ] Reminder loop uses provider-specific tool choice format

## 12.3 fallback completion

- [ ] Missing submit_result + schema-valid raw JSON accepted as success
- [ ] Missing submit_result + no schema + non-empty output accepted as success
- [ ] Missing submit_result + schema-invalid output keeps warning path

## 12.4 event/progress/usage

- [ ] Progress emitted no faster than 150ms except flush events
- [ ] Tool start/end updates current tool and recent tool ring
- [ ] Usage aggregated from assistant message_end only
- [ ] Event bus channels emit raw and progress events with expected payloads

## 12.5 async orchestration

- [ ] Async `details.async` states transition `running -> completed|failed`
- [ ] Job limit enforced via `async.maxJobs`
- [ ] Scheduling failures reflected in returned text and progress state
- [ ] Async completion follow-up message contains truncated preview + artifact link when needed

## 12.6 isolation and merge

- [ ] Mode fallback notifications match platform/prereq conditions
- [ ] Worktree/fuse/projfs cleanup executes on success and failure
- [ ] Patch merge check/apply flow parity
- [ ] Branch merge sequential cherry-pick parity with first-conflict stop
- [ ] Branch cleanup only when merge fully successful
- [ ] Nested patch apply non-fatal warning preserved

## 12.7 rendering

- [ ] Missing-submit prefix extraction strips first line only
- [ ] Review verdict + findings composite rendering precedence
- [ ] Nested task extracted details rendered after main output block
- [ ] Summary counts for succeeded/failed/merge-failed/aborted parity

## 12.8 determinism and ordering

- [ ] Result ordering always follows input task order under concurrency
- [ ] Progress snapshots sorted by index
- [ ] Branch merge order deterministic and reproducible
- [ ] Artifact IDs remain stable monotonic per session via output manager
