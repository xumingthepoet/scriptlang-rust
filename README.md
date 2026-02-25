# scriptlang-rs

Rust workspace implementation of ScriptLang (Phase 1), with Rhai as embedded script engine.

## Workspace Crates
- `crates/sl-core`: shared types, values, errors, snapshot/player schemas.
- `crates/sl-parser`: XML parser + include directive extraction.
- `crates/sl-compiler`: include graph validation + defs/json/script compilation to IR.
- `crates/sl-runtime`: execution engine (`next/choose/submit_input/snapshot/resume`).
- `crates/sl-api`: high-level create/compile/resume API.
- `crates/sl-cli`: `agent start/choose/input` command-line interface.

## Commands
- `cargo qk`: workspace check.
- `cargo qt`: workspace tests.
- `cargo qc`: workspace clippy (`-D warnings`).
- `make gate`: fmt + clippy + test.

## Agent CLI
```bash
cargo run -p sl-cli -- agent start --scripts-dir examples/scripts-rhai/06-snapshot-flow --state-out /tmp/sl-state.json
cargo run -p sl-cli -- agent choose --state-in /tmp/sl-state.json --choice 0 --state-out /tmp/sl-next.json
cargo run -p sl-cli -- agent input --state-in /tmp/sl-next.json --text "Rin" --state-out /tmp/sl-next2.json
```

## Examples
Rhai-authored smoke scenarios live in `examples/scripts-rhai`.
