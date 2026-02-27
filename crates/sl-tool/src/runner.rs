use std::path::Path;

use sl_api::{create_engine_from_xml, CreateEngineFromXmlOptions};
use sl_runtime::DEFAULT_COMPILER_VERSION;

use crate::source::{read_scripts_xml_from_dir, read_test_case};
use crate::{ExpectedEvent, SlToolError, TestAction, TestCase};

const MAX_STEPS: usize = 5_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunReport {
    pub observed_events: Vec<ExpectedEvent>,
    pub consumed_actions: usize,
    pub steps: usize,
}

pub fn run_case(example_dir: &Path, case: &TestCase) -> Result<RunReport, SlToolError> {
    let scripts_xml = read_scripts_xml_from_dir(example_dir)?;
    let mut engine = create_engine_from_xml(CreateEngineFromXmlOptions {
        scripts_xml,
        entry_script: Some(case.entry_script.clone()),
        entry_args: None,
        host_functions: None,
        random_seed: Some(1),
        compiler_version: Some(DEFAULT_COMPILER_VERSION.to_string()),
    })
    .map_err(SlToolError::Engine)?;

    let mut observed_events = Vec::new();
    let mut action_index = 0usize;

    for step in 1..=MAX_STEPS {
        let output = engine.next_output().map_err(SlToolError::Engine)?;
        match output {
            sl_core::EngineOutput::Text { text } => {
                observed_events.push(ExpectedEvent::Text { text });
            }
            sl_core::EngineOutput::Choices { items, prompt_text } => {
                let choices = items.into_iter().map(|item| item.text).collect();
                observed_events.push(ExpectedEvent::Choices {
                    prompt_text,
                    choices,
                });
                let event_index = observed_events.len() - 1;
                let action =
                    case.actions
                        .get(action_index)
                        .ok_or_else(|| SlToolError::MissingAction {
                            event_index,
                            expected_action_kind: "choose".to_string(),
                        })?;
                match action {
                    TestAction::Choose { index } => {
                        engine.choose(*index).map_err(SlToolError::Engine)?;
                    }
                    _ => {
                        return Err(SlToolError::ActionKindMismatch {
                            event_index,
                            expected_action_kind: "choose".to_string(),
                            actual_action_kind: action.kind_name().to_string(),
                        })
                    }
                }
                action_index += 1;
            }
            sl_core::EngineOutput::Input {
                prompt_text,
                default_text,
            } => {
                observed_events.push(ExpectedEvent::Input {
                    prompt_text,
                    default_text,
                });
                let event_index = observed_events.len() - 1;
                let action =
                    case.actions
                        .get(action_index)
                        .ok_or_else(|| SlToolError::MissingAction {
                            event_index,
                            expected_action_kind: "input".to_string(),
                        })?;
                match action {
                    TestAction::Input { text } => {
                        engine.submit_input(text).map_err(SlToolError::Engine)?;
                    }
                    _ => {
                        return Err(SlToolError::ActionKindMismatch {
                            event_index,
                            expected_action_kind: "input".to_string(),
                            actual_action_kind: action.kind_name().to_string(),
                        })
                    }
                }
                action_index += 1;
            }
            sl_core::EngineOutput::End => {
                observed_events.push(ExpectedEvent::End);
                if action_index != case.actions.len() {
                    return Err(SlToolError::UnusedActions {
                        used: action_index,
                        total: case.actions.len(),
                    });
                }
                return Ok(RunReport {
                    observed_events,
                    consumed_actions: action_index,
                    steps: step,
                });
            }
        }
    }

    Err(SlToolError::GuardExceeded {
        max_steps: MAX_STEPS,
    })
}

