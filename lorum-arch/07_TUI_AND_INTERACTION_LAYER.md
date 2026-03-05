# 07 — TUI and Interaction Layer

This document captures parity-critical behavior for rewriting Lorum’s terminal UX and interaction runtime in Rust.

Primary source surface:

- Mode entry/orchestration:
  - `packages/coding-agent/src/cli.ts`
  - `packages/coding-agent/src/main.ts`
  - `packages/coding-agent/src/modes/index.ts`
  - `packages/coding-agent/src/modes/{interactive-mode.ts,print-mode.ts,rpc/rpc-mode.ts}`
- Interactive controllers:
  - `packages/coding-agent/src/modes/controllers/{event-controller.ts,input-controller.ts,command-controller.ts,selector-controller.ts,extension-ui-controller.ts,mcp-command-controller.ts,ssh-command-controller.ts}`
- Core components:
  - `packages/coding-agent/src/modes/components/*`
  - high-impact files: `tool-execution.ts`, `assistant-message.ts`, `custom-editor.ts`, `status-line.ts`, `footer.ts`, `read-tool-group.ts`, selector/dialog components
- Theme/render helpers:
  - `packages/coding-agent/src/modes/theme/theme.ts`
  - `packages/coding-agent/src/modes/theme/mermaid-cache.ts`
  - `packages/coding-agent/src/modes/shared.ts`
  - `packages/coding-agent/src/modes/utils/ui-helpers.ts`
  - `packages/coding-agent/src/tui/{status-line.ts,index.ts}`
- Sanitization/truncation:
  - `packages/coding-agent/src/tools/render-utils.ts`
  - `packages/coding-agent/src/modes/components/visual-truncate.ts`
  - `packages/coding-agent/src/modes/shared.ts`
  - `packages/coding-agent/src/utils/sixel.ts`

---

## 1) Mode boundaries and runtime entry points

## 1.1 CLI routing and mode dispatch

- `cli.ts` rewrites unknown top-level argv to default `launch` command.
- `main.ts` computes:
  - `autoPrint` when stdin is piped and no explicit mode/print flag.
  - `isInteractive` only when not print, not autoPrint, and no `--mode`.
- Dispatch in `main.ts`:
  - `mode === "rpc"` => `runRpcMode(session)`
  - interactive predicate true => instantiate `InteractiveMode` and enter input loop
  - else => `runPrintMode(session, { mode: "text"|"json", ... })`

Parity implications:

1. Mode dispatch is not just CLI sugar; it determines UI availability (`sessionOptions.hasUI = isInteractive`) and extension behavior.
2. RPC mode rejects `@file` args up-front in `main.ts`; preserve this hard fail.
3. Interactive loop is unbounded (`while (true)` in `runInteractiveMode`) and depends on terminal-driven shutdown, not natural function completion.

## 1.2 Mode capability matrix

- **Interactive** (`interactive-mode.ts`): full TUI, keyboard handlers, selectors/dialogs, extension terminal input listeners, visual tool rendering.
- **Print** (`print-mode.ts`): non-TTY flow, no TUI context, optional JSON event streaming to stdout.
- **RPC** (`rpc-mode.ts`): command/response protocol + streamed events; UI methods are emulated via `extension_ui_request` messages, many TUI-only APIs are explicit no-ops.

---

## 2) Interactive mode architecture (controller + component split)

## 2.1 Composition root

`InteractiveMode` owns long-lived state and wires controllers:

- Session/runtime: `session`, `sessionManager`, `agent`, settings, keybindings.
- UI nodes: `chatContainer`, `pendingMessagesContainer`, `statusContainer`, `todoContainer`, `editorContainer`, `statusLine`.
- Streaming/tool state: `streamingComponent`, `streamingMessage`, `pendingTools`, `pendingBashComponents`, `pendingPythonComponents`.
- Operation loaders: `loadingAnimation`, `autoCompactionLoader`, `retryLoader`.
- Mode toggles: plan mode, tool expansion, thinking visibility, backgrounded state.

Controllers (strict responsibility split):

- `EventController`: maps `AgentSessionEvent` stream to UI mutations.
- `InputController`: keyboard binding + submit behavior + interruption/background controls.
- `CommandController`: slash-command back-end actions and informational rendering (`/session`, `/jobs`, `/hotkeys`, `/memory`, etc.).
- `SelectorController`: model/settings/history/tree/session/OAuth/extension dashboards.
- `ExtensionUiController`: hook/extension dialogs, custom components, extension input listeners.
- `MCPCommandController` / `SSHCommandController`: domain command suites.

