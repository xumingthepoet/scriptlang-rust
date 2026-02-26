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

#[cfg(test)]
mod line_tui_tests {
    use super::*;

    use crate::cli_test_support::{temp_path, write_file};
    use crate::{create_engine_for_scenario, load_source_by_scripts_dir, run_to_boundary};
    use std::io::Cursor;

    fn load_temp_scenario(name: &str, script_xml: &str) -> LoadedScenario {
        let scripts_dir = temp_path(name);
        write_file(&scripts_dir.join("main.script.xml"), script_xml);
        let scripts_dir_str = scripts_dir.to_string_lossy().to_string();
        load_source_by_scripts_dir(&scripts_dir_str, "main").expect("scenario should load")
    }

    fn create_engine_for_tests(scenario: &LoadedScenario) -> sl_runtime::ScriptLangEngine {
        create_engine_for_scenario(scenario, "main").expect("engine should be created")
    }

    #[test]
    fn run_tui_line_mode_with_io_handles_choice_input_and_command_paths() {
        let choice_input_scenario = load_temp_scenario(
            "line-tui-choice-input",
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
        let state_file = temp_path("line-tui-choice-input-save.json");
        let state_file_str = state_file.to_string_lossy().to_string();

        let mut engine = create_engine_for_tests(&choice_input_scenario);
        let mut reader = Cursor::new(b":help\n0\n:help\nGuild\n".to_vec());
        let mut writer = Vec::new();
        let code = run_tui_line_mode_with_io(
            &state_file_str,
            &choice_input_scenario,
            "main",
            &mut engine,
            &mut reader,
            &mut writer,
        )
        .expect("choice + input flow should pass");
        assert_eq!(code, 0);

        let choice_quit_scenario = load_temp_scenario(
            "line-tui-choice-quit",
            r#"
<script name="main">
  <choice text="Pick">
    <option text="A"><text>A</text></option>
  </choice>
</script>
"#,
        );
        let mut engine = create_engine_for_tests(&choice_quit_scenario);
        let mut reader = Cursor::new(b":quit\n".to_vec());
        let mut writer = Vec::new();
        let code = run_tui_line_mode_with_io(
            &state_file_str,
            &choice_quit_scenario,
            "main",
            &mut engine,
            &mut reader,
            &mut writer,
        )
        .expect("quit from choices should pass");
        assert_eq!(code, 0);

        let mut engine = create_engine_for_tests(&choice_quit_scenario);
        let mut reader = Cursor::new(b":restart\n0\n".to_vec());
        let mut writer = Vec::new();
        let code = run_tui_line_mode_with_io(
            &state_file_str,
            &choice_quit_scenario,
            "main",
            &mut engine,
            &mut reader,
            &mut writer,
        )
        .expect("restart from choices should pass");
        assert_eq!(code, 0);

        let input_quit_scenario = load_temp_scenario(
            "line-tui-input-quit",
            r#"
<script name="main">
  <var name="name" type="string">"Traveler"</var>
  <input var="name" text="Name"/>
  <text>${name}</text>
</script>
"#,
        );
        let mut engine = create_engine_for_tests(&input_quit_scenario);
        let mut reader = Cursor::new(b":quit\n".to_vec());
        let mut writer = Vec::new();
        let code = run_tui_line_mode_with_io(
            &state_file_str,
            &input_quit_scenario,
            "main",
            &mut engine,
            &mut reader,
            &mut writer,
        )
        .expect("quit from input should pass");
        assert_eq!(code, 0);

        let mut engine = create_engine_for_tests(&input_quit_scenario);
        let mut reader = Cursor::new(b":restart\nGuild\n".to_vec());
        let mut writer = Vec::new();
        let code = run_tui_line_mode_with_io(
            &state_file_str,
            &input_quit_scenario,
            "main",
            &mut engine,
            &mut reader,
            &mut writer,
        )
        .expect("restart from input should pass");
        assert_eq!(code, 0);

        let mut engine = create_engine_for_tests(&choice_quit_scenario);
        let mut reader = Cursor::new(b"invalid\n".to_vec());
        let mut writer = Vec::new();
        let error = run_tui_line_mode_with_io(
            &state_file_str,
            &choice_quit_scenario,
            "main",
            &mut engine,
            &mut reader,
            &mut writer,
        )
        .expect_err("invalid choice should fail");
        assert_eq!(error.code, "TUI_CHOICE_PARSE");
    }

    #[test]
    fn handle_tui_command_covers_all_commands() {
        let scenario = load_temp_scenario(
            "line-tui-commands",
            r#"
<script name="main">
  <choice text="Pick">
    <option text="A"><text>A</text></option>
  </choice>
</script>
"#,
        );
        let mut engine = create_engine_for_tests(&scenario);
        let _ = run_to_boundary(&mut engine).expect("boundary should resolve");

        let state_file = temp_path("line-tui-commands-save.json");
        let state_file_str = state_file.to_string_lossy().to_string();
        let mut emitted = Vec::new();
        let mut emit = |line: String| emitted.push(line);

        let action = handle_tui_command(
            ":save",
            &state_file_str,
            &scenario,
            "main",
            &mut engine,
            &mut emit,
        )
        .expect("save command should pass");
        assert_eq!(action, TuiCommandAction::Continue);

        engine.choose(0).expect("choose should pass");
        let action = handle_tui_command(
            ":load",
            &state_file_str,
            &scenario,
            "main",
            &mut engine,
            &mut emit,
        )
        .expect("load command should pass");
        assert_eq!(action, TuiCommandAction::RefreshBoundary);
        assert!(matches!(
            engine.next_output().expect("output should resolve"),
            EngineOutput::Choices { .. }
        ));

        let action = handle_tui_command(
            ":restart",
            &state_file_str,
            &scenario,
            "main",
            &mut engine,
            &mut emit,
        )
        .expect("restart command should pass");
        assert_eq!(action, TuiCommandAction::RefreshBoundary);
        assert!(matches!(
            engine.next_output().expect("output should resolve"),
            EngineOutput::Choices { .. }
        ));

        let action = handle_tui_command(
            ":unknown",
            &state_file_str,
            &scenario,
            "main",
            &mut engine,
            &mut emit,
        )
        .expect("unknown command should pass");
        assert_eq!(action, TuiCommandAction::NotHandled);
    }

    #[test]
    fn handle_line_cmd_delegates_to_tui_command_handler() {
        let scenario = load_temp_scenario(
            "line-tui-handle-line",
            r#"<script name="main"><text>ok</text></script>"#,
        );
        let state_file = temp_path("line-tui-handle-line-state.json");
        let state_file_str = state_file.to_string_lossy().to_string();
        let context = TuiCommandContext {
            state_file: &state_file_str,
            scenario: &scenario,
            entry_script: "main",
        };
        let mut engine = create_engine_for_tests(&scenario);
        let mut emitted = Vec::new();
        let mut emit = |line: String| emitted.push(line);

        let action = handle_line_cmd(":help", &context, &mut engine, &mut emit)
            .expect("line command should pass");
        assert_eq!(action, TuiCommandAction::Continue);
    }

    #[test]
    fn prompt_input_from_reads_and_trims_lines() {
        let mut reader = Cursor::new(b"hello\r\n".to_vec());
        let mut writer = Vec::new();
        let input = prompt_input_from("> ", &mut reader, &mut writer).expect("prompt should read");
        assert_eq!(input, "hello");
        assert_eq!(writer, b"> ");
    }
}
