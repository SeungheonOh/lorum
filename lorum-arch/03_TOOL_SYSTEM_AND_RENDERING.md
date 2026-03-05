# 03 — Tool System and Rendering Pipeline

This document maps the end-to-end tool architecture in Lorum’s coding-agent runtime, from tool declaration to execution events to TUI rendering, then translates those findings into Rust rewrite requirements.

Primary source surface:

- `packages/agent/src/types.ts`
- `packages/agent/src/agent-loop.ts`
- `packages/coding-agent/src/tools/index.ts`
- `packages/coding-agent/src/tools/context.ts`
- `packages/coding-agent/src/tools/tool-result.ts`
- `packages/coding-agent/src/tools/output-meta.ts`
- `packages/coding-agent/src/tools/renderers.ts`
- `packages/coding-agent/src/modes/components/tool-execution.ts`
- `packages/coding-agent/src/task/subprocess-tool-registry.ts`
- `packages/coding-agent/src/tools/submit-result.ts`
- `packages/coding-agent/src/tools/review.ts`
- `packages/coding-agent/src/task/render.ts`
- `packages/coding-agent/src/tools/pending-action.ts`
- `packages/coding-agent/src/tools/resolve.ts`
- `packages/natives/src/{index.ts,native.ts,glob/index.ts,grep/index.ts,ast/index.ts}`
- `crates/pi-natives/src/{lib.rs,task.rs,glob.rs,grep.rs,ast.rs}`

---

## 1) Core tool contract (agent-core level)

The fundamental interface is `AgentTool<TParameters, TDetails, TTheme>` (`packages/agent/src/types.ts`).

### 1.1 Core shape

A tool has:

- `name`: machine identifier used by model tool-calls
- `label`: UI-facing name
- `parameters`: JSON schema (TypeBox in practice)
- `execute(toolCallId, params, signal?, onUpdate?, context?) => Promise<AgentToolResult>`

Result contract (`AgentToolResult`):

- `content: (TextContent | ImageContent)[]` (model-visible)
- `details?: TDetails` (UI/runtime metadata; may be hidden from model prompt context depending on serialization path)

### 1.2 Behavior flags that materially affect execution

`AgentTool` supports execution semantics that are heavily used in coding-agent:

- `hidden?: boolean`
  - excluded by default unless explicitly included (e.g., `submit_result`, `resolve`)
- `deferrable?: boolean`
  - tool may stage pending action requiring separate explicit `resolve`
- `nonAbortable?: boolean`
  - tool ignores abort signal
- `concurrency?: "shared" | "exclusive"`
  - scheduler-level coordination when multiple tools are emitted in one assistant turn
- `lenientArgValidation?: boolean`
  - allows bypassing strict parameter validation failure and executes with raw args

### 1.3 Rendering hooks at tool-definition level

`AgentTool` optionally includes:

- `renderCall(args, options, theme)`
- `renderResult(result, options, theme)`

This is not cosmetic; it is a first-class execution/UI split. The same tool output can be represented differently in model-facing text vs operator-facing TUI.

---

## 2) Tool lifecycle events emitted by agent-loop

The runtime emits structured events (`packages/agent/src/types.ts`, `agent-loop.ts`) used by UI and task/subagent orchestration.

Tool lifecycle:

- `tool_execution_start { toolCallId, toolName, args, intent? }`
- `tool_execution_update { ..., partialResult }` (from `onUpdate` callback)
- `tool_execution_end { ..., result, isError? }`

Turn/session lifecycle coupling:

- `turn_end { message, toolResults }`
- `agent_end { messages }`

Critical behavior in `executeToolCalls(...)` (`agent-loop.ts`):

1. Assistant `toolCall` blocks are materialized into execution records.
2. Each record is scheduled according to `concurrency`:
   - `exclusive` tools wait for all active shared tools and block following tools.
   - `shared` tools run concurrently unless blocked by preceding exclusive tool.
3. Args pass through:
   - optional intent extraction (`extractIntent`) before validation
   - `validateToolArguments`
   - lenient bypass if tool enables `lenientArgValidation`
