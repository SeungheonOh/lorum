# 08 — Rust Target Architecture (Canonical Blueprint)

This document is the implementation blueprint for rebuilding the coding-agent runtime in Rust while preserving parity-critical behavior from docs 01–07.

Scope: workspace architecture, trait seams, runtime model, persistence/protocol compatibility, and cutover rules.

---

## 1) Architectural goals and parity envelope

### 1.1 Hard goals

1. Preserve observable behavior required by current CLI/TUI/print/RPC, tool execution, MCP, editing, and subagent orchestration.
2. Establish explicit crate boundaries so parallel teams can implement independently.
3. Keep deterministic ordering guarantees where TS runtime depends on ordering for correctness or stable UX.
4. Keep protocol and parser literals stable where downstream parsing/tests rely on exact strings.

### 1.2 Parity-critical invariants (non-negotiable)

- MCP protocol version string remains `"2025-03-26"`.
- MCP initialized notification remains `"notifications/initialized"`.
- MCP notification method strings remain exact:
  - `notifications/tools/list_changed`
  - `notifications/resources/list_changed`
  - `notifications/resources/updated`
  - `notifications/prompts/list_changed`
- Edit/patch error literals remain exact where documented:
  - `"old_text must not be empty."`
  - `"Diff contains no hunks"`
  - `"File not found: ${path}"`
- Subagent warning literals/prefixes remain exact:
  - `"SYSTEM WARNING: Subagent called submit_result with null data."`
  - `"SYSTEM WARNING: Subagent exited without calling submit_result tool after 3 reminders."`
  - Prefix: `"SYSTEM WARNING: Subagent exited without calling submit_result tool"`
- Deterministic ordering preserved for:
  - task result order == input task order
  - progress snapshots sorted by task index
  - branch merge/cherry-pick order == input task order
  - capability first-win dedupe by provider priority order
  - skill ordering (`compareSkillOrder` equivalent)
  - MCP template tie-break chain (literal chars, expr count, server index, template index)

### 1.3 Deliberate improvements allowed behind parity feature flags

Improvements are allowed only behind explicit feature flags and must default OFF in parity phase:

- richer typed errors (while preserving rendered text)
- stricter schema diagnostics
- optimized transport pooling/retries
- renderer performance improvements
- plugin ABI modernization

---

## 2) Workspace and crate decomposition

Rust workspace (`Cargo.toml` root) with explicit layering:

### 2.1 Product-facing binaries

1. `lorum-agent-cli`
   - process entrypoint, argv parsing, mode dispatch
   - launches interactive/print/rpc runtimes

2. `lorum-agent-rpcd` (optional split binary)
   - dedicated long-lived RPC daemon mode if separation desired

### 2.2 Core runtime crates

1. `lorum-domain`
   - canonical domain models/events shared by all modes
   - IDs, enums, normalized message/tool event contracts

2. `lorum-agent-core`
   - agent loop orchestration
   - tool-call scheduling (`shared`/`exclusive` semantics)
   - synthetic tool_result pairing on abort/error

3. `lorum-session`
   - session state machine
   - append-only log management
   - model/thinking/session-switch persistence semantics

4. `lorum-runtime`
   - composition root equivalent of TS `sdk.ts`
   - wires auth/model registry, tools, MCP, extensions, async jobs

### 2.3 AI/provider stack

1. `lorum-ai-contract`
   - canonical streaming contracts (`AssistantMessageEvent` equivalent)
   - provider/api/model types

2. `lorum-ai-connectors`
   - provider adapters (Anthropic/OpenAI/Google/Bedrock/etc)
   - stream normalization into canonical union
   - provider-session transport state map (Codex-style)

3. `lorum-ai-auth`
   - credential store/manager (API key + OAuth)
   - refresh, ranking, backoff/blocking lifecycle
   - fallback API key resolver chain

4. `lorum-ai-models`
   - static + dynamic model discovery pipeline
   - sqlite model cache with authoritative semantics
   - provider descriptors/default models

