# 02 — AI Connectors, Auth, Streaming, and AI-State Integration

This document captures how Lorum currently implements provider integration in `packages/ai` and how `packages/coding-agent` consumes it.

---

## 1) Package and boundary architecture

## 1.1 `packages/ai` responsibilities

Primary responsibilities:

1. Provider-agnostic types (`types.ts`) and stream protocol
2. Provider adapters (`providers/*.ts`) that normalize vendor streams to one event model
3. Auth/credential lifecycle (`auth-storage.ts` + `utils/oauth/*`)
4. Dynamic model discovery and cache (`model-manager.ts`, `model-cache.ts`, `provider-models/*`)
5. Usage limit reporting and credential ranking (`usage.ts` + `usage/*`)
6. Extensibility points for custom APIs and OAuth providers (`api-registry.ts`, `utils/oauth/index.ts`)

Export surface is intentionally broad and acts as a shared runtime SDK (`packages/ai/src/index.ts`).

## 1.2 `packages/coding-agent` AI integration boundaries

`coding-agent` does not directly implement provider protocol details. It composes them:

- `sdk.ts:createAgentSession(...)` is composition root
- `config/model-registry.ts` wraps `@oh-my-pi/pi-ai` model/discovery APIs and adds user config + extension registration behavior
- `session/auth-storage.ts` is only a re-export boundary to `@oh-my-pi/pi-ai`
- `main.ts` bootstraps `AuthStorage + ModelRegistry` before mode dispatch
- `session/agent-session.ts` owns runtime model switching and provider-session reset hooks

---

## 2) Canonical AI type system (shared contract)

Source: `packages/ai/src/types.ts`

## 2.1 Core unions

- API types:
  - `KnownApi` includes `openai-completions`, `openai-responses`, `openai-codex-responses`, `azure-openai-responses`, `anthropic-messages`, `bedrock-converse-stream`, `google-generative-ai`, `google-gemini-cli`, `google-vertex`, `cursor-agent`
  - `Api = KnownApi | string` (extensible)
- Provider types:
  - `KnownProvider` enumerates built-ins
  - `Provider = KnownProvider | string` (extension-safe)

## 2.2 Message/stream contracts

- Assistant content blocks are normalized into:
  - `TextContent`
  - `ThinkingContent`
  - `ToolCall`
- `AssistantMessage` contains:
  - provider/api/model identifiers
  - normalized usage (`input/output/cacheRead/cacheWrite/totalTokens/cost`)
  - `stopReason` (`stop | length | toolUse | error | aborted`)
- Streaming event union (`AssistantMessageEvent`) includes lifecycle events:
  - start
  - text/thinking/toolcall start+delta+end
  - terminal done/error

This contract is the most important port target for a Rust rewrite; all providers normalize to this.

## 2.3 Provider option map pattern

`ApiOptionsMap` maps each `KnownApi` to a provider-specific options type with compile-time exhaustiveness checks.

Rust equivalent recommendation:

- enum `ApiKind`
- trait `ProviderAdapter`
- associated type / enum for options payload per adapter
- compile-time registry check in tests/macros

---

## 3) Model registry and discovery pipeline

## 3.1 Static + dynamic merge model

Core logic: `packages/ai/src/model-manager.ts`

`resolveProviderModels(options, strategy)` pipeline:

1. Start with static models (`options.staticModels` or bundled `models.json`)
2. Read sqlite cache (`model-cache.ts`)
3. Optionally fetch models.dev fallback (`options.modelsDev`)
4. Optionally fetch dynamic provider endpoint (`fetchDynamicModels`)
5. Merge order: static -> models.dev -> cache -> dynamic
6. Dynamic entries field-merge over base model (`mergeDynamicModel`)

Important semantics:

- `stale` is tied to authoritative source state
- non-authoritative cache retry backoff is explicit (`NON_AUTHORITATIVE_RETRY_MS`)
- malformed model records are dropped via validation (`isModelLike`)

## 3.2 Cache implementation

Source: `packages/ai/src/model-cache.ts`

- Single sqlite DB (`models.db`) instead of per-provider JSON cache
- table: `model_cache(provider_id, version, updated_at, authoritative, models_json)`
- WAL mode + busy timeout
- best-effort read/write failure handling (cache errors do not break model resolution)

## 3.3 Provider descriptor system

Source: `packages/ai/src/provider-models/descriptors.ts`

- `ProviderDescriptor` is single source for:
  - `providerId`
  - `defaultModel`
  - `createModelManagerOptions(...)`
  - optional unauthenticated discovery allowance
  - optional catalog discovery metadata