4. `execute(...)` receives computed `toolContext` with batch metadata.
5. `onUpdate` pushes `tool_execution_update` events.
6. Errors are converted into structured tool result content with `isError = true`.
7. For aborted/interrupted execution, `tool_use/tool_result` pairing is still preserved via synthetic result generation (`createAbortedToolResult`, `createSkippedToolResult`).

Implication for Rust parity: execution correctness includes event sequence correctness, not just final outputs.

---

## 3) Coding-agent tool assembly pipeline (`createTools`)

Implemented in `packages/coding-agent/src/tools/index.ts`.

### 3.1 Registries

Two registries:

- `BUILTIN_TOOLS` (public/default-visible set)
- `HIDDEN_TOOLS` (`submit_result`, `report_finding`, `exit_plan_mode`, `resolve`)

Tool factory signature:

- `(session: ToolSession) => Tool | null | Promise<Tool | null>`

### 3.2 Session-gated inclusion logic

Inclusion depends on:

- explicit `toolNames` request (if provided)
- settings flags (`find.enabled`, `grep.enabled`, `astGrep.enabled`, etc.)
- recursion depth gate for `task` (`task.maxRecursionDepth`)
- LSP enable flag
- python mode selection and kernel availability

### 3.3 Python mode negotiation and warmup

`PI_PY` env + settings produce mode:

- `bash-only`
- `ipy-only`
- `both`

If Python unavailable, mode downgrades to bash-only.
Warmup path preloads environment via `warmPythonEnvironment` unless skipped.

### 3.4 Auto-injected companion tools

- If requested set includes `grep` and AST grep is enabled, `ast_grep` is auto-added.
- If requested set includes `edit` and AST edit is enabled, `ast_edit` is auto-added.
- `exit_plan_mode` is forced into requested set.
- If `requireSubmitResultTool`, `submit_result` is forced.
- If any selected tool is `deferrable` and `resolve` absent, hidden `resolve` is auto-added.

This implicit enrichment is important to preserve in Rust: tool set is policy-expanded, not literal.

---

## 4) Tool session context and per-call context injection

`ToolSession` in `tools/index.ts` is a large dependency bundle (cwd, settings, auth/model/MCP handles, output artifacts, pending-action store, plan mode state, etc.).

`ToolContextStore` (`tools/context.ts`) builds per-call `AgentToolContext` by merging:

- custom tool base context
- UI availability/context
- active tool names
- current `toolCall` metadata

This lets tool logic and custom extensions share runtime state without hard-coding mode-specific dependencies.

---

## 5) Result construction and metadata transport

### 5.1 Builder pattern (`tool-result.ts`)

Most tools use `toolResult(details)` fluent builder:

- set `text(...)` or explicit content
- attach truncation/limits/diagnostics/source metadata through helper methods
- finalize via `.done()`

### 5.2 Output metadata schema (`output-meta.ts`)

`details.meta` can encode:

- truncation summaries (range, bytes/lines mode, next offset, artifact id)
- source references (path/url/internal)
- diagnostics (e.g., LSP summary/messages)
- limit notices (match/result/head/column)

### 5.3 Automatic result post-processing wrapper

`wrapToolWithMetaNotice(...)` wraps every created tool execute method and:

- appends human-readable notice text to final text content based on `details.meta`
- normalizes thrown errors via `renderError(...)`

This wrapper is currently global behavior in tool construction and must remain centralized in Rust (not reimplemented per tool).

---

## 6) Deferred action model (`deferrable` + `resolve`)

### 6.1 Pending action store

`PendingActionStore` is a stack with push/pop/peek semantics and push subscriptions.

Action shape:

- `label`
- `sourceToolName`
- `apply(reason)` required
- `reject(reason)` optional
- optional details

### 6.2 Resolve tool semantics

`resolve` tool:

- requires pending action presence
- supports `action: "apply" | "discard"` and required `reason`
- pops top pending action and invokes apply/reject path
- returns merged details for renderer (`sourceToolName`, label, reason, action)

### 6.3 Practical producer example

`ast_edit` runs dry-run preview, then pushes pending action whose `apply()` executes real rewrite (`dryRun: false`).

