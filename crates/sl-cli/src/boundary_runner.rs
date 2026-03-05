use sl_api::{EngineOutput, ScriptLangError};

use crate::{BoundaryEvent, BoundaryResult, TextEvent};

pub(crate) fn run_to_boundary(
    engine: &mut sl_api::ScriptLangEngine,
) -> Result<BoundaryResult, ScriptLangError> {
    let mut texts = Vec::new();

    loop {
        match engine.next_output()? {
            EngineOutput::Text { text, tag } => texts.push(TextEvent { text, tag }),
            EngineOutput::Choices { items, prompt_text } => {
                return Ok(BoundaryResult {
                    event: BoundaryEvent::Choices,
                    texts,
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
            } => {
                return Ok(BoundaryResult {
                    event: BoundaryEvent::Input,
                    texts,
                    choices: Vec::new(),
                    choice_prompt_text: None,
                    input_prompt_text: Some(prompt_text),
                    input_default_text: Some(default_text),
                })
            }
            EngineOutput::End => {
                return Ok(BoundaryResult {
                    event: BoundaryEvent::End,
                    texts,
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

    for text_event in boundary.texts {
        println!(
            "TEXT_JSON:{}",
            serde_json::to_string(&text_event.text).expect("string json")
        );
        if let Some(tag) = text_event.tag {
            println!(
                "TEXT_TAG_JSON:{}",
                serde_json::to_string(&tag).expect("string json")
            );
        }
    }

    if let Some(prompt) = boundary.choice_prompt_text {
        println!(
            "PROMPT_JSON:{}",
            serde_json::to_string(&prompt).expect("string json")
        );
    }

    if let Some(prompt) = boundary.input_prompt_text {
        println!(
            "PROMPT_JSON:{}",
            serde_json::to_string(&prompt).expect("string json")
        );
    }

    for (index, text) in boundary.choices {
        println!(
            "CHOICE:{}|{}",
            index,
            serde_json::to_string(&text).expect("string json")
        );
    }

    if let Some(default_text) = boundary.input_default_text {
        println!(
            "INPUT_DEFAULT_JSON:{}",
            serde_json::to_string(&default_text).expect("string json")
        );
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
            load_source_by_scripts_dir(&scripts_dir, "main").expect("source should be loaded");
        assert!(loaded.id.starts_with("scripts-dir:"));
        assert_eq!(loaded.entry_script, "main");

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

        let boundary = run_to_boundary(&mut engine).expect("boundary should be emitted");
        assert_eq!(boundary.event, BoundaryEvent::Choices);
        assert!(!boundary.choices.is_empty());

        let loaded_by_ref = load_source_by_ref(&loaded.id).expect("load by ref should pass");
        assert_eq!(loaded_by_ref.entry_script, "main");
    }

    #[test]
    fn emit_boundary_supports_optional_text_tag_output() {
        emit_boundary(
            BoundaryResult {
                event: BoundaryEvent::End,
                texts: vec![
                    TextEvent {
                        text: "plain".to_string(),
                        tag: None,
                    },
                    TextEvent {
                        text: "sfx/path.ogg".to_string(),
                        tag: Some("sound".to_string()),
                    },
                ],
                choices: Vec::new(),
                choice_prompt_text: None,
                input_prompt_text: None,
                input_default_text: None,
            },
            None,
        );
    }
}
