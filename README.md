# scriptlang-rs

Rust workspace implementation of ScriptLang (Phase 1), with Rhai as the embedded script engine.

## Documentation
- [ScriptLang syntax rules](docs/scriptlang-syntax.md)

## Workspace Crates
- `crates/sl-core`: shared types, values, errors, snapshot/player schemas.
- `crates/sl-parser`: XML parser + include directive extraction.
- `crates/sl-compiler`: include graph validation + defs/json/script compilation to IR.
- `crates/sl-runtime`: execution engine (`next/choose/submit_input/snapshot/resume`).
- `crates/sl-api`: high-level create/compile/resume API.
- `crates/sl-cli`: host-side CLI (`agent` and `tui` modes).

## Commands
- `cargo qk`: `cargo check --workspace --all-targets --all-features`
- `cargo qa`: `cargo fmt --all -- --check`
- `cargo qc`: `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo qt`: `cargo test --workspace --all-targets --all-features`
- `cargo tarpaulin --engine llvm --workspace --all-features --all-targets --rustflags=--cfg=coverage --out Stdout --fail-under 100`: coverage gate.
- `make gate`: `check + fmt + clippy + test + coverage`.

## CLI Usage

### Agent mode
```bash
cargo run -p sl-cli -- agent start --scripts-dir examples/scripts-rhai/06-snapshot-flow --state-out /tmp/sl-state.json
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
cargo run -p sl-cli -- tui --scripts-dir examples/scripts-rhai/06-snapshot-flow
```

`tui` mode uses a `ratatui + crossterm` full-screen interface on real terminals:
- `Up/Down` to select choices
- typing + `Backspace` to edit input text
- `Enter` to submit choice/input
- `s` save snapshot, `l` load snapshot, `r` restart, `h` help, `q`/`Esc` quit

When stdin/stdout is not a TTY (for example, piped in tests), it automatically falls back to the previous line-based interactive mode.
When running under Rust test harness (`RUST_TEST_THREADS` is present), it also forces line mode to avoid entering full-screen TUI in tests.
Fallback line mode supports command input:
- `:help`
- `:save`
- `:load`
- `:restart`
- `:quit`

All example entry commands:
```bash
cargo run -p sl-cli -- tui --scripts-dir examples/scripts-rhai/01-text-code
cargo run -p sl-cli -- tui --scripts-dir examples/scripts-rhai/02-if-while
cargo run -p sl-cli -- tui --scripts-dir examples/scripts-rhai/03-choice-once
cargo run -p sl-cli -- tui --scripts-dir examples/scripts-rhai/04-call-ref-return
cargo run -p sl-cli -- tui --scripts-dir examples/scripts-rhai/05-return-transfer
cargo run -p sl-cli -- tui --scripts-dir examples/scripts-rhai/06-snapshot-flow
cargo run -p sl-cli -- tui --scripts-dir examples/scripts-rhai/07-battle-duel
cargo run -p sl-cli -- tui --scripts-dir examples/scripts-rhai/08-json-globals
cargo run -p sl-cli -- tui --scripts-dir examples/scripts-rhai/09-random
cargo run -p sl-cli -- tui --scripts-dir examples/scripts-rhai/10-once-static
cargo run -p sl-cli -- tui --scripts-dir examples/scripts-rhai/11-choice-fallover-continue
cargo run -p sl-cli -- tui --scripts-dir examples/scripts-rhai/12-while-break-continue
cargo run -p sl-cli -- tui --scripts-dir examples/scripts-rhai/13-loop-times
cargo run -p sl-cli -- tui --scripts-dir examples/scripts-rhai/14-defs-functions
cargo run -p sl-cli -- tui --scripts-dir examples/scripts-rhai/15-entry-override-recursive
cargo run -p sl-cli -- tui --scripts-dir examples/scripts-rhai/16-input-name
```

You can override defaults with:
- `--entry-script <name>` (default: `main`)
- `--state-file <path>` (default: `.scriptlang/save.json`)

## Examples
Rhai-authored smoke scenarios live in `examples/scripts-rhai`.