Design consequence: execution is intentionally split into proposal -> explicit resolution, not immediate mutation.

---

## 7) Renderer architecture (TUI layer)

### 7.1 Renderer registry

`tools/renderers.ts` maps tool name to renderer contract:

- `renderCall`
- `renderResult`
- `mergeCallAndResult?`
- `inline?`

Built-ins include renderers for `bash`, `grep`, `find`, `edit`, `task`, `read`, `write`, `lsp`, `ast_*`, etc.

### 7.2 Runtime render dispatch (`ToolExecutionComponent`)

`modes/components/tool-execution.ts` selects rendering path in priority order:

1. tool-provided custom renderer (`tool.renderCall/renderResult`)
2. built-in registry renderer (`toolRenderers[name]`)
3. generic fallback text formatter

Additional behaviors:

- spinner animation for partial task output and streaming args (`edit`/`write`)
- async diff preview computation for edit tool args (`computeEditDiff`, `computeHashlineDiff`, `computePatchDiff`)
- image content handling with Kitty conversion to PNG
- per-tool `renderContext` synthesis (e.g., bash/python output + timeout, edit diff helpers)
- renderer exception isolation with fallback raw output + logging

### 7.3 Safety/sanitization in rendering

Tool output rendering routes through utilities like:

- `replaceTabs(...)`
- `truncateToWidth(...)`
- JSON tree renderers with depth/line limits

This aligns with project-wide directive: sanitize all displayed text paths including error paths.

---

## 8) Subprocess tool-event extraction/render split

Subagent execution uses a separate registry: `task/subprocess-tool-registry.ts`.

`SubprocessToolHandler` hooks:

- `extractData(event)`
- `shouldTerminate(event)`
- `renderInline(data, theme)`
- `renderFinal(allData, theme, expanded)`

This split is central for task UX:

- structured data extraction for machine behavior (`submit_result`, `report_finding`, nested `task`)
- inline progress rendering during stream
- final aggregated rendering in task result panel

Registered handlers:

- `submit_result` (`tools/submit-result.ts`): extract status/data; terminate on non-error execution
- `report_finding` (`tools/review.ts`): parse/collect review findings and render compact/final listings
- `task` (`task/render.ts`): extract nested task details and render recursively

Rust rewrite should preserve this as a dedicated subsystem, not as ad hoc parsing in task executor.

---

## 9) Built-in tool inventory and backend coupling

### 9.1 Tool inventory assembled by coding-agent

From `BUILTIN_TOOLS` + selected hidden tools:

- Core IO/search/edit: `read`, `write`, `find`, `grep`, `ast_grep`, `edit`, `ast_edit`
- Execution/compute: `bash`, `python`, `calc`, `ssh`
- Browser/network: `puppeteer` (registered under `browser` alias in code), `fetch`, `web_search`
- IDE/notebook: `lsp`, `notebook`
- Workflow/orchestration: `task`, `todo_write`, `ask`, `await`, `cancel_job`, `checkpoint`, `rewind`
- Hidden control/review: `submit_result`, `report_finding`, `resolve`, `exit_plan_mode`

### 9.2 Native-backed tools and call paths

Representative mapping:

- `find` tool -> `@oh-my-pi/pi-natives.glob(...)`
- `grep` tool -> `@oh-my-pi/pi-natives.grep(...)`
- `ast_grep` tool -> `@oh-my-pi/pi-natives.astGrep(...)`
- `ast_edit` tool -> `@oh-my-pi/pi-natives.astEdit(...)`
- text sanitization/width utilities in TUI -> native text helpers (`sanitizeText`, width/wrap helpers)

JS wrapper path:

- `packages/coding-agent/src/tools/*` -> `packages/natives/src/*` wrappers -> `native.*` bindings -> Rust N-API exports in `crates/pi-natives/src/*`

---

## 10) Native implementation characteristics relevant to rewrite

### 10.1 Shared cancellation model

Rust native operations use cooperative cancellation via `CancelToken` (`crates/pi-natives/src/task.rs`):

- timeout support
- AbortSignal bridging
- `heartbeat()` checks in long loops

