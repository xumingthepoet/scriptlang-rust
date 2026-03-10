# scriptlang-rs

Rust workspace implementation of ScriptLang (Phase 1), with Rhai as the embedded script engine.

## Documentation
- [SL Engine API usage](docs/sl-engine-api.md): host-side Rust API and runtime integration.
- [SL CLI usage](docs/sl-cli-usage.md): `agent`/`tui` commands, machine output, replay workflow.
- [SL Lint usage](docs/sl-lint-usage.md): standalone lint checks and output contract.
- [ScriptLang syntax rules](docs/scriptlang-syntax.md): XML grammar and language semantics.
- [Example testing with sl-test-example runner](docs/testing-examples.md): example-case contract and runner usage.
- [Rust testability playbook for high coverage](docs/rust-testability-playbook.md): testing patterns and coverage tactics.

## Workspace Crates
- `crates/sl-core`: shared types, values, errors, snapshot/player schemas.
- `crates/sl-parser`: XML parser + import directive extraction.
- `crates/sl-compiler`: import graph validation + module/script compilation to compiled artifact.
- `crates/sl-runtime`: execution engine (`next/choose/submit_input/snapshot/resume`).
- `crates/sl-api`: high-level create/compile/resume API.
- `crates/sl-cli`: host-side CLI (`agent` and `tui` modes).
- `crates/sl-lint`: standalone ScriptLang lint CLI for source quality checks.
- `crates/sl-test-example`: example integration tests + in-crate testcase runner/assertion.

## Public Surface
- Stable/recommended user-facing entry points:
  - `sl-api` (Rust host integration)
  - `sl-cli` (command-line host tooling)
  - `sl-lint` (standalone lint tooling)
- Other crates (`sl-core/sl-parser/sl-compiler/sl-runtime/sl-test-example`) are internal building blocks and not recommended as direct integration entry points.

## Internal Module Layout
- `crates/sl-cli/src`:
  `lib.rs` only coordinates modules; runtime logic is split into
  `cli_args.rs`, `models.rs`, `source_loader.rs`, `state_store.rs`, `session_ops.rs`,
  `boundary_runner.rs`, `line_tui.rs`, `error_map.rs`, plus `agent.rs` and `tui.rs`.
  Ratatui internals are separated into `tui_state.rs`, `tui_actions.rs`, `tui_render.rs`.
  `error_map.rs` uses a shared mapper helper to keep CLI error conversions concise and consistent.
- `crates/sl-runtime/src`:
  public entry is `lib.rs -> engine/mod.rs`; engine logic is split into
  `engine/lifecycle.rs`, `step.rs`, `boundary.rs`, `snapshot.rs`, `frame_stack.rs`,
  `callstack.rs`, `control_flow.rs`, `eval.rs`, `scope.rs`, `once_state.rs`, `rng.rs`;
  helpers are in `helpers/value_path.rs` and `helpers/rhai_bridge.rs`.
- `crates/sl-compiler/src`:
  compile pipeline is split into `artifact.rs`, `context.rs`, `pipeline.rs`, `source_parse.rs`,
  `import_graph.rs`, `module_resolver.rs`, `error_context.rs`, `type_expr.rs`,
  `sanitize.rs`, `script_compile.rs`, `xml_utils.rs`, `macro_expand.rs`, `defaults.rs`.

This split keeps crate boundaries unchanged and enforces one-way internal dependencies.

## Testability Requirements (IMPORTANT)
All code must be written with testability in mind:
- **One-to-one test file mapping**: Each source file should have a corresponding test module in the same file (`#[cfg(test)] mod tests { ... }`).
- **Test order**: Test functions must be defined in the same order as the functions they test within each file.
- **No compatibility burden in this phase**: This is a development stage; do not introduce extra version compatibility handling unless explicitly required.
- **99.5% region coverage required**: `make gate` enforces a minimum compiler/runtime region coverage threshold of `99.50%`.
- **Write tests first**: When fixing bugs or adding features, write the failing test first (TDD approach).
- **Test support helpers**: Use the `*_test_support` modules provided by each crate for common test utilities.
- **Host-facing paths should fail gracefully**: For CLI/artifact/state IO paths, prefer returning typed errors instead of panicking assertions.

## Commands
- `make check`: `cargo check --workspace --all-targets --all-features`
- `make fmt`: `cargo fmt --all -- --check`
- `make lint`: `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `make test`: `cargo test --workspace --all-targets --all-features` with `LLVM_PROFILE_FILE` unset
- `cargo test -p sl-cli --bin sl-cli`: run `sl-cli` binary target unit tests (`src/main.rs`).
- `cargo test -p sl-test-example --all-targets --all-features`: run example cases from `crates/sl-test-example/examples/*/testcase.json`
- `make coverage`: runs `scripts/coverage.sh` (uses `cargo llvm-cov --workspace --exclude sl-cli --exclude sl-test-example  --all-features --all-targets --show-missing-lines` inside) and prints:
- `make coverage`: runs `scripts/coverage.sh` (uses `cargo llvm-cov --workspace --exclude sl-cli --exclude sl-lint --exclude sl-test-example  --all-features --all-targets --show-missing-lines` inside) and prints:
  - total line coverage percent
  - uncovered line count + merged ranges per file (for example `1-2,7-9`)
- `make gate`: runs `check + fmt + lint + test + coverage`.

## Compile Then Run (Recommended)

ScriptLang now supports a clear two-step host flow:

1. Compile source files (`*.xml`) into `CompiledProjectArtifact`.
2. Run or resume engine from that artifact.

Recommended API entry points are in `sl-api`:
- compile: `compile_artifact_from_xml_map`
- run: `create_engine_from_artifact`
- resume: `resume_engine_from_artifact`

If you need artifact file persistence, use `sl-compiler` helpers:
- `write_artifact_json(path, &artifact)`
- `read_artifact_json(path)`

`create_engine_from_xml` is still available as a compatibility convenience path, but it internally does `compile -> artifact -> run`.

## CLI Quick Start

```bash
cargo run -p sl-cli -- --help
cargo run -p sl-cli -- agent --help
cargo run -p sl-cli -- tui --help
```

For command details, machine output schema, and replay examples, use:
[docs/sl-cli-usage.md](docs/sl-cli-usage.md).

## Examples
Rhai-authored smoke scenarios live in `crates/sl-test-example/examples`.
Each example directory also carries a `testcase.json` consumed by `sl-test-example`.