- `DEFAULT_MODEL_PER_PROVIDER` derived from descriptors + special providers

This is a strong pattern to keep in Rust: declarative provider metadata + constructor closure.

## 3.4 Coding-agent model overlay layer

Source: `packages/coding-agent/src/config/model-registry.ts`

Adds product-level behavior on top of `pi-ai` discovery:

- merges built-in + models config file + runtime extension-registered providers
- supports provider-level overrides (`baseUrl/headers/apiKey`)
- supports per-model overrides
- supports local discovery providers (Ollama/LM Studio)
- supports keyless provider mode (`auth: none`)
- resolves fallback API keys from provider config through `AuthStorage` fallback resolver
- supports extension dynamic registration (`registerProvider`), including custom stream adapter and OAuth provider

Critical coupling point:

- extension runtime queues provider registrations, then `createAgentSession` flushes them into `ModelRegistry` before final model selection.

---

## 4) Auth storage and credential lifecycle

Primary source: `packages/ai/src/auth-storage.ts`

## 4.1 Storage model

Two-layer design:

1. `AuthStorage` runtime manager
2. `AuthCredentialStore` sqlite persistence implementation

Credential types:

- `api_key`
- `oauth` (`refresh/access/expires` + optional identity metadata)

SQLite schema:

- table `auth_credentials`
  - soft-delete via `disabled` flag
  - stores serialized JSON payload in `data`
- table `cache` for usage/report cache

Filesystem/security posture:

- default DB path: `<agentDir>/agent.db`
- parent dir created `0700`
- DB chmod `0600` (best effort)
- WAL mode + busy timeout

## 4.2 API key resolution priority

`AuthStorage.getApiKey(provider, sessionId, options)` order:

1. runtime override (`--api-key`)
2. persisted API key credentials
3. OAuth credentials (with refresh path)
4. environment variables (`stream.ts` provider map)
5. fallback resolver (custom provider API keys from model config)

## 4.3 OAuth runtime behavior

`#resolveOAuthApiKey(...)` + `#tryOAuthCredential(...)`:

- multiple oauth credentials per provider supported
- round-robin/session-affinity ordering
- optional usage-based ranking strategy per provider
- temporary backoff for blocked credentials
- refresh on expiry
- definitive failures remove credential (invalid_grant, revoked, strong 401/403 signals)
- transient failures temporarily block credential

## 4.4 Usage-aware credential ranking

`usage.ts` defines normalized quota report schema; providers in `usage/*` implement fetchers and ranking.

This allows choosing the least exhausted credential before hard rate-limit failures.

---

## 5) OAuth provider framework

Primary source: `packages/ai/src/utils/oauth/index.ts`

## 5.1 Built-in provider model

- Built-in provider list for UI discovery (`getOAuthProviders()`)
- Central refresh dispatcher (`refreshOAuthToken(provider, creds)`)
- `getOAuthApiKey(...)` normalizes token refresh behavior and provider-specific API key derivation

## 5.2 Custom OAuth provider extension point

- `registerOAuthProvider(provider)`
- `getOAuthProvider(id)`
- `unregisterOAuthProviders(sourceId)`

This is how extension-defined providers integrate with `/login` and runtime auth resolution.

## 5.3 Callback-server flow abstraction

Source: `utils/oauth/callback-server.ts`

`OAuthCallbackFlow` provides:

- preferred-port + random-port fallback callback server startup
- CSRF `state` generation and validation
- callback parsing and timeout cancellation
- race between callback server and manual code input (`onManualCodeInput`)
- provider subclasses implement `generateAuthUrl` and `exchangeToken`

## 5.4 Provider-specific examples

### OpenAI Codex (`utils/oauth/openai-codex.ts`)

- PKCE flow against `auth.openai.com`
- scope includes `offline_access`
- extracts account id from JWT custom claim
- refresh endpoint integration

### Cursor (`utils/oauth/cursor.ts`)

- generates challenge/uuid login URL
- polling flow with bounded retries/backoff
- refresh via bearer exchange endpoint
- expiry inferred from token JWT payload

---

## 6) Provider dispatch and stream normalization

## 6.1 Dispatcher

Source: `packages/ai/src/stream.ts`

- entrypoints: `stream(...)`, `complete(...)`, `streamSimple(...)`, `completeSimple(...)`
- custom API registry checked first (`getCustomApi(model.api)`)
- special auth providers (Vertex, Bedrock) bypass API-key check path
- provider-specific stream adapter selected by `model.api`

