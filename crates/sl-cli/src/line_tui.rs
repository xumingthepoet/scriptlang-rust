use std::io::{self, BufRead, Write};
use std::path::Path;

use sl_core::{EngineOutput, ScriptLangError};
use sl_runtime::DEFAULT_COMPILER_VERSION;

use crate::{
    create_engine_for_scenario, load_engine_from_state_for_ref, map_tui_io, save_engine_state,
    LoadedScenario, TuiCommandAction, TuiCommandContext,
};

pub(crate) fn run_tui_line_mode(
    state_file: &str,
    scenario: &LoadedScenario,
    entry_script: &str,
    engine: &mut sl_runtime::ScriptLangEngine,
) -> Result<i32, ScriptLangError> {
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let mut writer = io::stdout();
    run_tui_line_mode_with_io(
        state_file,
        scenario,
        entry_script,
        engine,
        &mut reader,
        &mut writer,
    )
}

pub(crate) fn run_tui_line_mode_with_io(
    state_file: &str,
    scenario: &LoadedScenario,
    entry_script: &str,
    engine: &mut sl_runtime::ScriptLangEngine,
    reader: &mut dyn BufRead,
    writer: &mut dyn Write,
) -> Result<i32, ScriptLangError> {
    println!("ScriptLang TUI");
    println!("commands: :help :save :load :restart :quit");
    let command_context = TuiCommandContext {
        state_file,
        scenario,
        entry_script,
    };

    loop {
        match engine.next_output()? {
            EngineOutput::Text { text } => {
                println!();
                println!("{}", text);
            }
            EngineOutput::Choices { items, prompt_text } => {
                println!();
                if let Some(prompt_text) = prompt_text {
                    println!("{}", prompt_text);
                }
                for item in &items {
                    println!("  [{}] {}", item.index, item.text);
                }
                loop {
                    let raw = prompt_input_from("> ", reader, writer)?;
                    let mut emit = |line: String| println!("{}", line);
                    let action =
                        handle_line_cmd(raw.as_str(), &command_context, engine, &mut emit)?;
                    match action {
                        TuiCommandAction::Continue => continue,
                        TuiCommandAction::RefreshBoundary => break,
                        TuiCommandAction::Quit => return Ok(0),
                        TuiCommandAction::NotHandled => {}
                    }
                    let choice = raw.parse::<usize>().map_err(|_| {
                        ScriptLangError::new(
                            "TUI_CHOICE_PARSE",
                            format!("Invalid choice index: {}", raw),
                        )
                    })?;
                    engine.choose(choice)?;
                    break;
                }
            }
            EngineOutput::Input {
                prompt_text,
                default_text,
            } => {
                println!();
                println!("{}", prompt_text);
                println!("(default: {})", default_text);
                loop {
                    let raw = prompt_input_from("> ", reader, writer)?;
                    let mut emit = |line: String| println!("{}", line);
                    let action =
                        handle_line_cmd(raw.as_str(), &command_context, engine, &mut emit)?;
                    match action {
                        TuiCommandAction::Continue => continue,
                        TuiCommandAction::RefreshBoundary => break,
                        TuiCommandAction::Quit => return Ok(0),
                        TuiCommandAction::NotHandled => {}
                    }
                    engine.submit_input(&raw)?;
                    break;
                }
            }
            EngineOutput::End => {
                println!();
                println!("[END]");
                return Ok(0);
            }
        }
    }
}

pub(crate) fn handle_tui_command(
    raw: &str,
    state_file: &str,
    scenario: &LoadedScenario,
    entry_script: &str,
    engine: &mut sl_runtime::ScriptLangEngine,
    emit: &mut dyn FnMut(String),
) -> Result<TuiCommandAction, ScriptLangError> {
    match raw {
        ":help" => {
            emit("commands: :help :save :load :restart :quit".to_string());
            Ok(TuiCommandAction::Continue)
        }
        ":save" => {
            save_engine_state(
                Path::new(state_file),
                engine,
                &scenario.id,
                DEFAULT_COMPILER_VERSION,
            )?;
            emit(format!("saved: {}", state_file));
            Ok(TuiCommandAction::Continue)
        }
        ":load" => {
            let (_, _, resumed) = load_engine_from_state_for_ref(Path::new(state_file))?;
            *engine = resumed;
            emit(format!("loaded: {}", state_file));
            Ok(TuiCommandAction::RefreshBoundary)
        }
        ":restart" => {
            let mut restarted = create_engine_for_scenario(scenario, entry_script)?;
            std::mem::swap(engine, &mut restarted);
            emit("restarted".to_string());
            Ok(TuiCommandAction::RefreshBoundary)
        }
        ":quit" => {
            emit("bye".to_string());
            Ok(TuiCommandAction::Quit)
        }
        _ => Ok(TuiCommandAction::NotHandled),
    }
}

pub(crate) fn handle_line_cmd(
    raw: &str,
    context: &TuiCommandContext<'_>,
    engine: &mut sl_runtime::ScriptLangEngine,
    emit: &mut dyn FnMut(String),
) -> Result<TuiCommandAction, ScriptLangError> {
    handle_tui_command(
        raw,
        context.state_file,
        context.scenario,
        context.entry_script,
        engine,
        emit,
    )
}

pub(crate) fn prompt_input_from(
    prefix: &str,
    reader: &mut dyn BufRead,
    writer: &mut dyn Write,
) -> Result<String, ScriptLangError> {
    write!(writer, "{}", prefix).map_err(map_tui_io)?;
    writer.flush().map_err(map_tui_io)?;
    let mut input = String::new();
    reader.read_line(&mut input).map_err(map_tui_io)?;
    Ok(input.trim_end_matches(&['\r', '\n'][..]).to_string())
}