### 2.4 Tooling stack

1. `lorum-tool-contract`
   - tool definition trait, schema descriptor, execution metadata

2. `lorum-tool-runtime`
   - tool registry assembly
   - policy-based inclusion and auto-enrichment (`submit_result`, `resolve`, AST companions)
   - output-meta notice wrapper and error normalization wrapper

3. `lorum-tool-render`
   - renderer registry and fallback renderers
   - merge call/result policies and inline rendering hints

4. `lorum-tool-deferred`
   - pending action store
   - resolve apply/discard flow

5. `lorum-edit-engine`
   - replace/patch/hashline engines + shared normalization/fuzzy/diff
   - LSP-agnostic fs adapter boundary

6. `lorum-native-bridge`
   - bridge surface to high-performance search/AST/text ops
   - houses N-API compatibility layer during migration

### 2.5 MCP/extensibility stack

1. `lorum-mcp-protocol`
   - JSON-RPC structs/constants/method strings

2. `lorum-mcp-transport`
   - HTTP/SSE + stdio transports
   - timeout/session header semantics

3. `lorum-mcp-client`
   - initialize/list/call/resources/prompts APIs

4. `lorum-mcp-manager`
   - startup grace/deferred fallback/cache replacement
   - server lifecycle, notification subscriptions, epoch-gated side effects

5. `lorum-capability`
   - provider registry, priority ordering, first-win dedupe, shadow diagnostics

6. `lorum-extensibility`
   - skills/plugins/extensions/custom tools loading + conflict policy
   - runtime bridges and guard states

7. `lorum-internal-urls`
   - `mcp://`, `skill://`, `artifact://`, `agent://`, `memory://`, `local://`, etc

### 2.6 Task/subagent stack

1. `lorum-task-schema`
   - task payload validation, env-driven output truncation settings

2. `lorum-subagent-executor`
   - child-session lifecycle, submit enforcement/reminders, progress coalescing

3. `lorum-task-finalize`
   - submit_result normalization and fallback acceptance paths

4. `lorum-task-render`
   - missing-submit warning extraction, nested aggregation, review overlays

5. `lorum-task-isolation`
   - none/worktree/fuse-overlay/fuse-projfs backends
   - patch/branch merge strategy

6. `lorum-async-jobs`
   - async job manager queue/retry/retention/delivery suppression

### 2.7 Frontend/mode stack

1. `lorum-ui-core`
   - mode-agnostic event reducer and `UiState`
   - UI command and notification channels

2. `lorum-ui-tui`
   - interactive terminal components/controllers
   - tool execution component, selector/dialog host, status/footer/editor

3. `lorum-ui-print`
   - text/json output stream renderer

4. `lorum-ui-rpc`
   - RPC command protocol + event stream + extension UI request bridging

---

## 3) Core domain model and event contracts

### 3.1 Domain model ownership

`lorum-domain` is single source of truth for:

- Session identifiers, task identifiers, tool call identifiers
- Assistant message content blocks (text/thinking/tool-call)
- Tool execution events and turn lifecycle events
- Async task progress snapshots
- MCP/extension notifications translated into session events

### 3.2 Event model split

Three distinct event channels:

1. `RuntimeEvent` (authoritative agent/session/tool events)
2. `UiCommand` (user intent -> runtime)
3. `UiNotification` (runtime -> user notification side-channel)

No direct component mutation from runtime internals; everything flows through reducer-friendly events.

### 3.3 Deterministic sequencing rules

- Runtime emits events in turn-stable order by `(turn_id, sequence_no)`.
- Tool events within a turn are emitted with scheduler-aware causality:
  - start before update before end for each call
  - exclusivity barriers preserve global ordering constraints
- UI reducer consumes events in received order and is pure/idempotent.

---

## 4) Critical Rust trait seams (pseudocode)

### 4.1 Provider adapters and stream normalization

