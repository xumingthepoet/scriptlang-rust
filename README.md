# scriptlang-rs

Rust workspace implementation of ScriptLang, with Rhai as the embedded script engine.

## Documentation
- [Architecture overview](ARCHITECTURE.md): workspace layers, dependency direction, crate boundaries, and compile/run flow.
- [SL Engine API usage](docs/sl-engine-api.md): host-side Rust API and runtime integration.
- [SL CLI usage](docs/sl-cli-usage.md): `agent`/`tui` commands, machine output, replay workflow.
- [SL Lint usage](docs/sl-lint-usage.md): standalone lint checks and output contract.
- [ScriptLang syntax rules](docs/scriptlang-syntax.md): XML grammar and language semantics.
- [Example testing with sl-test-example runner](docs/testing-examples.md): example-case contract and runner usage.
- [Rust testability playbook for high coverage](docs/rust-testability-playbook.md): testing patterns and coverage tactics.

Architecture details (crate responsibilities, dependency direction, public surface, and compile/run data flow) are maintained in [ARCHITECTURE.md](ARCHITECTURE.md).

## Testability Requirements (IMPORTANT)
All code must be written with testability in mind:
- **One-to-one test file mapping**: Each source file should have a corresponding test module in the same file (`#[cfg(test)] mod tests { ... }`).
- **Test order**: Test functions must be defined in the same order as the functions they test within each file.
- **No backward-compat burden by default**: Do not introduce extra version compatibility handling unless explicitly required.
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
- `make coverage`: runs `scripts/coverage.sh` (uses `cargo llvm-cov --workspace --exclude sl-cli --exclude sl-lint --exclude sl-test-example  --all-features --all-targets --show-missing-lines` inside) and prints:
  - total line coverage percent
  - uncovered line count + merged ranges per file (for example `1-2,7-9`)
- `make gate`: runs `check + fmt + lint + test + coverage`.

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
