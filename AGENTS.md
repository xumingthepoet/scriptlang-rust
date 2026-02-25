# AGENTS

This repository is set up for agent-first engineering of `scriptlang-rs`.

## Startup Checklist
1. Read `/PRD.md` for product goals and non-goals.
2. Read `/TASKLIST.md` for current implementation priorities.
3. Run local environment sanity check:
   - `cargo check`
   - `cargo test`
4. Follow the quality gate before handoff:
   - `make gate`

## Required Workflow
1. **Spec first**: update `/PRD.md` when behavior/contract changes.
2. **Task alignment**: reflect implementation progress in `/TASKLIST.md`.
3. **Code after spec/task sync**: implement only after docs reflect intent.
4. **Gate before handoff**: run `make gate`.

## Boundaries
- Keep parser, compiler, runtime, and host integration isolated.
- Keep platform UI (Dioxus app) separate from language/runtime crates.
- Snapshot format must remain explicit and JSON-first.

## Quality Gates
- `cargo fmt --all -- --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all-targets --all-features`

## Definition of Done
- Relevant PRD section updated when behavior changes.
- TASKLIST state updated for completed/in-progress work.
- `make gate` passes.
