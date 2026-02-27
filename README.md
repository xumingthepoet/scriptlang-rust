# scriptlang-rs

Rust workspace implementation of ScriptLang (Phase 1), with Rhai as the embedded script engine.

## Documentation
- [SL Engine API usage](docs/sl-engine-api.md)
- [ScriptLang syntax rules](docs/scriptlang-syntax.md)
- [Example testing with sl-tool](docs/testing-examples.md)
- [Rust testability playbook for 100% coverage](docs/rust-testability-playbook.md)

## Workspace Crates
- `crates/sl-core`: shared types, values, errors, snapshot/player schemas.
- `crates/sl-parser`: XML parser + include directive extraction.
- `crates/sl-compiler`: include graph validation + defs/json/script compilation to IR.
- `crates/sl-runtime`: execution engine (`next/choose/submit_input/snapshot/resume`).
- `crates/sl-api`: high-level create/compile/resume API.
- `crates/sl-cli`: host-side CLI (`agent` and `tui` modes).
- `crates/sl-tool`: reusable testing toolkit (`testcase.json` schema + case runner/assertion).
- `crates/sl-test-example`: example integration tests using `sl-tool`.

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
  compile pipeline is split into `context.rs`, `pipeline.rs`, `source_parse.rs`,
  `include_graph.rs`, `defs_resolver.rs`, `type_expr.rs`, `json_symbols.rs`,
  `sanitize.rs`, `script_compile.rs`, `xml_utils.rs`, `macro_expand.rs`, `defaults.rs`.

This split keeps crate boundaries unchanged and enforces one-way internal dependencies.

## Testability Requirements (IMPORTANT)
All code must be written with testability in mind:
- **One-to-one test file mapping**: Each source file should have a corresponding test module in the same file (`#[cfg(test)] mod tests { ... }`).
- **Test order**: Test functions must be defined in the same order as the functions they test within each file.
- **100% coverage required**: All code paths must be covered by tests; `make gate` enforces this.
- **Write tests first**: When fixing bugs or adding features, write the failing test first (TDD approach).
- **Test support helpers**: Use the `*_test_support` modules provided by each crate for common test utilities.

## Runtime/Compiler Performance Notes
- `sl-runtime` reuses a single internal `rhai::Engine` instance and keeps `random` builtin state in shared runtime storage, avoiding per-eval engine re-construction.
- defs prelude generation is cached per script in runtime, so repeated expression/code evaluation does not rebuild identical prelude text.
- parser/compiler/runtime regex usage for stable patterns is lazily initialized once via static caches.
- `sl-compiler` memoizes per-script reachable include closures during project compilation to avoid repeated DFS work.

## Defs Globals (`<defs><var>`)
- `*.defs.xml` now supports `<var name="..." type="...">expr</var>` as writable globals.
- globals initialize on `engine.start`, support short name and `ns.var` access, and follow include-closure visibility.
- when short names conflict across namespaces, only fully-qualified `ns.var` remains available.

