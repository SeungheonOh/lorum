# 05 — MCP, Skills, Capability Discovery, and Extensibility Runtime

This document defines parity-critical behavior for Rust migration of MCP integration, skill loading, capability/discovery precedence, plugin/extensibility runtime, and internal URL protocols.

Primary source surface:

- MCP core:
  - `packages/coding-agent/src/mcp/{manager.ts,client.ts,types.ts,config.ts,loader.ts,tool-bridge.ts,render.ts,oauth-flow.ts,oauth-discovery.ts,tool-cache.ts,config-writer.ts,smithery-auth.ts,smithery-registry.ts,smithery-connect.ts,json-rpc.ts,transports/http.ts,transports/stdio.ts}`
- MCP control/UI:
  - `packages/coding-agent/src/modes/controllers/mcp-command-controller.ts`
  - `packages/coding-agent/src/modes/components/mcp-add-wizard.ts`
- Session/SDK integration:
  - `packages/coding-agent/src/sdk.ts`
  - `packages/coding-agent/src/session/agent-session.ts`
- Capability/discovery:
  - `packages/coding-agent/src/capability/*`
  - `packages/coding-agent/src/discovery/*`
- Extensibility:
  - `packages/coding-agent/src/extensibility/{skills.ts,slash-commands.ts,custom-tools/*,extensions/*,plugins/*}`
- Internal URL protocols:
  - `packages/coding-agent/src/internal-urls/{mcp-protocol.ts,skill-protocol.ts}`

---

## 1) MCP protocol and transport contract (wire-level)

## 1.1 Protocol version and initialization handshake

From `mcp/client.ts` + `mcp/types.ts`:

- Supported MCP protocol version is hard-coded to:
  - `"2025-03-26"`
- Initialization request:
  - JSON-RPC method: `"initialize"`
  - Params include:
    - `protocolVersion`
    - `capabilities` (`roots.listChanged=false` currently)
    - `clientInfo` (`name: "lorum-coding-agent"`, `version: "1.0.0"`)
- Post-init notification is mandatory:
  - `transport.notify("notifications/initialized")`

Rust parity requirement:

- Preserve exact version string and initialized notification method.
- Do not reorder or drop initialize fields; some servers validate shape strictly.

## 1.2 JSON-RPC request/notification schema

From `mcp/types.ts`:

- Request: `{ jsonrpc: "2.0", id, method, params? }`
- Notification: `{ jsonrpc: "2.0", method, params? }`
- Response: `{ jsonrpc: "2.0", id, result?, error? }`
- Error object: `{ code, message, data? }`

Notification method constants (server -> client):

- `notifications/tools/list_changed`
- `notifications/resources/list_changed`
- `notifications/resources/updated`
- `notifications/prompts/list_changed`

Rust parity requirement:

- Preserve method strings exactly; notification dispatch keys depend on these literals.

## 1.3 Transport behavior and timeout semantics

### HTTP transport (`mcp/transports/http.ts`)

- JSON-RPC over HTTP POST, accepts JSON or SSE response.
- Session header support:
  - Reads/writes `Mcp-Session-Id`
  - Sends DELETE on close when session exists.
- Per-operation timeout default:
  - `config.timeout ?? 30000`
- Distinct timeout errors:
  - `Request timeout after ${timeout}ms`
  - `SSE response timeout after ${timeout}ms`
  - `Notify timeout after ${timeout}ms`
- Notification POST treats `202 Accepted` as success.
- On non-OK responses, error suffix may include auth hints from:
  - `WWW-Authenticate`
  - `Mcp-Auth-Server`

### stdio transport (`mcp/transports/stdio.ts`)

- Spawns subprocess, newline-delimited JSON stdin/stdout.
- Request timeout default:
  - `config.timeout ?? 30000`
  - error: `Request timeout after ${timeout}ms`
- Pending request map keyed by JSON-RPC id.
- Notification dispatch uses messages with `method` and no `id`.

Rust parity requirement:

