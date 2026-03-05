# Rust Rewrite Plan for Lorum Coding Agent

This directory contains a complete implementation and migration plan for rebuilding the coding-agent runtime in Rust while preserving parity-critical behavior.

## Document Map

- `00_INDEX.md` — master index and reading order
- `01_SYSTEM_OVERVIEW.md` — full system architecture and runtime lifecycle
- `02_AI_CONNECTORS_AND_AUTH.md` — provider integrations, OAuth/auth storage, model registry, streaming
- `03_TOOL_SYSTEM_AND_RENDERING.md` — tool contracts, execution pipeline, native bindings, renderer architecture
- `04_EDITING_AND_PATCHING_ENGINE.md` — hash-anchor edit workflow, patching semantics, safety checks
- `05_MCP_SKILLS_AND_EXTENSIBILITY.md` — MCP integration, skills discovery/loading, extension model
- `06_SUBAGENTS_TASKS_AND_ORCHESTRATION.md` — task delegation, sub-agent lifecycle, planning/todo orchestration
- `07_TUI_AND_INTERACTION_LAYER.md` — terminal UX architecture and rendering constraints
- `08_RUST_TARGET_ARCHITECTURE.md` — Rust module blueprint, trait contracts, async/runtime strategy
- `09_IMPLEMENTATION_ROADMAP.md` — phased build order with acceptance criteria
- `10_RISK_REGISTER_AND_PARITY_CHECKLIST.md` — migration risks, compatibility strategy, test matrix
- `11_RUST_CYCLE1_AI_CONNECTORS_PLAN.md` — detailed Cycle 1 milestone plan for AI/connectors/auth/models
- `12_CYCLE1_SPEC_LOCK_AND_HARDENING.md` — Cycle 1 compatibility lock and hardening criteria
- `13_PROVIDER_ERROR_MAPPING_COMPAT.md` — provider-specific error normalization and retry compatibility
- `14_CYCLE1_RC_TEST_REPORT_AND_DEFECT_LEDGER.md` — Cycle 1 release-candidate validation evidence
- `15_AGENTIC_LOOP_FIRST_REPLAN.md` — revised ordering: chat-only agentic loop before tool runtime
- `16_PHASE2A_EXECUTION_PLAN.md` — executable milestone plan for Phase 2A chat-only runtime delivery
- `17_MODULE_INTERFACE_CONTRACTS_AND_DETAILED_ARCHITECTURE.md` — explicit crate interfaces, ownership contracts, sequencing, and failure semantics
- `18_PHASE3_TOOL_RUNTIME_EXECUTION_PLAN.md` — concrete execution plan for tool runtime, renderer, deferred actions, and native bridge migration
- `19_CORE_UI_FIRST_REPLAN.md` — revised sequencing that inserts Phase 2B core/UI hardening before Phase 3 work
- `20_PHASE2B_AGENT_UI_IMPLEMENTATION_BLUEPRINT.md` — file-level implementation blueprint for Phase 2B hardening in agent-core/ui-core/runtime↔ui boundary

## Status

- [x] Repository-wide subsystem mapping
- [x] AI connector deep analysis
- [x] Tool + rendering deep analysis
- [x] MCP/skills/sub-agent deep analysis
- [x] Rust target architecture + roadmap
- [x] Final cross-check and consistency pass

- [x] Cycle 1 AI/auth/models/connectors implemented and hardened
- [x] Cycle 1 RC report and spec-lock docs published
- [x] Roadmap updated to require Phase 2A (agentic chat loop) before tool runtime
- [ ] Phase 2A execution and parity sign-off
- [x] Phase 2A execution plan authored with milestones, tests, and gates
- [x] Detailed module interface contracts and architecture spec published (doc 17)
- [x] Phase 2A M2A.0 bootstrap crates implemented and verified (chat-only runtime skeleton live)
- [x] Phase 3 execution plan drafted with milestones, gates, and risk controls (doc 18)
- [x] Core/UI-first replan published with Phase 2B gate (doc 19)
- [ ] Phase 2B hardening and sign-off (agent-core/ui-core/runtime↔ui contract freeze)
- [x] Detailed Phase 2B implementation blueprint published (doc 20)