### 10.2 Glob engine traits (`glob.rs`)

- uses shared filesystem scan cache (`fs_cache`) with empty-result recheck
- post-scan policy filtering (`node_modules`, hidden, gitignore)
- optional mtime sort and callback streaming

### 10.3 Grep engine traits (`grep.rs`)

- regex sanitizer for malformed brace literals (e.g., `${var}`)
- content mode vs count mode
- offset/maxCount semantics
- parallel search optimization when unlimited/no offset
- sequential deterministic mode when offset/maxCount applied
- line context capture and line truncation (`maxColumns`)

### 10.4 AST engine traits (`ast.rs`)

- language inference + explicit language override
- strictness parsing
- duplicate pattern normalization
- syntax-error detection as parse errors instead of silent failures
- deterministic sorting of matches
- rewrite overlap detection and dry-run support
- single-language enforcement for `ast_edit` when `lang` omitted and mixed-language candidate set encountered

---

## 11) Rust rewrite requirements for tool subsystem

### 11.1 Required trait boundaries

Do not collapse into a single “Tool” trait. Keep separate contracts:

1. **Tool execution contract**
   - schema, execution, cancellation, update streaming, concurrency metadata
2. **Tool output metadata contract**
   - truncation/limits/source/diagnostics, standardized notice generation
3. **Tool rendering contract**
   - call/result rendering with merge/inline policies
4. **Subprocess tool extraction contract**
   - extraction, termination predicate, inline and final aggregate rendering
5. **Deferred action contract**
   - pending actions + resolver tool

### 11.2 Must-preserve runtime behaviors

- concurrency scheduler semantics (`shared` vs `exclusive`)
- partial update event stream (`tool_execution_update`)
- synthetic tool result pairing on abort/failure paths
- auto tool-set enrichment (`submit_result`, `resolve`, AST sibling auto-add)
- dry-run + explicit resolve workflow for deferrable tools
- wrapper-based output notice and error normalization

### 11.3 Suggested Rust module decomposition

- `tool-contract` crate/module: trait + flags + schemas
- `tool-runtime`: tool assembly, policy filters, wrapper chain
- `tool-events`: event types and publisher
- `tool-render`: renderer trait + registry + fallback formatter
- `tool-metadata`: output meta builder + notice formatter
- `tool-deferred`: pending action stack + resolve adapter
- `subprocess-tool-registry`: extraction/render/terminate handlers
- `native-bridge`: napi/ffi adapters to `pi-natives` equivalents

---

## 12) Parity checks for this subsystem (to enforce during migration)

1. **Event parity**
   - For a turn with tool-calls, emitted event sequence and payloads match TS runtime.
2. **Scheduler parity**
   - Mixed shared/exclusive tool-calls execute in same ordering constraints.
3. **Error-path parity**
   - Missing tool, validation error, cancellation, timeout all produce tool_result pairing.
4. **Meta notice parity**
   - Truncation/limit/diagnostics notices append identically.
5. **Deferrable flow parity**
   - Deferrable tool proposal requires explicit `resolve`; no implicit apply.
6. **Renderer fallback parity**
   - Custom renderer failure does not crash UI and falls back to safe text render.
7. **Native search parity**
   - `find`/`grep`/`ast_*` limits, offsets, parse error surfacing, and sorting behavior preserved.

---

## 13) Open design decisions to finalize before coding Rust

1. **Schema engine choice**
   - keep JSON Schema + AJV-equivalent behavior (including lenient bypass) vs typed Rust schema wrappers + conversion layer.
2. **Renderer host model**
   - immediate-mode rendering trait vs retained component tree model compatible with existing TUI behavior.
3. **Native boundary strategy**
   - reuse current `pi-natives` crate directly from Rust rewrite core, or absorb those modules into the new runtime crate and expose compatibility shims.
4. **Custom-tool ABI**
   - whether Rust runtime continues to host TS custom tools via JS bridge, or introduces a Rust-native plugin ABI and transitional bridge.

These decisions influence crate boundaries in `08_RUST_TARGET_ARCHITECTURE.md` and sequencing in `09_IMPLEMENTATION_ROADMAP.md`.