- Keep timeout defaults and user-visible timeout strings unchanged.
- Keep HTTP 202-notification success behavior.
- Keep session header lifecycle behavior.

---

## 2) MCP manager lifecycle, startup grace, deferred fallback, and cache

## 2.1 Startup grace timeout and degraded availability

From `mcp/manager.ts`:

- `STARTUP_TIMEOUT_MS = 250`.
- During `connectServers(...)`, all server connection/tool-load tasks run in parallel.
- Manager does `Promise.race([allSettled(tool-load), delay(250ms)])`.
- After grace timeout:
  - Pending tasks try cache lookup (`MCPToolCache.get(...)`).
  - If cache hit: create `DeferredMCPTool` entries (degraded-but-available).
  - If no cache for pending tasks: await full completion.

Meaning:

- MCP startup is intentionally latency-biased.
- System degrades to cached schemas + deferred connection resolution instead of blocking.

Rust parity requirement:

- Preserve 250ms grace behavior and deferred/cached fallback path.
- Preserve post-start background replacement when real tools eventually load.

## 2.2 Deferred MCP tool behavior

From `mcp/tool-bridge.ts`:

- `DeferredMCPTool` wraps tool schema + fallback source metadata.
- On execute:
  - waits for real connection (`getConnection()`), abort-aware
  - then performs real `tools/call`
- Failure surface:
  - returns tool content text `MCP error: <message>` unless aborted

Rust parity requirement:

- Preserve degraded availability semantics; cached tool definitions are callable once connection resolves.
- Preserve abort conversion behavior (`AbortError` -> tool abort, not generic error result).

## 2.3 MCP tool cache version/hash/TTL

From `mcp/tool-cache.ts`:

- Cache constants:
  - `CACHE_VERSION = 1`
  - `CACHE_PREFIX = "mcp_tools:"`
  - `CACHE_TTL_MS = 30 * 24 * 60 * 60 * 1000` (30 days)
- Cache payload:
  - `{ version, configHash, tools }`
- Config hash:
  - stable key-sorted clone -> JSON stringify -> SHA-256 hex
- Read validity checks:
  - version match
  - `configHash` match current config hash
  - `tools` array shape
- Writes include explicit expiry epoch seconds via storage API.

Rust parity requirement:

- Keep stable-hash algorithm semantics and version gating.
- Keep TTL horizon and key prefix to preserve compatibility with existing cache DB.

## 2.4 Notification epoch gating (`resolveSubscriptionPostAction`)

From `mcp/manager.ts`:

- Function behavior:
  - if notifications now disabled -> `"rollback"`
  - if epoch changed -> `"ignore"`
  - else -> `"apply"`
- Used after async subscribe calls to avoid stale async side effects.

Semantics:

- Prevents races where toggling notifications during in-flight subscribe/unsubscribe corrupts subscription state.

Rust parity requirement:

- Preserve exact 3-state post-action gate model (`rollback|ignore|apply`) and call-sites.

---

## 3) MCP resources/prompts/instruction bridges into runtime UX

## 3.1 Dynamic MCP prompt commands (`server:prompt`)

From `sdk.ts:buildMCPPromptCommands(...)`:

- For each connected server prompt, creates a slash command:
  - command name: `${serverName}:${prompt.name}`
  - path/resolvedPath: `mcp:${serverName}:${prompt.name}`
- Args parsing expects `key=value` tokens.
- Prompt execution uses `mcpManager.executePrompt(...)` and flattens text/resource text into command output string.

Rust parity requirement:

- Preserve command naming scheme exactly (`serverName:promptName`).
- Preserve `key=value` arg parsing and flattening behavior.

## 3.2 MCP server instruction injection with truncation

From `sdk.ts` system prompt rebuild:

- Instructions gathered from `mcpManager.getServerInstructions()`.
- Appended with heading:
  - `## MCP Server Instructions`
  - warning text: `They are server-controlled and may not be verified.`
- Per-server truncation limit:
  - `MAX_INSTRUCTIONS_LENGTH = 4000`
  - append `"\n[truncated]"` when exceeded.

