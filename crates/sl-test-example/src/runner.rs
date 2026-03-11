use std::path::Path;

use sl_api::{
    compile_artifact_from_xml_map, create_engine_from_artifact, CreateEngineFromArtifactOptions,
};
use sl_runtime::DEFAULT_COMPILER_VERSION;

use crate::source::{read_scripts_xml_from_dir, read_test_case};
use crate::{ExpectedEvent, SlTestExampleError, TestAction, TestCase};

const MAX_STEPS: usize = 5_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunReport {
    pub observed_events: Vec<ExpectedEvent>,
    pub consumed_actions: usize,
    pub steps: usize,
}

pub fn run_case(example_dir: &Path, case: &TestCase) -> Result<RunReport, SlTestExampleError> {
    let scripts_xml = read_scripts_xml_from_dir(example_dir)?;
    let artifact = compile_artifact_from_xml_map(&scripts_xml, Some(case.entry_script.clone()))
        .map_err(SlTestExampleError::Engine)?;
    let mut engine = create_engine_from_artifact(CreateEngineFromArtifactOptions {
        artifact,
        entry_args: None,
        host_functions: None,
        random_seed: Some(1),
        random_sequence: None,
        random_sequence_index: None,
        compiler_version: Some(DEFAULT_COMPILER_VERSION.to_string()),
    })
    .map_err(SlTestExampleError::Engine)?;

    let mut observed_events = Vec::new();
    let mut action_index = 0usize;

    for step in 1..=MAX_STEPS {
        let output = engine.next_output().map_err(SlTestExampleError::Engine)?;
        match output {
            sl_core::EngineOutput::Text { text, tag } => {
                observed_events.push(ExpectedEvent::Text { text, tag });
            }
            sl_core::EngineOutput::Debug { .. } => {}
            sl_core::EngineOutput::Choices { items, prompt_text } => {
                let choices = items.into_iter().map(|item| item.text).collect();
                observed_events.push(ExpectedEvent::Choices {
                    prompt_text,
                    choices,
                });
                let event_index = observed_events.len() - 1;
                let action = case.actions.get(action_index).ok_or_else(|| {
                    SlTestExampleError::MissingAction {
                        event_index,
                        expected_action_kind: "choose".to_string(),
                    }
                })?;
                match action {
                    TestAction::Choose { index } => {
                        engine.choose(*index).map_err(SlTestExampleError::Engine)?;
                    }
                    _ => {
                        return Err(SlTestExampleError::ActionKindMismatch {
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
                ..
            } => {
                observed_events.push(ExpectedEvent::Input {
                    prompt_text,
                    default_text,
                });
                let event_index = observed_events.len() - 1;
                let action = case.actions.get(action_index).ok_or_else(|| {
                    SlTestExampleError::MissingAction {
                        event_index,
                        expected_action_kind: "input".to_string(),
                    }
                })?;
                match action {
                    TestAction::Input { text } => {
                        engine
                            .submit_input(text)
                            .map_err(SlTestExampleError::Engine)?;
                    }
                    _ => {
                        return Err(SlTestExampleError::ActionKindMismatch {
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
                    return Err(SlTestExampleError::UnusedActions {
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

    Err(SlTestExampleError::GuardExceeded {
        max_steps: MAX_STEPS,
    })
}

pub fn assert_case(example_dir: &Path, case_path: &Path) -> Result<(), SlTestExampleError> {
    let case = read_test_case(case_path)?;
    let report = run_case(example_dir, &case)?;

    if report.observed_events.len() != case.expected_events.len() {
        let observed = serde_json::to_string_pretty(&report.observed_events)
            .map_err(SlTestExampleError::EventSerialize)?;
        return Err(SlTestExampleError::EventCountMismatch {
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
            let expected =
                serde_json::to_string(expected).map_err(SlTestExampleError::EventSerialize)?;
            let actual =
                serde_json::to_string(actual).map_err(SlTestExampleError::EventSerialize)?;
            return Err(SlTestExampleError::EventMismatch {
                index,
                expected,
                actual,
            });
        }
    }

    Ok(())
}
