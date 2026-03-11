use sl_api::{EngineOutput, ScriptLangError};

use crate::{BoundaryEvent, BoundaryResult, DebugEvent, OutputEvent, TextEvent};

fn json_string(value: &str) -> String {
    serde_json::Value::String(value.to_string()).to_string()
}

pub(crate) fn run_to_boundary(
    engine: &mut sl_api::ScriptLangEngine,
    show_debug: bool,
) -> Result<BoundaryResult, ScriptLangError> {
    let mut outputs = Vec::new();

    loop {
        match engine.next_output()? {
            EngineOutput::Text { text, tag } => {
                outputs.push(OutputEvent::Text(TextEvent { text, tag }))
            }
            EngineOutput::Debug { text } => {
                if show_debug {
                    outputs.push(OutputEvent::Debug(DebugEvent { text }));
                }
            }
            EngineOutput::Choices { items, prompt_text } => {
                return Ok(BoundaryResult {
                    event: BoundaryEvent::Choices,
                    outputs,
                    choices: items
                        .into_iter()
                        .map(|item| (item.index, item.text))
                        .collect(),
                    choice_prompt_text: prompt_text,
                    input_prompt_text: None,
                    input_default_text: None,
                })
            }
            EngineOutput::Input {
                prompt_text,
                default_text,
                ..
            } => {
                return Ok(BoundaryResult {
                    event: BoundaryEvent::Input,
                    outputs,
                    choices: Vec::new(),
                    choice_prompt_text: None,
                    input_prompt_text: Some(prompt_text),
                    input_default_text: Some(default_text),
                })
            }
            EngineOutput::End => {
                return Ok(BoundaryResult {
                    event: BoundaryEvent::End,
                    outputs,
                    choices: Vec::new(),
                    choice_prompt_text: None,
                    input_prompt_text: None,
                    input_default_text: None,
                })
            }
        }
    }
}

pub(crate) fn emit_boundary(boundary: BoundaryResult, state_out: Option<String>) {
    println!("RESULT:OK");
    match boundary.event {
        BoundaryEvent::Choices => println!("EVENT:CHOICES"),
        BoundaryEvent::Input => println!("EVENT:INPUT"),
        BoundaryEvent::End => println!("EVENT:END"),
    }

    for output in boundary.outputs {
        match output {
            OutputEvent::Text(text_event) => {
                println!("TEXT_JSON:{}", json_string(&text_event.text));
                if let Some(tag) = text_event.tag {
                    println!("TEXT_TAG_JSON:{}", json_string(&tag));
                }
            }
            OutputEvent::Debug(debug_event) => {
                println!("DEBUG_JSON:{}", json_string(&debug_event.text));
            }
        }
    }

    if let Some(prompt) = boundary.choice_prompt_text {
        println!("PROMPT_JSON:{}", json_string(&prompt));
    }

    if let Some(prompt) = boundary.input_prompt_text {
        println!("PROMPT_JSON:{}", json_string(&prompt));
    }

    for (index, text) in boundary.choices {
        println!("CHOICE:{}|{}", index, json_string(&text));
    }

    if let Some(default_text) = boundary.input_default_text {
        println!("INPUT_DEFAULT_JSON:{}", json_string(&default_text));
    }

    println!(
        "STATE_OUT:{}",
        state_out.unwrap_or_else(|| "NONE".to_string())
    );
}

#[cfg(test)]
mod boundary_runner_tests {
    use super::*;
    use crate::cli_test_support::*;
    use crate::{load_source_by_ref, load_source_by_scripts_dir};
    use sl_api::DEFAULT_COMPILER_VERSION;
    use sl_api::{create_engine_from_xml, CreateEngineFromXmlOptions};

    #[test]
    fn run_to_boundary_and_load_source_helpers_work_with_examples() {
        let scripts_dir = example_scripts_dir("06-snapshot-flow");
        let loaded =
            load_source_by_scripts_dir(&scripts_dir, "main.main").expect("source should be loaded");
        assert!(loaded.id.starts_with("scripts-dir:"));
        assert_eq!(loaded.entry_script, "main.main");

        let mut engine = create_engine_from_xml(CreateEngineFromXmlOptions {
            scripts_xml: loaded.scripts_xml.clone(),
            entry_script: Some(loaded.entry_script.clone()),
            entry_args: None,
            host_functions: None,
            random_seed: Some(1),
            random_sequence: None,
            random_sequence_index: None,
            compiler_version: Some(DEFAULT_COMPILER_VERSION.to_string()),
        })
        .expect("engine should build");

        let boundary = run_to_boundary(&mut engine, false).expect("boundary should be emitted");
        assert_eq!(boundary.event, BoundaryEvent::Choices);
        assert!(!boundary.choices.is_empty());

        let loaded_by_ref = load_source_by_ref(&loaded.id).expect("load by ref should pass");
        assert_eq!(loaded_by_ref.entry_script, "main.main");
    }

    #[test]
    fn emit_boundary_supports_optional_text_tag_output() {
        emit_boundary(
            BoundaryResult {
                event: BoundaryEvent::End,
                outputs: vec![
                    OutputEvent::Text(TextEvent {
                        text: "plain".to_string(),
                        tag: None,
                    }),
                    OutputEvent::Debug(DebugEvent {
                        text: "dbg".to_string(),
                    }),
                    OutputEvent::Text(TextEvent {
                        text: "sfx/path.ogg".to_string(),
                        tag: Some("sound".to_string()),
                    }),
                ],
                choices: Vec::new(),
                choice_prompt_text: None,
                input_prompt_text: None,
                input_default_text: None,
            },
            None,
        );
    }

    #[test]
    fn run_to_boundary_hides_or_shows_debug_events_by_flag() {
        let mut hidden = create_engine_from_xml(CreateEngineFromXmlOptions {
            scripts_xml: std::collections::BTreeMap::from([(
                "main.xml".to_string(),
                r#"<module name="main" default_access="public">
<script name="main"><debug>dbg</debug><text>ok</text></script>
</module>"#
                    .to_string(),
            )]),
            entry_script: Some("main.main".to_string()),
            entry_args: None,
            host_functions: None,
            random_seed: Some(1),
            random_sequence: None,
            random_sequence_index: None,
            compiler_version: Some(DEFAULT_COMPILER_VERSION.to_string()),
        })
        .expect("engine should build");
        let hidden_boundary = run_to_boundary(&mut hidden, false).expect("boundary hidden");
        assert_eq!(hidden_boundary.outputs.len(), 1);
        assert!(matches!(hidden_boundary.outputs[0], OutputEvent::Text(_)));

        let mut shown = create_engine_from_xml(CreateEngineFromXmlOptions {
            scripts_xml: std::collections::BTreeMap::from([(
                "main.xml".to_string(),
                r#"<module name="main" default_access="public">
<script name="main"><debug>dbg</debug><text>ok</text></script>
</module>"#
                    .to_string(),
            )]),
            entry_script: Some("main.main".to_string()),
            entry_args: None,
            host_functions: None,
            random_seed: Some(1),
            random_sequence: None,
            random_sequence_index: None,
            compiler_version: Some(DEFAULT_COMPILER_VERSION.to_string()),
        })
        .expect("engine should build");
        let shown_boundary = run_to_boundary(&mut shown, true).expect("boundary shown");
        assert_eq!(shown_boundary.outputs.len(), 2);
        assert!(matches!(shown_boundary.outputs[0], OutputEvent::Debug(_)));
        assert!(matches!(shown_boundary.outputs[1], OutputEvent::Text(_)));
    }
}
