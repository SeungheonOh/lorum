# 13 — Cycle 1 Provider Error Mapping Compatibility

## Scope

This document locks provider-specific error normalization and retry semantics for Cycle 1 connectors/auth.

Applies to:

- `crates/lorum-ai-connectors/src/lib.rs`
- `crates/lorum-ai-auth/src/lib.rs`

---

## A) Connector-level normalized error contract

Connector adapters normalize stream/provider failures into `lorum-ai-contract::ProviderError`.

Primary normalization entry point:

- `normalize_provider_error(code, message, retryable)` in `lorum-ai-connectors`

### Mapping table

| Input condition | Normalized error |
|---|---|
| `code` contains `"rate"` (case-insensitive) | `ProviderError::RateLimited { message }` |
| otherwise and `retryable == true` | `ProviderError::Transport { message }` |
| otherwise | `ProviderError::InvalidResponse { message: "{code}: {message}" }` |

### Retry behavior (applies to all adapters)

`RetryPolicy::should_retry(attempt, error)` retries only when:

1. `attempt < max_attempts`
2. error is one of:
   - `ProviderError::RateLimited { .. }`
   - `ProviderError::Transport { .. }`

No retries for:

- `ProviderError::Auth { .. }`
- `ProviderError::InvalidRequest { .. }`
- `ProviderError::InvalidResponse { .. }`
- `ProviderError::Unknown { .. }`

---

## B) Anthropic adapter mapping behavior

Adapter emits stream events and final message for valid frame sequences.

Anthropic frame `Error { code, message, retryable }` maps via common normalization:

- uses `normalize_provider_error`
- emits `AssistantMessageEvent::Error` before returning mapped `ProviderError`

Transport failures without explicit provider code map as `ProviderError::Transport`.

---

## C) OpenAI Responses adapter mapping behavior

OpenAI Responses frame `Error { code, message, retryable }` mapping is identical to Anthropic:

- normalized via `normalize_provider_error`
- emits `AssistantMessageEvent::Error`
- returns normalized `ProviderError`

Tool-call JSON parse failures map to:

- `ProviderError::InvalidResponse { message }`

Sink push failures map to:

- `ProviderError::Transport { message: sink_error }`

---

## D) OpenAI Codex adapter mapping behavior

Transport order:

1. websocket (if enabled)
2. sse fallback

Error semantics:

- websocket transport error triggers fallback to sse when configured
- state persisted with `websocket_disabled = true` on fallback path
- if all attempts fail, returns last error or fallback transport error:
  - default terminal fallback: `ProviderError::Transport { message: "codex transport failed without explicit error" }`

Frame-level error mapping for codex stream uses OpenAI Responses parser path, therefore same normalization table as section C.

---

## E) OAuth refresh error mapping (auth crate)

`OpenAiCodexOAuthProvider` maps token refresh responses to `OAuthRefreshError`:

| OAuth error code | Mapped `OAuthRefreshError` |
|---|---|
| `invalid_grant` | `InvalidGrant` |
| `revoked` | `Revoked` |
| `unauthorized`, `invalid_client` | `Unauthorized` |
| `forbidden` | `Forbidden` |
| `temporarily_unavailable`, `timeout` | `Transient(description)` |
| any other code | `Permanent("{code}: {description}")` |

### Resolver reaction rules

In `AuthResolver::get_api_key` for expired OAuth credentials:

- refresh success: upsert refreshed credential, clear transient block, continue selection
- definitive failure (`InvalidGrant`, `Revoked`, `Unauthorized`, `Forbidden`, `Permanent(_)`):
  - disable credential in store
  - clear transient block
  - continue to next credential
- non-definitive failure (`Transient(_)`):
  - mark credential blocked for `transient_block_secs`
  - continue to next credential

If no usable credential remains, resolver continues precedence chain (env/fallback) and eventually returns `Ok(None)` when exhausted.

---

## Compatibility lock for Cycle 1

The mapping tables and retryability behavior above are contract-locked for Cycle 1.

Any change requires:

1. design note with rationale
2. regression proof across contract/golden/smoke suites
3. explicit gate sign-off
