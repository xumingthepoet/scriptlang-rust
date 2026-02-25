# scriptlang-rs PRD (Phase 1)

## Goals
- Rebuild ScriptLang core capabilities in Rust with industrial-grade modularity.
- Deliver parser/compiler/runtime/api/agent-cli using Rhai as embedded script language.
- Keep agent CLI protocol compatible with existing script-lang implementation (`RESULT/EVENT/TEXT_JSON/...`).

## Non-goals (Phase 1)
- No TUI player.
- No JS syntax compatibility layer.
- No snapshot backward compatibility with TS `snapshot.v2`.
- No automatic migration tool from JS to Rhai.

## Product Decisions
- Snapshot schema is `snapshot.v3`.
- Player state schema is `player-state.v3`.
- Host integration uses typed Rust trait interfaces.

## Implemented Contracts (Current)
- Workspace split: `sl-core`, `sl-parser`, `sl-compiler`, `sl-runtime`, `sl-api`, `sl-cli`.
- Runtime pull API: `next/choose/submit_input/snapshot/resume`.
- Compiler supports include graph, defs/json/script roots, loop macro expansion, core node set.
- Runtime supports control flow, choice/input boundary, call/return/ref, deterministic random, once-state, snapshot/resume.
- Agent CLI supports `agent start/choose/input` and state persistence.
- Rhai examples under `/examples/scripts-rhai` are smoke-tested end-to-end.

## Known Gaps (Explicit)
- Host function dispatch trait exists, but invoking host functions from Rhai scripts is not wired yet.
- Player-state portable codec currently uses direct JSON serialization (not yet tagged Map/non-finite encoding).
- Test coverage gate exists via lint/test/gate, but not yet strict 100% coverage threshold tooling.

## Acceptance Criteria (Phase 1 target)
- Rust library and CLI build and run independently.
- Rhai examples execute through CLI smoke.
- `make gate` passes.