## Commands
- `make check`: `cargo check --workspace --all-targets --all-features`
- `make fmt`: `cargo fmt --all -- --check`
- `make lint`: `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `make test`: `cargo test --workspace --all-targets --all-features` with `LLVM_PROFILE_FILE` unset
- `cargo test -p sl-test-example --all-targets --all-features`: run example cases from `crates/sl-test-example/examples/*/testcase.json`
- `make coverage`: runs `scripts/coverage.sh` (uses `cargo llvm-cov --workspace --exclude sl-cli --exclude sl-test-example  --all-features --all-targets --show-missing-lines` inside) and prints:
  - total line coverage percent
  - uncovered line count + merged ranges per file (for example `1-2,7-9`)
- `make gate`: runs `check + fmt + lint + test + coverage`.

## CLI Usage

### Agent mode
```bash
cargo run -p sl-cli -- agent start --scripts-dir crates/sl-test-example/examples/06-snapshot-flow --state-out /tmp/sl-state.json
cargo run -p sl-cli -- agent choose --state-in /tmp/sl-state.json --choice 0 --state-out /tmp/sl-next.json
cargo run -p sl-cli -- agent input --state-in /tmp/sl-next.json --text "Rin" --state-out /tmp/sl-next2.json
```

`agent` mode prints line-based machine-readable output:
- `RESULT:OK|ERROR`
- `EVENT:CHOICES|INPUT|END`
- `TEXT_JSON:...`
- `PROMPT_JSON:...`
- `CHOICE:<index>|<json_text>`
- `INPUT_DEFAULT_JSON:...`
- `STATE_OUT:<path|NONE>`
- `ERROR_CODE:...` (only when `RESULT:ERROR`)
- `ERROR_MSG_JSON:...` (only when `RESULT:ERROR`)

### TUI mode
```bash
cargo run -p sl-cli -- tui --scripts-dir crates/sl-test-example/examples/06-snapshot-flow
```

`tui` mode uses a `ratatui + crossterm` full-screen interface on real terminals:
- `Up/Down` to select choices
- typing + `Backspace` to edit input text
- `Enter` to submit choice/input
- `s` save snapshot, `l` load snapshot, `r` restart, `h` help, `q`/`Esc` quit

When stdin/stdout is not a TTY (for example, piped in tests), it automatically falls back to the previous line-based interactive mode.
When running under Rust test harness (unit-test build or `RUST_TEST_THREADS` is present), it also forces line mode to avoid entering full-screen TUI in tests.
Fallback line mode supports command input:
- `:help`
- `:save`
- `:load`
- `:restart`
- `:quit`

All example entry commands:
```bash
cargo run -p sl-cli -- tui --scripts-dir crates/sl-test-example/examples/01-text-code
cargo run -p sl-cli -- tui --scripts-dir crates/sl-test-example/examples/02-if-while
cargo run -p sl-cli -- tui --scripts-dir crates/sl-test-example/examples/03-choice-once
cargo run -p sl-cli -- tui --scripts-dir crates/sl-test-example/examples/04-call-ref-return
cargo run -p sl-cli -- tui --scripts-dir crates/sl-test-example/examples/05-return-transfer
cargo run -p sl-cli -- tui --scripts-dir crates/sl-test-example/examples/06-snapshot-flow
cargo run -p sl-cli -- tui --scripts-dir crates/sl-test-example/examples/07-battle-duel
cargo run -p sl-cli -- tui --scripts-dir crates/sl-test-example/examples/08-json-globals
cargo run -p sl-cli -- tui --scripts-dir crates/sl-test-example/examples/09-random
cargo run -p sl-cli -- tui --scripts-dir crates/sl-test-example/examples/10-once-static
cargo run -p sl-cli -- tui --scripts-dir crates/sl-test-example/examples/11-choice-fallover-continue
cargo run -p sl-cli -- tui --scripts-dir crates/sl-test-example/examples/12-while-break-continue
cargo run -p sl-cli -- tui --scripts-dir crates/sl-test-example/examples/13-loop-times
cargo run -p sl-cli -- tui --scripts-dir crates/sl-test-example/examples/14-defs-functions
cargo run -p sl-cli -- tui --scripts-dir crates/sl-test-example/examples/15-entry-override-recursive
cargo run -p sl-cli -- tui --scripts-dir crates/sl-test-example/examples/16-input-name
cargo run -p sl-cli -- tui --scripts-dir crates/sl-test-example/examples/17-defs-global-shadowing
cargo run -p sl-cli -- tui --scripts-dir crates/sl-test-example/examples/18-group-container
```

You can override defaults with:
- `--entry-script <name>` (default: `main`)
- `--state-file <path>` (default: `.scriptlang/save.json`)

## Examples
Rhai-authored smoke scenarios live in `crates/sl-test-example/examples`.
Each example directory also carries a `testcase.json` consumed by `sl-tool`/`sl-test-example`.
