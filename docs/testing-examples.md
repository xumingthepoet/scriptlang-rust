# Example Testing with `sl-test-example`

This workspace uses `sl-test-example` to validate all examples under `crates/sl-test-example/examples`.

## Layout
- Case file path: `crates/sl-test-example/examples/<example>/testcase.json`
- Example catalog: `crates/sl-test-example/examples/README.md` (coverage matrix and curation rules)
- Runner API: `sl_test_example::run_case(...)` and `sl_test_example::assert_case(...)`
- Integration tests: `crates/sl-test-example/tests/all_examples_it.rs` (single test binary to reduce per-process startup overhead)

## Curation Baseline
- Keep numbering unique and ordered.
- Keep one primary scenario per example, but avoid toy-only scripts when a realistic flow is possible.
- Refactors must preserve or increase test points (no silent coverage drop).
- Prefer deterministic outputs so `expectedEvents` stays stable.

## Case Schema (`sl-tool-case`)

```json
{
  "schemaVersion": "sl-tool-case",
  "entryScript": "main.main",
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
- `entryScript`: defaults to `main.main` if omitted.
- 运行链路：`xml -> compile artifact -> create engine from artifact`
- Random seed: fixed to `1` inside `sl-test-example` runner for deterministic outputs.
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
