# scriptlang-rs

Rust workspace implementation of ScriptLang (Phase 1), with Rhai as the embedded script engine.

## Documentation
- [SL Engine API usage](docs/sl-engine-api.md)
- [SL CLI usage](docs/sl-cli-usage.md)
- [ScriptLang syntax rules](docs/scriptlang-syntax.md)
- [Example testing with sl-test-example runner](docs/testing-examples.md)
- [Rust testability playbook for 100% coverage](docs/rust-testability-playbook.md)

## Workspace Crates
- `crates/sl-core`: shared types, values, errors, snapshot/player schemas.
- `crates/sl-parser`: XML parser + include directive extraction.
- `crates/sl-compiler`: include graph validation + defs/module/json/script compilation to compiled artifact.
- `crates/sl-runtime`: execution engine (`next/choose/submit_input/snapshot/resume`).
- `crates/sl-api`: high-level create/compile/resume API.
- `crates/sl-cli`: host-side CLI (`agent` and `tui` modes).
- `crates/sl-test-example`: example integration tests + in-crate testcase runner/assertion.

## Public Surface
- Stable/recommended user-facing entry points:
  - `sl-api` (Rust host integration)
  - `sl-cli` (command-line host tooling)
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
  `include_graph.rs`, `defs_resolver.rs`, `type_expr.rs`, `json_symbols.rs`,
  `sanitize.rs`, `script_compile.rs`, `xml_utils.rs`, `macro_expand.rs`, `defaults.rs`.

This split keeps crate boundaries unchanged and enforces one-way internal dependencies.

## Testability Requirements (IMPORTANT)
All code must be written with testability in mind:
- **One-to-one test file mapping**: Each source file should have a corresponding test module in the same file (`#[cfg(test)] mod tests { ... }`).
- **Test order**: Test functions must be defined in the same order as the functions they test within each file.
- **No compatibility burden in this phase**: This is a development stage; do not introduce extra version compatibility handling unless explicitly required.
- **100% coverage required**: All code paths must be covered by tests; `make gate` enforces this.
- **Write tests first**: When fixing bugs or adding features, write the failing test first (TDD approach).
- **Test support helpers**: Use the `*_test_support` modules provided by each crate for common test utilities.

## Runtime/Compiler Performance Notes
- `sl-runtime` reuses a single internal `rhai::Engine` instance and keeps `random` builtin state in shared runtime storage, avoiding per-eval engine re-construction.
- defs prelude generation is cached per script in runtime, so repeated expression/code evaluation does not rebuild identical prelude text.
- parser/compiler/runtime regex usage for stable patterns is lazily initialized once via static caches.
- `sl-compiler` memoizes per-script reachable include closures during project compilation to avoid repeated DFS work.
- include supports both single files and directory expansion via `<!-- include: dir/ -->`; directory expansion is recursive and uses stable path-sorted ordering.

## Modules And Defs
- `*.module.xml` uses `<module name="...">` and may contain `<type>`, `<function>`, `<var>`, and multiple `<script>` nodes.
- Module scripts compile to qualified names like `battle.main`; host entry/call/return targets should use that qualified form.
- Inside the same module, scripts may reference sibling scripts with short names (`<call script="next"/>`), which compiles to the qualified module target.
- Module `<var>` behaves like existing defs globals: writable, snapshotted, and visible through include-closure rules.
- `*.defs.xml` remains supported as a compatibility form of “module without `<script>`”.

## Defs Globals (`<defs><var>` / `<module><var>`)
- `*.defs.xml` now supports `<var name="..." type="...">expr</var>` as writable globals.
- `*.module.xml` `<var>` shares the same runtime model and namespace-qualified access rules.
- globals initialize on `engine.start`, support short name and `ns.var` access, and follow include-closure visibility.
- when short names conflict across namespaces, only fully-qualified `ns.var` remains available.

## Choice Dynamic Options
- `<choice>` supports mixed static `<option>` and `<dynamic-options>` blocks.
- `<dynamic-options array=\"...\" item=\"...\" index=\"...\">` must contain exactly one template `<option>`.
- Template `<option>` supports `text` and `when`; `once` and `fall_over` are rejected.
- Expanded dynamic items keep source order with neighboring static options.

