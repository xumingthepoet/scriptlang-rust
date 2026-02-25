# scriptlang-rs TASKLIST

## Done
- [x] M1 workspace and crate boundaries.
- [x] M2 parser/compiler core (XML + include graph + IR + loop macro).
- [x] M3 runtime core (Rhai execution + control flow + snapshot.v3).
- [x] M4 high-level API (`compile/create/resume`).
- [x] M5 agent CLI (`start/choose/input`, state load/save, scripts-dir scanning).
- [x] M6 Rhai examples baseline and smoke test (`examples/scripts-rhai`).
- [x] Convert `script-lang` into git submodule.

## In Progress
- [ ] Wire host function calls from Rhai runtime via `HostFunctionRegistry::call`.
- [ ] Add tagged portable state codec for Map/non-finite numbers.
- [ ] Add strict 100% coverage tooling and complete path-level tests.

## Risks
- Rhai semantics differ from JS in some expression/object edge cases.
- Strict compatibility with historical TS runtime error wording is intentionally relaxed.