```rust
#[async_trait]
pub trait ProviderAdapter: Send + Sync {
    fn api_kind(&self) -> ApiKind;
    fn provider_id(&self) -> &str;

    async fn stream(
        &self,
        req: ProviderRequest,
        ctx: ProviderContext,
        sink: &mut dyn AssistantEventSink,
    ) -> Result<ProviderFinal, ProviderError>;

    async fn complete(
        &self,
        req: ProviderRequest,
        ctx: ProviderContext,
    ) -> Result<AssistantMessage, ProviderError>;

    fn supports_stateful_transport(&self) -> bool { false }
}

pub trait AssistantEventSink {
    fn push(&mut self, ev: AssistantMessageEvent) -> Result<(), StreamSinkError>;
}
```

### 4.2 Auth/OAuth and credential ranking

```rust
#[async_trait]
pub trait CredentialStore: Send + Sync {
    async fn list_credentials(&self, provider: &str) -> Result<Vec<CredentialRecord>, AuthError>;
    async fn upsert(&self, rec: CredentialRecord) -> Result<(), AuthError>;
    async fn disable(&self, credential_id: &str) -> Result<(), AuthError>;
}

#[async_trait]
pub trait AuthResolver: Send + Sync {
    async fn get_api_key(&self, provider: &str, session_id: &str, opts: ApiKeyOptions)
        -> Result<ApiKeyResolution, AuthError>;
}

#[async_trait]
pub trait OAuthProvider: Send + Sync {
    fn id(&self) -> &str;
    async fn begin_flow(&self, ctx: OAuthBeginContext) -> Result<OAuthStart, AuthError>;
    async fn exchange_code(&self, code: &str, verifier: Option<&str>) -> Result<OAuthCredential, AuthError>;
    async fn refresh(&self, cred: &OAuthCredential) -> Result<OAuthCredential, AuthError>;
}
```

### 4.3 Model registry/discovery/cache

```rust
#[async_trait]
pub trait ModelRegistry: Send + Sync {
    async fn refresh(&self, opts: RefreshOptions) -> Result<(), ModelError>;
    async fn available(&self) -> Result<Vec<ModelInfo>, ModelError>;
    async fn resolve_default(&self, role: ModelRole) -> Result<ModelInfo, ModelError>;
    async fn register_provider(&self, registration: ProviderRegistration) -> Result<(), ModelError>;
}
```

### 4.4 Tool execution and rendering

```rust
#[async_trait]
pub trait ToolExecutor: Send + Sync {
    fn descriptor(&self) -> ToolDescriptor; // name, schema, flags
    async fn execute(
        &self,
        call_id: ToolCallId,
        params: serde_json::Value,
        ctx: ToolExecutionContext,
        updates: &mut dyn ToolUpdateSink,
    ) -> Result<ToolResult, ToolError>;
}

pub trait ToolUpdateSink {
    fn update(&mut self, partial: ToolPartialResult);
}

pub trait ToolRenderer: Send + Sync {
    fn render_call(&self, call: &ToolCallRenderInput, theme: &Theme) -> RenderBlock;
    fn render_result(&self, result: &ToolResultRenderInput, theme: &Theme) -> RenderBlock;
    fn merge_call_and_result(&self) -> bool { false }
    fn inline(&self) -> bool { false }
}
```

### 4.5 Deferred actions (`resolve` flow)

```rust
#[async_trait]
pub trait PendingAction: Send + Sync {
    fn label(&self) -> &str;
    fn source_tool(&self) -> &str;
    async fn apply(&self, reason: &str) -> Result<serde_json::Value, ToolError>;
    async fn reject(&self, reason: &str) -> Result<serde_json::Value, ToolError>;
}

pub trait PendingActionStore: Send + Sync {
    fn push(&self, action: Arc<dyn PendingAction>);
    fn pop(&self) -> Option<Arc<dyn PendingAction>>;
    fn peek(&self) -> Option<Arc<dyn PendingAction>>;
}
```