Rust recommendation: keep this split as independent controller services, not one giant event/input object.

## 2.2 Initialization sequence

`InteractiveMode.init()` does, in order:

1. Load keybindings and slash command state.
2. Build welcome/changelog block (unless startup quiet).
3. Construct final layout tree and focus editor.
4. Register key handlers and submit handler.
5. Load todo list.
6. Start TUI render loop.
7. Initialize extension hook UI context.
8. Restore persisted mode (`plan` / `plan_paused`).
9. Subscribe to agent events.
10. Attach theme/terminal appearance watchers.

Parity requirement: preserve this order; event subscription after UI readiness prevents early events mutating uninitialized component state.

---

## 3) Event flow: agent runtime -> session -> UI

## 3.1 Event bus path

- Agent core emits `AgentEvent`.
- `AgentSession` (`session/agent-session.ts`) receives and augments to `AgentSessionEvent` (adds auto-compaction/retry/TTSR/todo reminder events).
- `InteractiveMode` subscribes through `EventController.subscribeToAgent()`.
- `EventController.handleEvent()` mutates components and local mode state.

## 3.2 Lifecycle rendering behavior

- `agent_start`:
  - resets retry/intent/read-group state.
  - starts working loader (`Working… (esc to interrupt)`).
- `message_start`:
  - user/custom/hook/fileMention messages render immediately.
  - assistant starts streaming component instance.
- `message_update`:
  - updates assistant markdown/thinking incrementally.
  - materializes tool-call components as toolCall blocks appear.
  - updates working message from tool `intent` field.
- `tool_execution_start|update|end`:
  - updates pending tool components; handles async tool states (`details.async.state`).
  - special read-tool grouping/inlining logic (see section 4).
- `message_end`:
  - finalizes assistant block; handles aborted/error display rules.
- `agent_end`:
  - clears loaders/pending tool placeholders, applies pending model switch, completion notification when backgrounded.

Turn boundaries are consumed in `AgentSession` for extension hook emission and maintenance logic even when UI only directly reacts to subset.

---

## 4) Tool rendering stack and fallback semantics

## 4.1 Renderer precedence

`ToolExecutionComponent` rendering order:

1. Tool-defined renderer (`tool.renderCall` / `tool.renderResult`)
2. Built-in renderer registry (`tools/renderers.ts`)
3. Generic fallback formatter (`#formatToolExecution()`)

Both custom and built-in renderer paths isolate renderer exceptions (`try/catch`) and log warnings; on error, UI falls back to safe raw text output.

This fallback behavior is parity-critical for reliability: renderer failures must not crash a turn.

## 4.2 ToolExecutionComponent runtime behavior

Key behaviors in `modes/components/tool-execution.ts`:

- Supports streamed args and partial results.
- Spinner policy:
  - args streaming for `edit`/`write`.
  - partial `task` updates (excluding async-running state).
- Async edit preview computation:
  - replace/patch/hashline preview via `computeEditDiff` / `computePatchDiff` / `computeHashlineDiff`.
- Render-context injection for specific tools:
  - bash/python: raw output + previewLines + timeout for width-aware renderer logic.
  - edit: diff preview + diff renderer.
- Image handling:
  - result images can come from `content` and `details.images`.
  - kitty protocol conversion pipeline converts non-PNG to PNG asynchronously.
  - fallback to textual image indicator if images disabled/unsupported.

## 4.3 Read tool special grouping

`ReadToolGroupComponent` merges multiple `read` calls into one compact block with per-entry status and shortened path + range suffix.

`EventController` adds additional behavior:

- Associates read tool calls with current assistant component.
- Inlines read-returned images into assistant message when enabled.
- Tracks read calls that continue asynchronously.

---

## 5) Interaction affordances and command surface

## 5.1 Input model

`CustomEditor` + `InputController` implement deterministic key interception before base editor handling.

Parity-critical defaults (`config/keybindings.ts`):

