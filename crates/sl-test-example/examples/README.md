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
| `05-return-transfer` | `goto script=... args=...` transfer |
| `06-snapshot-flow` | choice boundary flow suitable for snapshot/resume |
| `07-battle-duel` | multi-module combined scenario |
| `08-json-globals` | JSON global data + branching |
| `09-random` | deterministic `random(...)` rendering |
| `10-once-static` | `once` static option behavior |
| `11-choice-fallover-continue` | `fall_over` and `continue` in choice |
| `12-while-break-continue` | loop control (`break`/`continue`) |
| `13-for-macro` | `for temps/condition/iteration` macro with `continue`/`break` |
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
| `24-invoke-dynamic` | dynamic `invoke(fnVar, args)` via `function` reference |
| `25-function-invoke` | function-call + invoke composition |
| `26-enum-flow` | `<enum>` + `Type.Member` in attribute args |
| `27-dynamic-transfer-target` | dynamic script target for goto (plus call chain) |
| `28-map-coverage` | map `#{K=>V}` + `#{V}` usage (string key + enum key + nested/array/ref/function/when) |
| `29-alias-directives` | multi-module alias stress: type/var/const alias across arrays/maps/const/function/multi-script call chains |
| `30-invoke-function-alias-module-var` | regression for `invoke(function, ...)` where function body reads explicit alias to imported module var |
| `31-invoke-map-short-function-ref` | regression for short `*function` refs forwarded via enum-key map and function-body direct forwarding before `invoke(...)` |
| `32-temp-input-macro` | `temp-input` macro for `string temp + input` with blank fallback and max_length |
| `33-script-context-macro` | compile-time `__script__` context macro in expression/template across scripts |
| `34-invoke-private-capability` | capability semantics: private `function` reference can be invoked after legal cross-module forwarding |
| `35-script-private-capability` | capability semantics: private `script` reference can be forwarded and executed only through allowed capability flow |
| `36-terminal-structure-check` | compile-time terminal-structure validation for script tail `if/else` branches |
| `37-lint-function-script-literal` | lint regression: alias-only import usage + function-body `@script` literal tracking |
| `38-invalid-qualified-enum-name` | compile-time declaration-name validation: `<enum name>` must be short identifier, not qualified path |
| `39-duplicate-import` | compile-time validation: duplicate `import` target in one module is rejected |
| `40-duplicate-alias` | compile-time validation: duplicate `alias` directive in one module is rejected |
| `41-nested-module-visibility` | nested module flatten + same-root sibling visibility (`b` can read `c` exported symbols) |
| `42-nested-module-root-gate-deny` | root `export module:*` gate blocks external access to non-exported submodule |
| `43-nested-module-internal-descendant-visibility-deny` | same-root initializer cannot access `c.d.*` when `c` does not export `module:d` |
| `44-xml-initializer-format-combo` | `format=\"xml\"` structured init for `var/const/temp` across object/array/map/enum-map with inline coexistence |
| `45-xml-initializer-mixed-content-deny` | compile-time rejection when `format=\"xml\"` mixes non-empty text and structural child nodes |

## Notes
- `26-enum-flow` intentionally covers enum member usage directly in XML attribute expressions (`args="ids.LocationId.A"`).
- `27-dynamic-transfer-target` was renumbered to avoid duplicate `21-*` confusion.