## 6.2 Delta event transport behavior

Source: `packages/ai/src/utils/event-stream.ts`

`AssistantMessageEventStream`:

- terminal events resolve final result promise
- `text_delta`, `thinking_delta`, `toolcall_delta` are throttled and merged (50ms window)
- non-delta events flush pending deltas immediately

This decouples raw provider token cadence from UI event storm.

## 6.3 Anthropic adapter behavior

Source: `providers/anthropic.ts`

- provider-local retry loop for transient failures and rate limits
- transforms content block events to canonical lifecycle events
- incremental tool JSON argument parsing (`parseStreamingJson`)
- usage updated from `message_start` and `message_delta`
- strict stop-reason mapping with exhaustiveness failure on unknown reason

## 6.4 OpenAI Responses adapter behavior

Source: `providers/openai-responses.ts`

- maps response item stream (`reasoning`, `message`, `function_call`) to canonical content events
- enforces strict call/result pairing when configured
- tool result images transformed into follow-up user image input message
- usage finalized at completion event
- stop reason mapped from response status with forced `toolUse` when tool calls exist

## 6.5 OpenAI Codex Responses stateful transport

Source: `providers/openai-codex-responses.ts`

Notable advanced behavior:

- supports SSE and websocket transport
- maintains provider session state map (`providerSessionState`) keyed by provider constant
- supports session-level websocket reuse/append semantics
- fallback from websocket to SSE with retry budget and disable flags
- exposes transport detail for provider diagnostics (`provider-details.ts`)

This is the canonical example of provider-local mutable transport state using `providerSessionState` in shared stream options.

---

## 7) Coding-agent startup, session restore, and model lifecycle coupling

## 7.1 Startup wiring in `main.ts`

- creates `AuthStorage` via `discoverAuthStorage()`
- creates `ModelRegistry(authStorage)` and `refresh(...)`
- assembles `CreateAgentSessionOptions`
- passes `authStorage` and `modelRegistry` into `createAgentSession`

`--api-key` is runtime override only (non-persistent) and attached to the selected provider.

## 7.2 Composition root in `sdk.ts`

`createAgentSession` responsibilities include:

1. auth/model/settings/session initialization
2. model restore strategy:
   - try saved session model
   - fallback to settings default role
   - fallback to first available keyed model
3. deferred model pattern resolution after extension provider registration
4. agent construction with provider-aware `getApiKey(provider)` callback
5. appending initial `model_change` and `thinking_level_change` for new sessions
6. restoring session messages + thinking defaults for existing sessions

## 7.3 Runtime model/thinking persistence in `AgentSession`

- `setModel` persists both session log and settings role
- `setModelTemporary` persists session log only
- `setThinkingLevel` appends thinking-level change entry on transition
- model switch triggers provider-session reset logic for stateful providers (`openai-codex-responses`)

## 7.4 Session switching behavior

`switchSession(...)`:

1. flush current writes
2. set new session file
3. replace in-memory agent messages with rebuilt context
4. restore model if available in `getAvailable()` set
5. restore/clamp thinking level; backfill if absent

Risk worth preserving in rewrite docs: model restore can silently fail if unavailable, leaving current model active.

---

## 8) OAuth UX integration in coding-agent

Relevant files:

- `/login` slash command: `slash-commands/builtin-registry.ts`
- interactive flow orchestration: `modes/controllers/selector-controller.ts`
- manual callback state machine: `modes/oauth-manual-input.ts`

Flow:

1. user runs `/login [provider]`
2. selector/controller invokes `authStorage.login(provider, callbacks)`
3. callback URL shown in TUI and opened in browser
4. manual callback URL/code can be pasted into `/login <url>` when needed
5. success path refreshes `ModelRegistry` and prints credential DB location

---

## 9) Rust rewrite design requirements derived from current behavior

Minimum parity requirements for AI/auth subsystem:

1. **Canonical stream protocol** independent from providers
2. **Provider adapter trait** with normalized event output
3. **Model discovery manager** with merge precedence and authoritative cache semantics
4. **Credential manager** with:
   - persisted API key + OAuth records
   - runtime overrides
   - refresh + ranking + backoff
5. **OAuth framework** with callback server abstraction + manual fallback input path
6. **Extensibility hooks** for custom provider stream adapters and custom OAuth providers
7. **Session-aware provider state map** for advanced transports (Codex websocket reuse pattern)
8. **Composition root** that resolves model/auth/session order exactly (including deferred extension model registration)

These map directly to trait boundaries in `08_RUST_TARGET_ARCHITECTURE.md`.