pub fn assert_case(example_dir: &Path, case_path: &Path) -> Result<(), SlToolError> {
    let case = read_test_case(case_path)?;
    let report = run_case(example_dir, &case)?;

    if report.observed_events.len() != case.expected_events.len() {
        let observed = serde_json::to_string_pretty(&report.observed_events)
            .map_err(SlToolError::EventSerialize)?;
        return Err(SlToolError::EventCountMismatch {
            expected: case.expected_events.len(),
            actual: report.observed_events.len(),
            observed,
        });
    }

    for (index, (expected, actual)) in case
        .expected_events
        .iter()
        .zip(report.observed_events.iter())
        .enumerate()
    {
        if expected != actual {
            let expected = serde_json::to_string(expected).map_err(SlToolError::EventSerialize)?;
            let actual = serde_json::to_string(actual).map_err(SlToolError::EventSerialize)?;
            return Err(SlToolError::EventMismatch {
                index,
                expected,
                actual,
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod runner_tests {
    use super::*;

    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should move forward")
            .as_nanos();
        std::env::temp_dir().join(format!("sl-tool-runner-{}-{}", name, nanos))
    }

    fn write_file(path: &Path, content: &str) {
        let parent = path.parent().expect("path should have parent");
        fs::create_dir_all(parent).expect("parent dir should be created");
        fs::write(path, content).expect("file should be written");
    }

    fn simple_case(expected_events: Vec<ExpectedEvent>) -> TestCase {
        TestCase {
            schema_version: crate::TESTCASE_SCHEMA_V1.to_string(),
            entry_script: "main".to_string(),
            actions: Vec::new(),
            expected_events,
        }
    }

    #[test]
    fn run_case_executes_text_only_script() {
        let root = temp_dir("text-only");
        write_file(
            &root.join("main.script.xml"),
            r#"<script name="main"><text>Hello</text></script>"#,
        );

        let case = simple_case(vec![
            ExpectedEvent::Text {
                text: "Hello".to_string(),
            },
            ExpectedEvent::End,
        ]);
        let report = run_case(&root, &case).expect("run should pass");

        assert_eq!(report.consumed_actions, 0);
        assert_eq!(report.observed_events, case.expected_events);
    }

    #[test]
    fn run_case_consumes_choose_and_input_actions() {
        let root = temp_dir("boundaries");
        write_file(
            &root.join("main.script.xml"),
            r#"
<script name="main">
  <choice text="Pick">
    <option text="A"><text>A</text></option>
  </choice>
  <var name="name" type="string">"Traveler"</var>
  <input var="name" text="Name"/>
  <text>${name}</text>
</script>
"#,
        );

        let case = TestCase {
            schema_version: crate::TESTCASE_SCHEMA_V1.to_string(),
            entry_script: "main".to_string(),
            actions: vec![
                TestAction::Choose { index: 0 },
                TestAction::Input {
                    text: "Guild".to_string(),
                },
            ],
            expected_events: vec![
                ExpectedEvent::Choices {
                    prompt_text: Some("Pick".to_string()),
                    choices: vec!["A".to_string()],
                },
                ExpectedEvent::Text {
                    text: "A".to_string(),
                },
                ExpectedEvent::Input {
                    prompt_text: "Name".to_string(),
                    default_text: "Traveler".to_string(),
                },
                ExpectedEvent::Text {
                    text: "Guild".to_string(),
                },
                ExpectedEvent::End,
            ],
        };

        let report = run_case(&root, &case).expect("run should pass");
        assert_eq!(report.consumed_actions, 2);
        assert_eq!(report.observed_events, case.expected_events);
    }

    #[test]
    fn run_case_reports_missing_or_wrong_action_kinds() {
        let root = temp_dir("missing-action");
        write_file(
            &root.join("main.script.xml"),
            r#"
<script name="main">
  <choice text="Pick">
    <option text="A"><text>A</text></option>
  </choice>
</script>
"#,
        );

        let missing = TestCase {
            schema_version: crate::TESTCASE_SCHEMA_V1.to_string(),
            entry_script: "main".to_string(),
            actions: vec![],
            expected_events: vec![],
        };
        let missing_error = run_case(&root, &missing).expect_err("missing action should fail");
        assert!(matches!(missing_error, SlToolError::MissingAction { .. }));

        let wrong_kind = TestCase {
            schema_version: crate::TESTCASE_SCHEMA_V1.to_string(),
            entry_script: "main".to_string(),
            actions: vec![TestAction::Input {
                text: "x".to_string(),
            }],
            expected_events: vec![],
        };
        let wrong_kind_error = run_case(&root, &wrong_kind).expect_err("kind mismatch should fail");
        assert!(matches!(
            wrong_kind_error,
            SlToolError::ActionKindMismatch { .. }
        ));

        let input_root = temp_dir("missing-input-action");
        write_file(
            &input_root.join("main.script.xml"),
            r#"
<script name="main">
  <var name="name" type="string">"Traveler"</var>
  <input var="name" text="Name"/>
</script>
"#,
        );

        let missing_input = TestCase {
            schema_version: crate::TESTCASE_SCHEMA_V1.to_string(),
            entry_script: "main".to_string(),
            actions: vec![],
            expected_events: vec![],
        };
        let missing_input_error =
            run_case(&input_root, &missing_input).expect_err("missing input action should fail");
        assert!(matches!(
            missing_input_error,
            SlToolError::MissingAction { .. }
        ));

        let wrong_input_kind = TestCase {
            schema_version: crate::TESTCASE_SCHEMA_V1.to_string(),
            entry_script: "main".to_string(),
            actions: vec![TestAction::Choose { index: 0 }],
            expected_events: vec![],
        };
        let wrong_input_kind_error = run_case(&input_root, &wrong_input_kind)
            .expect_err("wrong input action kind should fail");
        assert!(matches!(
            wrong_input_kind_error,
            SlToolError::ActionKindMismatch { .. }
        ));
    }

    #[test]
    fn run_case_reports_unused_actions_and_engine_errors() {
        let unused_root = temp_dir("unused-action");
        write_file(
            &unused_root.join("main.script.xml"),
            r#"<script name="main"><text>x</text></script>"#,
        );
        let unused_case = TestCase {
            schema_version: crate::TESTCASE_SCHEMA_V1.to_string(),
            entry_script: "main".to_string(),
            actions: vec![TestAction::Input {
                text: "x".to_string(),
            }],
            expected_events: vec![],
        };
        let unused_error =
            run_case(&unused_root, &unused_case).expect_err("unused action should fail");
        assert!(matches!(unused_error, SlToolError::UnusedActions { .. }));

        let bad_choose_root = temp_dir("bad-choose");
        write_file(
            &bad_choose_root.join("main.script.xml"),
            r#"
<script name="main">
  <choice text="Pick">
    <option text="A"><text>A</text></option>
  </choice>
</script>
"#,
        );
        let bad_choose_case = TestCase {
            schema_version: crate::TESTCASE_SCHEMA_V1.to_string(),
            entry_script: "main".to_string(),
            actions: vec![TestAction::Choose { index: 99 }],
            expected_events: vec![],
        };
        let bad_choose_error =
            run_case(&bad_choose_root, &bad_choose_case).expect_err("invalid choose should fail");
        assert!(matches!(bad_choose_error, SlToolError::Engine(_)));
    }

    #[test]
    fn run_case_reports_guard_exceeded() {
        let root = temp_dir("guard");
        write_file(
            &root.join("main.script.xml"),
            r#"
<script name="main">
  <var name="i" type="int">0</var>
  <while when="i &lt; 6000">
    <code>i = i + 1;</code>
    <text>tick</text>
  </while>
</script>
"#,
        );

        let case = simple_case(vec![]);
        let error = run_case(&root, &case).expect_err("guard should fail");
        assert!(matches!(error, SlToolError::GuardExceeded { .. }));
    }

    #[test]
    fn assert_case_reports_count_and_value_mismatches() {
        let root = temp_dir("assert");
        fs::create_dir_all(&root).expect("root should exist");
        write_file(
            &root.join("main.script.xml"),
            r#"<script name="main"><text>Hello</text></script>"#,
        );

        let count_case = root.join("count.json");
        write_file(
            &count_case,
            r#"{
  "schemaVersion":"sl-tool-case.v1",
  "actions":[],
  "expectedEvents":[{"kind":"end"}]
}"#,
        );
        let count_error = assert_case(&root, &count_case).expect_err("count mismatch should fail");
        assert!(matches!(
            count_error,
            SlToolError::EventCountMismatch { .. }
        ));

        let value_case = root.join("value.json");
        write_file(
            &value_case,
            r#"{
  "schemaVersion":"sl-tool-case.v1",
  "actions":[],
  "expectedEvents":[{"kind":"text","text":"Wrong"},{"kind":"end"}]
}"#,
        );
        let value_error = assert_case(&root, &value_case).expect_err("value mismatch should fail");
        assert!(matches!(value_error, SlToolError::EventMismatch { .. }));
    }

    #[test]
    fn assert_case_passes_with_matching_expected_events() {
        let root = temp_dir("assert-pass");
        fs::create_dir_all(&root).expect("root should exist");
        write_file(
            &root.join("main.script.xml"),
            r#"<script name="main"><text>Hello</text></script>"#,
        );

        let case_path = root.join("testcase.json");
        write_file(
            &case_path,
            r#"{
  "schemaVersion":"sl-tool-case.v1",
  "actions":[],
  "expectedEvents":[{"kind":"text","text":"Hello"},{"kind":"end"}]
}"#,
        );

        assert_case(&root, &case_path).expect("assert should pass");
    }
}
