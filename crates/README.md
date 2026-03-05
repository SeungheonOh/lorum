# Crate Purpose Inventory

This workspace is split into focused crates so each layer owns one concern.

## `lorum-ai-contract`
**Purpose:** Shared, provider-agnostic AI contract types and traits.

**Owns:**
- `ApiKind`, `ModelRef`
- assistant message/event schema (`AssistantMessage`, `AssistantMessageEvent`, deltas, stop reasons)
- provider interfaces (`ProviderAdapter`, request/context/final/error types)

**Does not own:**
- credential storage, provider HTTP transports, runtime orchestration

## `lorum-ai-testkit`
**Purpose:** Deterministic validation and regression tooling for assistant event streams.

**Owns:**
- fixture loading/parsing
- sequence/order/terminal invariants (`assert_valid_sequence`, `assert_deterministic_ordering`)
- snapshot/hash generation and regression report helpers

**Does not own:**
- live provider logic or runtime execution

## `lorum-ai-auth`
**Purpose:** Authentication and credential resolution for AI providers.

**Owns:**
- credential data model + SQLite store
- API key + OAuth resolution policy (`AuthResolver`)
- OAuth flow primitives (`OAuthProvider`, callback parsing, token refresh)
- OAuth provider bootstrap/config helpers (`OAuthProviderCatalog`, env mapping helpers)

**Does not own:**
- provider response streaming adapters or runtime event orchestration

## `lorum-ai-models`
**Purpose:** Model catalog resolution and cache management.

**Owns:**
- model metadata types (`ModelInfo`, descriptors)
- model source abstraction (`ModelSource`)
- SQLite model cache (`SqliteModelCache`)
- merge/precedence + stale/non-authoritative cache behavior

**Does not own:**
- auth, provider transport, or runtime event flow

## `lorum-ai-connectors`
**Purpose:** Provider adapter implementations and transport integration for AI backends.

**Owns:**
- adapter implementations (`OpenAiResponsesAdapter`, `AnthropicAdapter`, `OpenAiCodexResponsesAdapter`)
- transport interfaces and retry/state policies
- connector runtime catalog (`ProviderCatalog`, model presets, provider registry assembly)
- frame parsing/coalescing helpers

**Does not own:**
- UI loop, session persistence, or runtime command orchestration

## `lorum-domain`
**Purpose:** Core domain model shared across runtime and UI.

**Owns:**
- IDs (`SessionId`, `TurnId`, `MessageId`)
- runtime event schema (`RuntimeEvent`)
- turn terminal reasons and event-order validation
- UI command/notification envelope types

**Does not own:**
- storage engines, provider integrations, command loops

## `lorum-session`
**Purpose:** Session event persistence/replay abstraction and baseline in-memory implementation.

**Owns:**
- `SessionStore` trait
- `InMemorySessionStore`
- session switch metadata result model

**Does not own:**
- runtime turn execution or UI rendering

## `lorum-agent-core`
**Purpose:** Turn execution state machine that bridges provider events to runtime events.

**Owns:**
- `TurnEngine` contract and `ChatTurnEngine`
- mapping assistant stream deltas to `RuntimeEvent`
- terminal/error classification and ordering guards
- cancellation semantics + emission guardrails

**Does not own:**
- auth lookup policy, provider registry wiring, UI presentation

## `lorum-ui-core`
**Purpose:** UI reducer/state logic for runtime event consumption.

**Owns:**
- `UiState` and derived turn/session buffers
- `UiReducer` trait + `DefaultUiReducer`
- UI-side sequencing/session consistency checks

**Does not own:**
- transport protocol, terminal rendering, provider/auth integrations

## `lorum-ui-print`
**Purpose:** Text/JSONL rendering and exit-code mapping for runtime events.

**Owns:**
- textual transcript rendering
- JSON-lines event rendering
- exit code policy (`done` vs `aborted` vs runtime error)

**Does not own:**
- interactive command handling or runtime orchestration

## `lorum-ui-rpc`
**Purpose:** RPC envelope schema and serialization for UI/runtime transport.

**Owns:**
- protocol version and `RpcEnvelope` variants (`ready`, `event`, `error`)
- JSON encoding helpers for RPC payloads

**Does not own:**
- socket/process transport plumbing or runtime execution

## `lorum-runtime`
**Purpose:** Chat runtime orchestration that wires auth, model selection, provider registry, and session sink.

**Owns:**
- runtime controller API (`submit_user_input`, `set_model`, `subscribe`)
- model override lifecycle
- provider selection + turn invocation via `lorum-agent-core`
- persistence + broadcast fan-out to subscribers

**Does not own:**
- provider-specific transport implementations, terminal command parsing

## `lorum-tui`
**Purpose:** Interactive terminal application that composes runtime/UI/auth/connectors crates into a runnable chat interface.

**Owns:**
- CLI command loop (`/help`, `/status`, `/history`, `/use`, `/model`, `/apikey`, `/login`)
- runtime/auth/model/provider catalog wiring
- OAuth callback capture UX and browser/manual fallback
- display of streamed/runtime events
- top-level application composition of crate integrations
- `lorum-agent-core` usage is **indirect** via `lorum-runtime` (not a direct dependency of `lorum-tui`)

**Does not own:**
- provider adapter internals or auth persistence internals
- turn-state machine implementation (that is `lorum-agent-core`, consumed by `lorum-runtime`)

---

## Dependency direction (high-level)
- Contracts/domain at the bottom: `lorum-ai-contract`, `lorum-domain`
- Turn engine: `lorum-agent-core`
- Runtime orchestrator: `lorum-runtime` (depends on `lorum-agent-core`)
- Implementations in the middle: `lorum-ai-auth`, `lorum-ai-models`, `lorum-ai-connectors`, `lorum-session`, `lorum-ui-*`
- Composition entrypoint at the top: `lorum-tui` (depends on `lorum-runtime`, not directly on `lorum-agent-core`)