### 4.6 MCP transport/client/manager

```rust
#[async_trait]
pub trait McpTransport: Send + Sync {
    async fn request(&self, req: JsonRpcRequest, timeout_ms: u64) -> Result<JsonRpcResponse, McpError>;
    async fn notify(&self, method: &str, params: Option<serde_json::Value>, timeout_ms: u64) -> Result<(), McpError>;
    async fn close(&self) -> Result<(), McpError>;
}

#[async_trait]
pub trait McpClient: Send + Sync {
    async fn initialize(&self, init: McpInitializeParams) -> Result<McpInitializeResult, McpError>;
    async fn list_tools(&self) -> Result<Vec<McpTool>, McpError>;
    async fn call_tool(&self, req: McpToolCall) -> Result<McpToolResult, McpError>;
    async fn list_resources(&self) -> Result<Vec<McpResource>, McpError>;
    async fn read_resource(&self, uri: &str) -> Result<McpResourceContent, McpError>;
    async fn list_prompts(&self) -> Result<Vec<McpPrompt>, McpError>;
    async fn execute_prompt(&self, name: &str, args: McpPromptArgs) -> Result<McpPromptResult, McpError>;
}

#[async_trait]
pub trait McpManager: Send + Sync {
    async fn connect_all(&self) -> Result<(), McpError>;
    async fn refresh(&self) -> Result<(), McpError>;
    async fn get_tools(&self) -> Vec<ToolDescriptor>;
    async fn on_tools_changed(&self, cb: Box<dyn Fn() + Send + Sync>);
    async fn on_prompts_changed(&self, cb: Box<dyn Fn() + Send + Sync>);
    async fn on_resources_changed(&self, cb: Box<dyn Fn(McpResourceUpdate) + Send + Sync>);
}
```

### 4.7 Capability providers and extensibility bridges

```rust
#[async_trait]
pub trait CapabilityProvider: Send + Sync {
    fn id(&self) -> &str;
    fn priority(&self) -> i32;
    async fn load(&self) -> Result<Vec<CapabilityItem>, CapabilityError>;
}

#[async_trait]
pub trait ExtensionBridge: Send + Sync {
    async fn initialize(&self, ctx: ExtensionInitContext) -> Result<(), ExtensionError>;
    async fn emit_event(&self, event: &RuntimeEvent) -> Result<(), ExtensionError>;
    async fn collect_tool_registrations(&self) -> Result<Vec<CustomToolRegistration>, ExtensionError>;
    async fn collect_provider_registrations(&self) -> Result<Vec<ProviderRegistration>, ExtensionError>;
}
```

### 4.8 Subagent execution and finalization

```rust
#[async_trait]
pub trait SubagentExecutor: Send + Sync {
    async fn run(&self, req: SubagentTaskRequest, sink: &mut dyn SubagentProgressSink)
        -> Result<SingleTaskResult, TaskError>;
}

pub trait SubagentProgressSink {
    fn on_event(&mut self, event: SubagentRuntimeEvent);
    fn on_progress(&mut self, progress: AgentProgressSnapshot);
}

pub trait TaskFinalizer: Send + Sync {
    fn finalize(&self, run: RawSubagentRun, schema: Option<&JtdSchema>) -> FinalizedSubagentOutput;
}
```

---

## 5) Runtime model decisions

### 5.1 Async runtime and cancellation

- Runtime: Tokio (multi-thread scheduler for general runtime; `LocalSet` where single-thread affinity required).
- Cancellation primitives:
  - root `CancellationToken` per session
  - child token per turn
  - child token per tool call/subagent
- Propagation rules:
  - parent cancel cancels descendants immediately
  - exclusive tool barriers honor cancellation before enqueuing next item
  - deferred actions can still be resolved only if pending action remains valid
- Cooperative cancellation in long-running native operations must heartbeat-check token.

### 5.2 Bounded concurrency