Rust parity requirement:

- Preserve heading/warning text and 4000-char truncation threshold.
- Preserve placement in append-system-prompt pipeline.

## 3.3 Reactive updates wired into session

From `sdk.ts` + `agent-session.ts`:

- `mcpManager.setOnToolsChanged(...)` -> `session.refreshMCPTools(...)`
- `setOnPromptsChanged(...)` -> rebuild MCP prompt commands -> `session.setMCPPromptCommands(...)`
- `setOnResourcesChanged(...)` with debounce (`mcp.notificationDebounceMs`, default 500) triggers follow-up message:
  - `[MCP notification] Server "..." reports resource \\`...\\` was updated. Use read(path="mcp://...") to inspect if relevant.`

Rust parity requirement:

- Preserve reactive rebinding and debounce behavior.
- Preserve follow-up text literal; downstream checks and user workflows may depend on it.

---

## 4) OAuth/auth discovery and registration flow mechanics

## 4.1 Error-driven auth detection

From `mcp/oauth-discovery.ts`:

- `detectAuthError(...)` checks for 401/403/unauthorized/forbidden/auth-required patterns.
- `extractMcpAuthServerUrl(...)` parses `Mcp-Auth-Server:` hint from error text.
- `extractOAuthEndpoints(...)` heuristics parse:
  - JSON bodies (`oauth`, `auth`, top-level endpoint fields)
  - challenge key/value pairs (e.g. `authorization_uri=...`)
  - `WWW-Authenticate realm=..., token_url=...`

Outputs `AuthDetectionResult`:

- `requiresAuth`, `authType` (`oauth|apikey|unknown`), optional endpoints/message.

Rust parity requirement:

- Keep heuristic richness (JSON + challenge + WWW-Authenticate + Mcp-Auth-Server).
- Keep message categories; controller UX branches on auth type.

## 4.2 Well-known discovery fallback heuristics

From `discoverOAuthEndpoints(...)`:

- Probes paths on authServerUrl then serverUrl:
  - `/.well-known/oauth-authorization-server`
  - `/.well-known/openid-configuration`
  - `/.well-known/oauth-protected-resource`
  - `/oauth/metadata`
  - `/.mcp/auth`
  - `/authorize`
- Supports recursive follow-up through `authorization_servers` from protected-resource metadata.
- Endpoint extraction supports multiple field aliases and optional scopes/client-id derivation.

Rust parity requirement:

- Preserve path probing order and recursion for `authorization_servers`.

## 4.3 OAuth flow internals (PKCE + dynamic client registration)

From `mcp/oauth-flow.ts`:

- Uses callback server flow (`OAuthCallbackFlow`) default port 3000 path `/callback`.
- PKCE always used (`code_verifier`, `code_challenge_method=S256`).
- Client ID resolution:
  - explicit config clientId, else `client_id` from authorization URL.
- Dynamic client registration attempt when client ID absent:
  - metadata from `/.well-known/oauth-authorization-server`
  - POST registration with native/public client payload
  - stores `client_id`/`client_secret` if returned
- Token exchange posts form params with optional `client_secret` and PKCE verifier.

Rust parity requirement:

- Preserve PKCE generation/usage and fallback registration behavior.
- Preserve non-fatal behavior on registration/metadata failure.

## 4.4 Auth storage coupling

From `mcp/manager.ts` auth resolution:

- If `config.auth.type === "oauth"` and credential exists:
  - HTTP/SSE: inject `Authorization: Bearer <access>` header
  - stdio: inject env `OAUTH_ACCESS_TOKEN`
- Also resolves config values via `resolveConfigValue(...)` for env/header substitutions.

Rust parity requirement:

- Preserve transport-specific oauth token injection behavior.

---

## 5) MCP configuration/discovery and precedence rules

## 5.1 Config source model

`mcp/config.ts` loads via capability system (`mcpCapability`) then converts canonical `MCPServer` to legacy runtime config.

Behavior:

