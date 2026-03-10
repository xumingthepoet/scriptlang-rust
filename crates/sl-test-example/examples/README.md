# Example Catalog (`sl-test-example/examples`)

This catalog keeps example intent explicit so examples can be refactored without losing coverage.

## Curation Rules
- Keep one primary scenario per example, but prefer realistic flow over trivial snippets.
- Preserve or increase test points when refactoring examples.
- Use stable deterministic outputs for testcase assertions.
- Keep numbering unique and ordered.

## Coverage Matrix
| Example | Primary Coverage |
| --- | --- |
| `01-text-code` | `temp` + `text` + `code` baseline flow |
| `02-if-while` | `if/else` and `while` execution |
| `03-choice-loop` | repeated `choice` inside loop with actions |
| `04-call-ref-return` | `call` with `ref` arg and write-back |
| `05-return-transfer` | `return script=... args=...` transfer |
| `06-snapshot-flow` | choice boundary flow suitable for snapshot/resume |
| `07-battle-duel` | multi-module combined scenario |
| `08-json-globals` | JSON global data + branching |
| `09-random` | deterministic `random(...)` rendering |
| `10-once-static` | `once` static option behavior |
| `11-choice-fallover-continue` | `fall_over` and `continue` in choice |
| `12-while-break-continue` | loop control (`break`/`continue`) |
| `13-loop-times` | `loop times=...` sugar |
| `14-module-functions` | module `type/function` and cross-module invoke |
| `15-entry-override-recursive` | entry override and recursive imports |
| `16-input-name` | multiple `input` boundaries and defaults |
| `17-module-global-shadowing` | module global vs local shadowing |
| `18-group-container` | `<group>` variable scope isolation |
| `19-dynamic-choice-mixed` | static + dynamic options mixed |
| `20-dynamic-choice-nested` | nested dynamic choices |
| `21-directory-import` | directory import resolution |
| `22-access-control` | `public/private` access boundaries |
| `23-const-basics` | const declarations and const init dependency |
| `24-invoke-dynamic` | dynamic `invoke(name, args)` from data |
| `25-function-invoke` | function-call + invoke composition |
| `26-enum-flow` | `<enum>` + `Type.Member` in attribute args |
| `27-dynamic-transfer-target` | dynamic script target for call/return |
| `28-map-coverage` | map `#{K=>V}` + `#{V}` usage (string key + enum key + nested/array/ref/function/when) |

## Notes
- `26-enum-flow` intentionally covers enum member usage directly in XML attribute expressions (`args="ids.LocationId.A"`).
- `27-dynamic-transfer-target` was renumbered to avoid duplicate `21-*` confusion.