- Use `tokio::sync::Semaphore` for:
  - task max concurrency
  - async job max active jobs
  - MCP connection parallelism guard
- Use `FuturesOrdered` where deterministic output order required.
- Use explicit priority queues only where current semantics imply deterministic first-in scheduling.

### 5.3 Serialization and schema boundaries

- Primary serialization: `serde_json` for wire/tool payloads.
- Session and artifact persistence: JSON Lines for append-only event logs (wire-compatible schema IDs).
- Config/settings: existing JSON files retained for compatibility.
- Schema validation boundaries:
  - tool params validated at tool runtime boundary
  - task output schema validated in `task_finalize`
  - MCP wire payload validated at transport/client boundary
- JTD support for task output contracts retained (existing behavior parity).

### 5.4 Error model

- Internal errors use typed enums (`thiserror`) by subsystem.
- User-visible error rendering passes through compatibility formatter that preserves required literal strings.
- Distinguish:
  - abort/cancel
  - validation error
  - transport timeout
  - protocol incompatibility
  - internal unexpected error

### 5.5 N-API / FFI strategy

Phase 1 (parity-first):

- Keep `pi-natives` as Rust crate and expose stable N-API surface expected by existing JS modules where needed for mixed runtime.
- New Rust runtime can call native internals directly via crate linkage, bypassing N-API internally.

Phase 2 (post-parity):

- Remove JS-facing N-API dependency where no longer needed.
- Keep compatibility shim binary/interfaces only for extension ecosystems still expecting JS entrypoints.

---

## 6) Subsystem architecture details

### 6.1 AI provider connector architecture

Pipeline:

1. Session/model selection chooses `(provider, api, model)`.
2. `AuthResolver` resolves key/token with precedence:
   - runtime override
   - persisted API key
   - OAuth (+refresh)
   - env fallback
   - provider config fallback resolver
3. `ProviderAdapter` streams canonical events.
4. `AssistantEventNormalizer` merges/throttles deltas (~50ms parity behavior).
5. Finalized assistant message persisted + emitted.

Stateful provider transport state:

- `ProviderSessionStateStore` keyed by `(session_id, provider_constant)`; supports websocket reuse/fallback semantics.

### 6.2 Tool execution engine and renderer separation

Execution path:

1. Assemble tool set from built-ins + extensions + policy filters.
2. Auto-enrich required companions (`ast_*`, `resolve`, `submit_result`, `exit_plan_mode`) under existing rules.
3. Validate args; allow lenient bypass only for tools with flag.
4. Schedule by `shared`/`exclusive` semantics.
5. Emit lifecycle events and partial updates.
6. Wrap final result with meta notices + normalized error surface.

Rendering path:

- Resolve renderer by precedence:
  1. tool-supplied renderer
  2. registry renderer
  3. generic fallback
- Renderer failures are isolated and must not crash runtime/UI.

### 6.3 Editing/patch subsystem

`lorum-edit-engine` retains three independent engines:

1. replace engine
2. patch engine
3. hashline engine

Shared requirements:

- pre-mutation full validation
- strict ambiguity fail-fast
- line-ending/BOM restore semantics
- hashline anchor validation and mismatch diagnostics with remap
- preview computation interfaces for UI (`replace/patch/hashline`)

LSP integration:

- `EditFileSystem` trait allows plain FS and LSP writethrough adapters without coupling edit logic to LSP internals.

### 6.4 MCP manager/transport/auth/discovery

`lorum-mcp-manager` behavior:

- parallel connect/list attempts
- startup grace timeout `250ms`
- fallback to cached/deferred tool descriptors for still-pending servers
- eventual replacement with live schemas when connection resolves
- notification post-action epoch gate returns one of `rollback|ignore|apply`

Auth/discovery:

- preserve OAuth detection heuristics (JSON/challenge/WWW-Authenticate/Mcp-Auth-Server)
- preserve well-known probing order and recursive `authorization_servers` behavior
- preserve PKCE + dynamic client registration fallback behavior