- project configs optionally disabled via `enableProjectConfig`
- disabled servers loaded from user `mcp.json` (`disabledServers` list)
- Exa/browser MCP filtering optionally applied

Validation errors from `validateServerConfig(...)` include transport exclusivity and required command/url checks.

## 5.2 Capability provider precedence and first-win dedupe semantics

From `capability/index.ts` + discovery registrations:

- Providers sorted by descending priority at registration time.
- Load across providers is parallel, but dedupe is deterministic by pre-sorted provider order:
  - first item for a capability key wins
  - duplicates marked `_shadowed=true` in `all`
- Optional validation removes invalid non-shadowed items unless `includeInvalid=true`.

Priority snapshot relevant to MCP/skills/discovery:

- 100: `native` (Lorum)
- 80: `claude`
- 70: `agents`, `codex`, `claude-plugins`
- 60: `gemini`
- 55: `opencode`
- 50: `cursor`, `windsurf`
- 40: `cline`
- 30: `github`
- 20: `vscode`
- 5: `mcp-json` fallback

Rust parity requirement:

- Keep provider priority ordering contract.
- Keep first-win key dedupe and `_shadowed` diagnostics behavior.

## 5.3 Provider enable/disable persistence and introspection APIs

From `capability/index.ts`:

- persistent disabled provider set loaded via settings `disabledProviders`
- APIs:
  - `disableProvider`, `enableProvider`, `isProviderEnabled`, `setDisabledProviders`, `getDisabledProviders`
  - introspection: `getCapabilityInfo`, `getAllCapabilitiesInfo`, `getProviderInfo`, `getAllProvidersInfo`

Rust parity requirement:

- Keep persistence and introspection API surfaces; MCP command UX and diagnostics depend on them.

---

## 6) Internal URL protocol behavior

## 6.1 `mcp://` resolution strategy and tie-break determinism

From `internal-urls/mcp-protocol.ts`:

Resolution algorithm:

1. Parse resource URI from host+path+query+hash.
2. Exact match scan across connected server concrete resources (`resource.uri===target`).
3. If no exact match, template scoring over resource templates:
   - match URI against template literals + `{...}` holes
   - maximize literal character count
   - minimize expression count
   - tie-break by server order index, then template index
4. Read with `mcpManager.readServerResource(targetServer, uri)`.

User-facing hard-coded errors:

- `mcp:// URL requires a resource URI: mcp://<resource-uri>`
- `No MCP manager available. MCP servers may not be configured.`
- `No MCP server has resource "${uri}".\n\nAvailable resources:\n...`
- `MCP resource read error: ...`
- `Server "${targetServer}" returned no content for "${uri}".`

Rust parity requirement:

- Preserve exact-match-then-template behavior and deterministic tie-break chain.
- Preserve error strings where tests or UX checks parse text.

## 6.2 `skill://` resolution and traversal safety

From `internal-urls/skill-protocol.ts`:

- `skill://<name>` reads `SKILL.md`.
- `skill://<name>/<path>` reads relative file in skill base dir.
- Security checks:
  - reject absolute paths
  - reject `..` traversal patterns
  - resolve path and enforce resolved path remains under base dir.

User-facing hard-coded errors include:

- `skill:// URL requires a skill name: skill://<name>`
- `Unknown skill: ...\nAvailable: ...`
- `Absolute paths are not allowed in skill:// URLs`
- `Path traversal (..) is not allowed in skill:// URLs`
- `Path traversal is not allowed`
- `File not found: ...`

Rust parity requirement:

- Preserve traversal guards and error behavior exactly.

---

## 7) Skills loading, ordering, collisions, and filtering

From `extensibility/skills.ts`:

- Skills discovered via capability `skillCapability`, then filtered by settings:
  - source toggles (`enableCodexUser`, `enableClaudeUser`, `enableClaudeProject`, `enablePiUser`, `enablePiProject`)
  - include/ignore glob filters
- Collision handling:
  - dedupe by realpath first (symlink-safe)
  - name collision => keep first, add warning: `name collision: "..." already loaded ...`
