# Example Testing with `sl-tool`

This workspace uses `sl-tool` + `sl-test-example` to validate all examples under `crates/sl-test-example/examples`.

## Layout
- Case file path: `crates/sl-test-example/examples/<example>/testcase.json`
- Runner API: `sl_tool::run_case(...)` and `sl_tool::assert_case(...)`
- Integration tests: `crates/sl-test-example/tests/*_it.rs`

## Case Schema (`sl-tool-case.v1`)

```json
{
  "schemaVersion": "sl-tool-case.v1",
  "entryScript": "main",
  "actions": [
    { "kind": "choose", "index": 0 },
    { "kind": "input", "text": "Guild" }
  ],
  "expectedEvents": [
    { "kind": "text", "text": "before 10" },
    { "kind": "choices", "promptText": "Choose", "choices": ["Heal", "Hit"] },
    { "kind": "input", "promptText": "Name", "defaultText": "Traveler" },
    { "kind": "end" }
  ]
}
```

## Action Rules
- `choose`: uses visible option index at runtime.
- `input`: submits text to pending input boundary.
- Actions are consumed in order.
- If a choice/input boundary appears without a matching action, test fails.
- If actions remain after `end`, test fails.

## Event Assertion Rules
- Full event stream order is compared exactly.
- `text`: compares rendered text.
- `choices`: compares `promptText` and ordered choice text list.
- `input`: compares `promptText` and `defaultText`.
- `end`: terminal event.

## Runtime Defaults
- `entryScript`: defaults to `main` if omitted.
- Random seed: fixed to `1` inside `sl-tool` runner for deterministic outputs.
- Guard: max `5000` engine steps per case.

## Commands
- Run only example cases:
  - `cargo test -p sl-test-example --all-targets --all-features`
- Run full quality gate:
  - `make gate`

## Typical Failure Causes
- Wrong choice index due changed option visibility/order.
- Text drift after script logic updates.
- Changed choice/input prompt/default text.
- Action count not matching runtime boundary count.