### 6.5 Skills/plugins/extensions/custom tools runtime

- Capability loading is parallel, but dedupe is deterministic by pre-sorted provider priority.
- Skills:
  - source toggles/filtering
  - realpath dedupe
  - name-collision keep-first + warning
  - deterministic ordering
- Plugins:
  - manifest features/settings model parity
  - state and project override precedence parity
- Extensions:
  - initialization guard with explicit not-initialized errors
  - conflict detection for commands/shortcuts with skip+diagnostics

### 6.6 Task/subagent orchestration and isolation

Subagent runtime phases:

1. validate task payload and spawn policy
2. create child session with required `submit_result`
3. run prompt, consume events, coalesce progress (`150ms`)
4. reminder loop up to 3 attempts when submit_result missing
5. finalize output using decision table (submit payload, fallback parse, warnings)
6. persist output artifact and emit final result

Isolation backends:

- mode resolution with platform fallbacks and warning notifications
- patch merge mode and branch merge mode retained
- deterministic merge order and conflict handling retained

### 6.7 TUI/print/RPC frontends and reducer model

Shared core:

- all frontends consume same `RuntimeEvent` stream
- interactive mode adds `UiCommand` input loop

Interactive mode:

- controller split retained (`Event`, `Input`, `Command`, `Selector`, `ExtensionUI`, `MCP`, `SSH`)
- initialization ordering preserved (subscribe after UI readiness)
- state transitions handled by reducer + effect handlers

Print mode:

- text mode exits non-zero on assistant error/aborted
- JSON mode emits session header + event stream

RPC mode:

- emits ready sentinel
- explicit unsupported UI API responses remain explicit (no fake success)

### 6.8 Persistence/session/artifacts/internal URLs

Persistence:

- append-only session event log with compaction support
- durable artifacts store with stable artifact IDs
- sqlite for auth/model cache/tool cache

Internal URL protocols:

- protocol handlers registered centrally (`InternalUrlResolverRegistry`)
- `mcp://` resolution strategy and error strings preserved
- `skill://` traversal guards and error strings preserved
- `agent://`, `artifact://`, `memory://`, `local://`, `jobs://`, `pi://` remain routed via explicit resolver implementations

---

## 7) Compatibility and cutover strategy

### 7.1 Wire-compatible surfaces to keep stable

1. Tool parameter and result JSON shapes for built-ins.
2. MCP protocol payload shape/version/method literals.
3. Session event JSON envelopes used by print/RPC and tests.
4. Warning/error literals documented as parser-dependent.
5. Internal URL behavior and key error texts.

### 7.2 Feature-flagged behavior improvements

Flags (default OFF during parity):

- `rust.strict_schema_diagnostics`
- `rust.optimized_provider_retries`
- `rust.renderer_incremental_diff_cache`
- `rust.plugin_native_abi`

Each flag must have:

- explicit test gate
- fallback parity behavior path
- release note entry

### 7.3 Migration surface and test gates

Stage gates:

1. **Contract tests**: tool schemas, event order, warning literals.
2. **Golden transcript tests**: TS vs Rust turn/event outputs for representative scenarios.
3. **Protocol tests**: MCP handshake/notifications/timeouts/auth detection.
4. **Mutation safety tests**: edit replace/patch/hashline edge/ambiguity cases.
5. **Orchestration tests**: submit_result enforcement, fallback completion, async state transitions, isolation merge behavior.
6. **Mode tests**: interactive reducer snapshots, print exit behavior, RPC command protocol.

Cutover policy:

- default runtime remains TS until all subsystem gates pass.
- switch by config flag at process entry.
- no mixed semantic mode inside one session instance.

---

## 8) Implementation ownership map for parallel teams