- `Esc` interrupt/cancel path
- `Ctrl+C` clear/exit (double press behavior via controller timing)
- `Ctrl+D` exit when empty
- `Ctrl+Z` suspend/background
- `Shift+Tab` thinking cycle
- `Ctrl+P`/`Shift+Ctrl+P` role model cycle
- `Alt+P` temporary model selector
- `Ctrl+L` model selector
- `Ctrl+R` history search
- `Ctrl+O` tool expansion toggle
- `Ctrl+T` todo expansion toggle
- `Ctrl+G` external editor
- `Ctrl+V` clipboard image paste
- `Alt+Up` dequeue queued prompts
- `Alt+Shift+C` copy prompt
- `?` with empty editor => hotkeys panel

## 5.2 Submit behavior tiers

`InputController.setupEditorSubmitHandler()` flow:

1. Empty submit while streaming + queued messages => abort current stream to drain queue.
2. `.` or `c` => continue shortcut (empty input message).
3. Extension `input` handlers can transform/consume text+images.
4. Built-in slash command dispatch.
5. `/skill:*` expansion into custom skill message payload.
6. `!`/`!!` bash execution path, `$`/`$$` python execution path.
7. Streaming mode uses `session.prompt(..., { streamingBehavior: "steer" })`.
8. Idle mode resolves to normal prompt submission via callback.

## 5.3 Selector/dialog surfaces

- SelectorController: settings/model/history/tree/session/oauth/agent+extension dashboards.
- ExtensionUiController: hook selector/confirm/input/editor/custom dialog injection in editor region.
- Dialog timeout/abort support uses `ExtensionUIDialogOptions` across selector/input flows.

## 5.4 MCP command UX hooks

`MCPCommandController` provides `/mcp ...` management UX:

- Add/list/remove/test/reauth/unauth/enable/disable/resources/prompts/notifications/reload/smithery flows.
- Interactive add wizard (`components/mcp-add-wizard.ts`) with scope + auth transport options.
- Uses hook selectors/inputs for auth and choice prompts.
- On mutations, triggers MCP reload and tool rebinding via session (`refreshMCPTools`).

---

## 6) Async job and background UX behavior

## 6.1 Background mode

`InputController.handleBackgroundCommand()`:

- switches to no-UI extension context (`hasUI=false`) so interactive-only tools fail fast.
- unsubscribes interactive event renderer; subscribes background handler.
- stops TUI and sends SIGTSTP on POSIX.

Completion notification path:

- `EventController.sendCompletionNotification()` sends terminal notification (`TERMINAL.sendNotification`) when backgrounded and notifications enabled.

## 6.2 Async job reporting

- `UiHelpers.addMessageToChat()` handles `customType === "async-result"` with “Background job completed” line.
- `/jobs` command (`CommandController.handleJobsCommand`) renders running/recent async jobs from `session.getAsyncJobSnapshot()`.

---

## 7) Sanitization, truncation, and terminal safety constraints

## 7.1 Mandatory sanitization points

- `sanitizeStatusText` (`modes/shared.ts`): single-line status/footer sanitization (`\r\n\t` => space, collapse spaces, trim).
- `sanitizeText` (`@oh-my-pi/pi-natives`) used by:
  - bash output component
  - python output component
  - tool execution text output path
  - prompt copy preview
- SIXEL safety (`utils/sixel.ts`):
  - optional passthrough only when both env gates enabled.
  - otherwise sanitize all text.

## 7.2 Width/path/tab handling

- Tabs normalized with `replaceTabs` (`tools/render-utils.ts`) using indentation policy.
- Width-limited rendering with `truncateToWidth`/`visibleWidth` across selectors, status lines, tool previews, welcome screen.
- Path shortening via `shortenPath` (home-dir -> `~`) used in read tool groups, tree summaries, footer, MCP/SSH listings.
- `visual-truncate.ts` enforces visual-line truncation based on wrapped render lines, not raw newline count.

## 7.3 Long-line and image behavior

- Tool generic fallback clamps displayed lines/line width in collapsed mode.
- Kitty image protocol requires PNG; conversion is attempted, otherwise textual image fallback preserved.
- Assistant tool images render inline when protocol supports and setting enabled; else `[Image: mime]` fallback.

---

## 8) Error paths, abort, and cancellation UX

Parity-critical cancellation/error behavior:

- Esc while loader active => abort stream / abort compaction / abort retry depending active state.
- Esc in bash/python active execution => abort command; Esc in command mode (no active run) exits that mode.
- Auto-compaction and auto-retry install temporary Esc handlers and restore prior handler on end.
- Aborted assistant message:
  - if TTSR abort pending, assistant block is rendered as stop to avoid duplicate abort noise.
  - otherwise explicit abort message shown.
