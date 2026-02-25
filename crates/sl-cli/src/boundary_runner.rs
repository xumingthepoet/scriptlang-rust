use sl_core::{EngineOutput, ScriptLangError};

use crate::{BoundaryEvent, BoundaryResult};

pub(crate) fn run_to_boundary(
    engine: &mut sl_runtime::ScriptLangEngine,
) -> Result<BoundaryResult, ScriptLangError> {
    let mut texts = Vec::new();

    loop {
        match engine.next_output()? {
            EngineOutput::Text { text } => texts.push(text),
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

    for text in boundary.texts {
        println!(
            "TEXT_JSON:{}",
            serde_json::to_string(&text).expect("string json")
        );
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
