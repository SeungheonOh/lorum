# 01 — System Overview (Lorum)

## 1) Monorepo composition and responsibility boundaries

Primary packages relevant to an agentic coding runtime:

- `packages/coding-agent/` — top-level CLI/runtime orchestration, tools, TUI modes, MCP bridge, extensibility system, task/sub-agent framework
- `packages/ai/` — provider abstraction layer (Anthropic/OpenAI/Gemini/etc), auth storage, model registry data, rate limit utilities, usage accounting
- `packages/agent/` — core agent loop and tool-call execution contract
- `packages/tui/` — terminal rendering primitives/components and interaction model
- `packages/natives/` — JS/TS bindings over Rust-native high-performance operations
- `packages/utils/` — shared infra (logging, async helpers, stream utilities, env/path utils)
- `crates/pi-natives/` — Rust implementation for grep/glob/text/shell/AST and other low-level capabilities

Supporting but less central packages include `stats/`, `swarm-extension/`, and benchmark tooling.

## 2) Coding-agent source topology (current)

`packages/coding-agent/src/` contains the runtime composition root.

Key directories:

- `cli/`, `commands/` — command parsing, command entrypoints, launch flow
- `main.ts`, `cli.ts` — process entry and root orchestration
- `modes/` — interactive TUI mode, print mode, RPC mode
- `session/` — `AgentSession`, session persistence, storage, compaction, artifacts
- `tools/` — built-in tools and rendering/meta contracts
- `task/` — sub-agent spawning, isolation, output aggregation, orchestration
- `mcp/` — MCP transport/manager/tool-bridge, OAuth flow glue, registry integration
- `extensibility/` — skills, custom tools, custom commands, plugin hooks/extensions
- `lsp/`, `exec/`, `ipy/`, `ssh/`, `web/` — specialized subsystems consumed by tools
- `internal-urls/` — `agent://`, `artifact://`, `skill://`, `memory://`, etc resource handlers
- `patch/` — edit/patch model and applicator logic

## 3) Runtime boot/lifecycle (high level)

### 3.1 CLI boot chain

1. `src/cli.ts` parses argv and normalizes default command to `launch`
2. command handler in `src/commands/*` calls into `runRootCommand(...)` (`src/main.ts`)
3. root orchestration initializes settings/theme/auth/model registry/session manager
4. creates `AgentSession` via SDK path
5. dispatches runtime mode:
   - interactive TUI
   - non-interactive print
   - JSONL RPC server

### 3.2 Core runtime stack

- **Agent core** (`packages/agent`) emits structured events
- **AgentSession** (`packages/coding-agent/src/session/agent-session.ts`) translates between core runtime and persistence/UI integrations
- **Mode runtime** (`modes/`) materializes UX surface (terminal UI, text stream, RPC server)
- **Tools subsystem** (`tools/`) implements capability surface exposed to LLM
- **External integrations** (MCP, LSP, web, exec, native bindings) are consumed as adapters

## 4) Architectural qualities to preserve in a Rust rewrite

1. **Strong layering**: CLI/orchestration is separate from agent loop and tool implementations
2. **Tool contract + render split**: tool execution and user-facing rendering are decoupled
3. **Mode polymorphism**: same agent session can run in interactive, print, or RPC mode
4. **Session durability**: append-only session log + branch/tree semantics + compaction
5. **Extensibility as first-class**: plugins/skills/custom tools/commands/hooks are core runtime features
6. **Bridge architecture**: MCP and native modules are adapters behind stable interfaces
7. **Evented design**: internal state transitions are event-driven and streamable

## 5) Detailed follow-up docs in this plan

- AI provider/auth stack: `02_AI_CONNECTORS_AND_AUTH.md`
- Tool execution + rendering + native integration: `03_TOOL_SYSTEM_AND_RENDERING.md`
- Edit/hash-anchor mechanics: `04_EDITING_AND_PATCHING_ENGINE.md`
- MCP/skills/extensibility: `05_MCP_SKILLS_AND_EXTENSIBILITY.md`
- Sub-agents/task orchestration: `06_SUBAGENTS_TASKS_AND_ORCHESTRATION.md`
- TUI/interaction architecture: `07_TUI_AND_INTERACTION_LAYER.md`
- Rust target architecture: `08_RUST_TARGET_ARCHITECTURE.md`
- Build order and migration strategy: `09_IMPLEMENTATION_ROADMAP.md`
- Risks/parity checklist: `10_RISK_REGISTER_AND_PARITY_CHECKLIST.md`

## 6) Current analysis status

This document is written incrementally while deep-reading subsystem implementation files. Subsequent docs include file-level behavior and implementation patterns to port.