- Tool renderer exceptions never propagate to crash render; fallback output always shown.
- MCP test/reload operations support explicit cancel paths and restoration of original editor escape handler.

---

## 9) Print and RPC mode differences (non-interactive contract)

## 9.1 Print mode (`modes/print-mode.ts`)

- JSON mode emits session header then all session events as JSONL.
- Text mode emits final assistant text only; assistant `stopReason=error|aborted` exits with non-zero.
- Extension runner initialized with no UI context; UI-dependent APIs are absent/no-op.
- Flushes stdout before returning to avoid process-exit races.

## 9.2 RPC mode (`modes/rpc/rpc-mode.ts`)

- Emits `{ type: "ready" }` sentinel at startup.
- Streams all session events to stdout JSON.
- Command protocol supports prompt/abort/model/queue/session/batch operations.
- Extension UI is transported through `extension_ui_request` envelopes.
- TUI-only APIs are explicitly unsupported (theme switching, tool expansion visuals, custom footer/header/editor component insertion).

Rust parity requirement: preserve unsupported-API behavior and explicit error responses; do not silently pretend success.

---

## 10) Rust rewrite recommendations

## 10.1 Core render model

Adopt explicit unidirectional flow:

1. **Domain events** (agent/session/tool/runtime)
2. **Reducer** into immutable `UiState`
3. **Deterministic render** from `UiState` into terminal component tree
4. **Input intents** routed through controllers back to domain commands

Required invariants:

- Rendering idempotent for same state snapshot.
- Event ordering preserved per turn/tool call id.
- Renderer errors isolated to component boundary with fallback text.
- No direct business mutations inside component paint methods.

## 10.2 Event bus boundaries

Separate buses/channels:

- `AgentSessionEvent` channel (authoritative runtime stream)
- `UiCommand` channel (user intents)
- `UiNotification` channel (background completion, extension notifications)

Maintain strict bridge adapter from agent events to UI reducer so print/rpc/interactive can share same event contract but different sinks.

## 10.3 Component abstraction

Introduce Rust traits mirroring existing behavior:

- `RenderableComponent` (render, invalidate)
- `ToolRenderer` (render_call, render_result, merge_call_and_result, inline)
- `DialogHost` (selector/input/confirm/editor/custom)
- `ThemeProvider` (symbol, color, status icon, width-aware formatting)

Keep tool renderer registry data-driven (name -> renderer object), not switch-heavy procedural logic.

## 10.4 Deterministic rendering constraints

- Centralize ANSI-width and truncation helpers; no ad-hoc width math in components.
- Centralize sanitization pipeline with explicit escape-hatch for SIXEL passthrough.
- Ensure tab replacement policy is uniform across tool output, debug transcript, diff previews.
- Keep image policy deterministic by protocol + settings + conversion availability.

---

## 11) Parity test checklist (usability + non-regression)

## 11.1 Interaction matrix

1. Keybinding parity for all defaults and configured overrides.
2. Submit path parity across plain prompt, slash command, skill command, bash/python prefixes.
3. Esc behavior parity in all active states: streaming, bash running, python running, auto-compaction, auto-retry, selectors.
4. Background mode parity: suspend, notifications, no-UI hook context.

## 11.2 Event/render parity

1. Streaming assistant text/thinking updates preserve ordering and partial renders.
2. Tool call lifecycle renders for start/update/end with correct spinner and async-state handling.
3. Read tool grouping and read-image inline behavior parity.
4. Turn-end/agent-end cleanup parity (pending components, loader teardown).

## 11.3 Safety/truncation parity

1. Status/footer sanitization strips multiline/tab injection.
2. ANSI-aware truncation works under narrow widths and long model/path strings.
3. Tab expansion uses configured indentation width.
4. Long output fallback keeps deterministic “N more lines” behavior.
5. Sixel passthrough only under explicit env gates.

## 11.4 Non-interactive parity

1. Print text mode non-zero exit for assistant error/aborted.
2. Print JSON mode emits full event stream and header.
3. RPC emits ready sentinel + event stream + command response contract.
4. RPC unsupported UI methods remain explicit no-op/error pathways.

## 11.5 Failure-path parity

1. Tool renderer throw -> fallback content visible, no crash.
2. Image conversion failure -> output still rendered with fallback indicators.
3. MCP command cancel paths restore previous key handlers and UI state.
4. Auto-compaction/retry abort paths restore escape handlers and show status/warnings correctly.