1. Team A: `lorum-ai-*` (contracts/connectors/auth/models)
2. Team B: `lorum-tool-*` + `lorum-edit-engine`
3. Team C: `lorum-mcp-*` + `lorum-capability` + `lorum-internal-urls`
4. Team D: `lorum-extensibility`
5. Team E: `lorum-subagent-*` + `lorum-task-*` + `lorum-async-jobs`
6. Team F: `lorum-ui-*` frontends + reducer model
7. Team G: integration in `lorum-runtime` + `lorum-session` + `lorum-agent-core`

Shared integration contracts are only in `lorum-domain` and trait crates; teams must not import peer implementation crates directly.

---

## 9) Subsystem acceptance checklist (definition of done)

### 9.1 Core/runtime/session

- [ ] Session/event log append, restore, switch, and compaction semantics match baseline.
- [ ] Model/thinking persistence and restore fallback order preserved.
- [ ] Event ordering deterministic and reproducible under concurrency.

### 9.2 AI/auth/models

- [ ] Canonical stream events produced for all supported providers.
- [ ] Delta throttling/merge behavior parity validated.
- [ ] Auth resolution precedence parity validated.
- [ ] OAuth refresh/backoff/revocation behavior parity validated.
- [ ] Model merge precedence (`static -> models.dev -> cache -> dynamic`) preserved.

### 9.3 Tool runtime/render

- [ ] Shared/exclusive scheduler semantics match TS behavior.
- [ ] Tool lifecycle events and synthetic pairing on abort/failure preserved.
- [ ] Auto tool-set enrichment behavior preserved.
- [ ] Meta notice wrapper and error normalization wrapper globally applied.
- [ ] Renderer fallback on renderer panic/error verified.

### 9.4 Editing/patch/hashline

- [ ] Runtime mode selection precedence (`env/model/global`) preserved.
- [ ] Replace ambiguity/no-match diagnostics parity validated.
- [ ] Patch parser accepted/rejected forms parity validated.
- [ ] Hashline mismatch/remap diagnostics parity validated.
- [ ] Newline/BOM/indentation preservation parity validated.

### 9.5 MCP/capability/extensibility

- [ ] MCP initialize payload and initialized notification literal parity validated.
- [ ] HTTP/stdio timeout defaults and timeout strings parity validated.
- [ ] Startup grace (250ms) + deferred cached-tool fallback parity validated.
- [ ] Notification epoch gating (`rollback|ignore|apply`) race tests pass.
- [ ] Capability priority and first-win dedupe/shadow diagnostics parity validated.
- [ ] Skills collision/order/filtering parity validated.
- [ ] Plugin manifest/feature/settings precedence parity validated.

### 9.6 Task/subagent/isolation

- [ ] `submit_result` enforced (injection + reminder loop + fallback behavior) parity validated.
- [ ] Missing/null submit warning literals and prefix parsing parity validated.
- [ ] Progress coalescing cadence (`150ms`) parity validated.
- [ ] Async state transitions and job-limit behavior parity validated.
- [ ] Isolation fallback warnings and merge behavior parity validated.
- [ ] Deterministic task result ordering preserved under concurrency.

### 9.7 Frontends (TUI/print/RPC)

- [ ] Interactive init ordering and controller responsibilities preserved.
- [ ] Tool rendering precedence/fallback and read grouping behavior preserved.
- [ ] Print mode JSON/text contracts and exit-code behavior preserved.
- [ ] RPC ready sentinel, event streaming, and unsupported UI API responses preserved.
- [ ] Sanitization/truncation/image fallback behavior parity validated.

### 9.8 Persistence/internal URL protocols

- [ ] Auth/model/tool caches remain readable across migration.
- [ ] Artifact/session formats are backward-readable or migrated with tested converter.
- [ ] `mcp://` and `skill://` resolution/traversal/error behavior parity validated.
- [ ] Remaining internal protocols (`agent://`, `artifact://`, `memory://`, `local://`, `jobs://`, `pi://`) covered by contract tests.

---

This blueprint is the canonical target architecture for Rust implementation. Any deviation from listed parity invariants requires explicit RFC and compatibility review.