- Additional custom directories loaded as provider `custom`.
- Final ordering deterministic via `compareSkillOrder(...)`:
  - case-insensitive name, then exact name, then path.

Rust parity requirement:

- Preserve deterministic ordering and collision semantics for prompt stability.

---

## 8) Extensibility runtime: custom tools, extensions, plugins

## 8.1 Custom tools

From `extensibility/custom-tools/*`:

- Loaded from capability-discovered tool files + plugin tool paths + explicit configured paths.
- Name conflict policy:
  - conflicts with built-ins or previously loaded tools are rejected with explicit load error.
- Supports pending-action bridge via `pushPendingAction(...)` into shared pending-action store.

Rust parity requirement:

- Preserve loading precedence and name conflict hard-fail behavior.
- Preserve pending-action bridge semantics.

## 8.2 Extensions runtime

From `extensibility/extensions/*`:

- API supports event subscriptions, tool registration, command registration, shortcuts, flags, message renderers, provider registrations.
- Runtime starts with throwing stubs until initialized (`ExtensionRuntimeNotInitializedError`), then action methods wired by `ExtensionRunner.initialize(...)`.
- Registered commands conflict with reserved/built-in names are skipped with diagnostics.
- Shortcut conflicts and reserved shortcuts are warned/skipped.

Rust parity requirement:

- Preserve initialization guard model and extension error isolation.
- Preserve conflict diagnostics and skip behavior.

## 8.3 Plugin manifest/schema/runtime/override model

From `extensibility/plugins/types.ts`, `manager.ts`, `loader.ts`, `parser.ts`:

Manifest schema (`package.json` `lorum`/`pi`):

- base entrypoints: `tools`, `hooks`, `commands`
- optional `features` map with per-feature additive entrypoints
- optional `settings` schema (`string|number|boolean|enum`, min/max/secret/env)

Runtime state (`lorum-plugins.lock.json`):

- per-plugin: version, enabledFeatures (`null` means defaults), enabled
- global plugin settings map

Project override file (`.lorum/plugin-overrides.json` or `.pi/plugin-overrides.json` via config paths):

- disabled list
- per-plugin feature override
- per-plugin settings override

Resolution semantics:

- plugin loader ignores globally disabled and project-disabled plugins
- feature resolution precedence:
  - project override > runtime state > default feature flags
- settings merge precedence:
  - project setting overrides global runtime settings

Install spec parser semantics (`parser.ts`):

- `pkg` -> features null (defaults)
- `pkg[*]` -> all features
- `pkg[]` -> no optional features
- `pkg[a,b]` -> explicit features

Rust parity requirement:

- Preserve manifest field names and runtime/override precedence model.
- Preserve feature bracket parser behavior exactly.

---

## 9) MCP command/wizard UX behavior and migration risks

Known behavior-level mismatches in current TS to carry as explicit migration risk notes:

1. HTTP manual auth location ambiguity in wizard:
   - In `MCPAddWizard.#buildServerConfigWithAuth`, `authLocation === "env"` still writes to headers and uses `headerName`, not `envVarName`.
   - Later final build (`#buildConfig`) also maps HTTP env-mode to headers.
   - UX label implies env var semantics, but persisted config uses headers.

2. Quick-add vs wizard parity is intentionally asymmetric:
   - `/mcp add <name> -- <command...>` quick-add skips auth detection/OAuth flow.
   - URL quick-add attempts auth detection/OAuth to approximate wizard path.

3. Timeout inconsistency between test config and saved config:
   - Wizard connection-test config sets `timeout: 5000` (test-only)
   - Final persisted config omits timeout unless user specified elsewhere.

4. Scope defaults differ by entry mode:
   - command parser defaults quick-add/remove/search scope to `project`
   - wizard requires explicit scope step selection.

Rust rewrite guidance:

- Preserve current behavior for parity phase, but record these as intentional debt for post-parity cleanup.

---

## 10) Parity-critical hard-coded strings