## Text Tag Passthrough
- `<text>` supports optional `tag` attribute as host metadata.
- Runtime and API expose it via `EngineOutput::Text { text, tag }`.
- CLI machine output keeps `TEXT_JSON` and emits optional `TEXT_TAG_JSON`.

## Debug Node
- `<debug>...</debug>` supports `${expr}` interpolation and emits `EngineOutput::Debug { text }`.
- `<debug>` does not support attributes (`once/tag` etc. are rejected at compile time).
- CLI hides debug events by default; pass `--show-debug` to emit `DEBUG_JSON` / `DEBUG: ...`.

## Dynamic Call/Return Targets
- `<call script="...">` and `<return script="...">` accept `${expr}` interpolation in `script`.
- Existing static names like `script="battle"` remain unchanged.
- Module-qualified static names such as `script="battle.main"` are first-class, and module-local short names resolve against the current module when possible.
- Target resolution happens at runtime and must resolve to a non-empty compiled script name.

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

## Compile Then Run (Recommended)

ScriptLang now supports a clear two-step host flow:

1. Compile source files (`*.script.xml` / `*.defs.xml` / `*.module.xml` / `*.json`) into `CompiledProjectArtifact`.
2. Run or resume engine from that artifact.

Recommended API entry points are in `sl-api`:
- compile: `compile_artifact_from_xml_map`
- run: `create_engine_from_artifact`
- resume: `resume_engine_from_artifact`

If you need artifact file persistence, use `sl-compiler` helpers:
- `write_artifact_json(path, &artifact)`
- `read_artifact_json(path)`

`create_engine_from_xml` is still available as a compatibility convenience path, but it internally does `compile -> artifact -> run`.

## CLI Usage

详细 CLI 文档见 [docs/sl-cli-usage.md](docs/sl-cli-usage.md)。

### Quick Start

```bash
cargo run -p sl-cli -- --help
cargo run -p sl-cli -- agent --help
cargo run -p sl-cli -- tui --help
cargo run -p sl-cli -- agent start --scripts-dir crates/sl-test-example/examples/06-snapshot-flow --state-out /tmp/sl-state.json
cargo run -p sl-cli -- agent start --scripts-dir crates/sl-test-example/examples/06-snapshot-flow --state-out /tmp/sl-state.json --rand "12,3,1,4"
cargo run -p sl-cli -- agent choose --state-in /tmp/sl-state.json --choice 0 --state-out /tmp/sl-next.json
cargo run -p sl-cli -- agent input --state-in /tmp/sl-next.json --text "Rin" --state-out /tmp/sl-next2.json
cargo run -p sl-cli -- agent replay --scripts-dir crates/sl-test-example/examples/16-input-name --step input:Rin
cargo run -p sl-cli -- agent replay --scripts-dir crates/sl-test-example/examples/16-input-name --step input:Rin --rand "12,3,1,4"
cargo run -p sl-cli -- agent replay --scripts-dir crates/sl-test-example/examples/16-input-name --step input:Rin --step choose:0
cargo run -p sl-cli -- agent compile --scripts-dir crates/sl-test-example/examples/01-text-code --dry-run
cargo run -p sl-cli -- agent compile --scripts-dir crates/sl-test-example/examples/01-text-code -o /tmp/artifact.json
cargo run -p sl-cli -- tui --scripts-dir crates/sl-test-example/examples/06-snapshot-flow
```

更多参数、输出协议、回放语法和完整流程说明，请查看 [docs/sl-cli-usage.md](docs/sl-cli-usage.md)。

## Examples
Rhai-authored smoke scenarios live in `crates/sl-test-example/examples`.
Each example directory also carries a `testcase.json` consumed by `sl-test-example`.

## User Pitfalls And Guardrails
- XML attribute escaping is mandatory: use `&lt;` for `<`, `&amp;&amp;` for `&&`.
- Type visibility is per include-closure: scripts under `actions/locations/events` must each include needed `*.defs.xml`, or include a directory that expands to those defs for that script.
- In `when="..."`, prefer string literals as `&quot;...&quot;` instead of single quotes.
- `Data type incorrect: f64 (expecting i64)` in array indexing is treated as a runtime type-stability bug; prioritize runtime fix/upgrade over user-side workarounds.
- Validation should be `compile --dry-run` + `replay --rand "<fixed-seq>"` together; compile-only is not enough for runtime-path safety.