Do not silently alter the following without compatibility review:

- MCP manager/client errors and timeouts:
  - `Connection to MCP server "..." timed out after ...ms`
  - `MCP server not connected: ...`
  - `Request timeout after ...ms`
  - `SSE response timeout after ...ms`
  - `Notify timeout after ...ms`
- Internal URL errors:
  - `mcp:// URL requires a resource URI: mcp://<resource-uri>`
  - `No MCP manager available. MCP servers may not be configured.`
  - `No MCP server has resource "...".`
  - `skill:// URL requires a skill name: skill://<name>`
  - `Path traversal (..) is not allowed in skill:// URLs`
- OAuth flow/controller messages used in UX checks:
  - `OAuth flow timed out after 5 minutes`
  - `OAuth provider requires client_id`

---

## 11) Rust rewrite architecture requirements

## 11.1 Suggested module decomposition

- `mcp/protocol.rs`
  - JSON-RPC message structs, method constants
- `mcp/transport/{http.rs,stdio.rs}`
  - timeout/session semantics isolated per transport
- `mcp/client.rs`
  - initialize/list/call/resources/prompts APIs
- `mcp/manager.rs`
  - server lifecycle, startup grace, epoch gating, refresh callbacks
- `mcp/tool_bridge.rs`
  - MCPTool + DeferredMCPTool wrappers
- `mcp/tool_cache.rs`
  - config hashing, payload versioning, TTL handling
- `mcp/oauth/{detection.rs,discovery.rs,flow.rs}`
  - auth heuristics + PKCE + DCR
- `capability/{registry.rs,types.rs}`
  - provider registration, priority ordering, dedupe, persistence hooks
- `extensibility/{skills.rs,plugins.rs,extensions.rs,custom_tools.rs}`
  - keep loading/resolution logic explicit and separately testable
- `internal_urls/{mcp.rs,skill.rs}`
  - strict protocol handlers with traversal/security checks

## 11.2 Required invariants

- deterministic ordering across provider loads, skill lists, template matches
- non-blocking degraded startup via cached/deferred MCP tools
- strict safety checks for internal URL path traversal
- exact literal protocol/method strings for JSON-RPC notifications
- runtime reactivity (MCP updates reflected in active tool/command sets without restart)

---

## 12) Subsystem migration parity checklist

1. **Protocol handshake parity**
   - initialize payload + `notifications/initialized` exactly matches TS.
2. **Transport timeout/error parity**
   - same defaults and timeout error text across HTTP/stdio paths.
3. **Startup grace/deferred parity**
   - `STARTUP_TIMEOUT_MS` behavior and deferred cached-tool fallback observed.
4. **Tool cache parity**
   - version/hash/TTL semantics and cache invalidation behavior match.
5. **Notification epoch gating parity**
   - `rollback|ignore|apply` state transitions verified under toggle races.
6. **OAuth parity**
   - PKCE + endpoint discovery + dynamic registration + fallback heuristics preserved.
7. **Prompt bridge parity**
   - dynamic `server:prompt` command generation and arg parsing preserved.
8. **Instruction injection parity**
   - MCP instruction append section and per-server 4000-char truncation preserved.
9. **Capability precedence parity**
   - provider priority matrix and first-win dedupe/shadowed handling preserved.
10. **Provider state persistence parity**
    - disable/enable persistence and introspection API outputs preserved.
11. **Skill semantics parity**
    - source filtering, collision handling, realpath dedupe, deterministic ordering preserved.
12. **Plugin/extensibility parity**
    - manifest/features/settings/runtime override precedence and parser semantics preserved.
13. **Internal URL parity**
    - `mcp://` exact-match then template scoring tie-break behavior preserved.
    - `skill://` traversal guards and error behavior preserved.
14. **Reactive runtime parity**
    - MCP resource/prompt/tool changes propagate into session toolset and commands live.
15. **Known UX mismatch lock-in (parity phase)**
    - wizard/controller quirks documented above are intentionally preserved unless explicitly changed.
