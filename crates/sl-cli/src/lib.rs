use std::ffi::OsString;

use clap::Parser;
use sl_api::{create_engine_from_xml, CreateEngineFromXmlOptions};
use sl_core::ScriptLangError;
use sl_runtime::DEFAULT_COMPILER_VERSION;

mod agent;
mod boundary_runner;
mod cli_args;
mod error_map;
mod line_tui;
mod models;
mod source_loader;
mod state_store;
mod tui;

pub(crate) use boundary_runner::{emit_boundary, run_to_boundary};
pub(crate) use cli_args::{
    AgentArgs, AgentCommand, ChooseArgs, Cli, InputArgs, Mode, StartArgs, TuiArgs,
};
pub(crate) use error_map::{
    emit_error, map_cli_source_path, map_cli_source_read, map_cli_source_scan,
    map_cli_state_invalid, map_cli_state_read, map_cli_state_write, map_tui_io,
};
pub(crate) use line_tui::run_tui_line_mode;
#[cfg(test)]
pub(crate) use line_tui::{handle_line_cmd, handle_tui_command};
pub(crate) use models::{
    BoundaryEvent, BoundaryResult, LoadedScenario, PlayerStateV3, TuiCommandAction,
    TuiCommandContext, PLAYER_STATE_SCHEMA,
};
pub(crate) use source_loader::{load_source_by_ref, load_source_by_scripts_dir};
pub(crate) use state_store::{load_player_state, save_player_state};

pub fn run_cli_from_args<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(error) => return error.exit_code(),
    };
    match run(cli) {
        Ok(code) => code,
        Err(error) => emit_error(error),
    }
}

fn run(cli: Cli) -> Result<i32, ScriptLangError> {
    match cli.command {
        Mode::Agent(args) => run_agent(args),
        Mode::Tui(args) => run_tui(args),
    }
}

fn run_agent(args: AgentArgs) -> Result<i32, ScriptLangError> {
    agent::run_agent(args)
}

fn run_tui(args: TuiArgs) -> Result<i32, ScriptLangError> {
    let entry_script = args.entry_script.unwrap_or("main".to_string());
    let state_file = args
        .state_file
        .unwrap_or(".scriptlang/save.json".to_string());
    let scenario = load_source_by_scripts_dir(&args.scripts_dir, &entry_script)?;
    let mut engine = create_engine_from_xml(CreateEngineFromXmlOptions {
        scripts_xml: scenario.scripts_xml.clone(),
        entry_script: Some(entry_script.clone()),
        entry_args: None,
        host_functions: None,
        random_seed: None,
        compiler_version: Some(DEFAULT_COMPILER_VERSION.to_string()),
    })?;

    tui::run_tui_ratatui_mode(&state_file, &scenario, &entry_script, &mut engine)
}

#[cfg(test)]
pub(crate) mod cli_test_support {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    pub(crate) fn temp_path(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be monotonic")
            .as_nanos();
        std::env::temp_dir().join(format!("scriptlang-rs-{}-{}", name, nanos))
    }

    pub(crate) fn write_file(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("parent should be created");
        }
        fs::write(path, content).expect("file should be written");
    }

    pub(crate) fn example_scripts_dir(example: &str) -> String {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("examples")
            .join("scripts-rhai")
            .join(example)
            .to_string_lossy()
            .to_string()
    }
}

#[cfg(test)]
mod lib_tests {
    use super::*;
    use crate::cli_test_support::*;
    use std::path::Path;

    #[test]
    fn run_entry_and_command_helpers_paths_are_covered() {
        let choice_scenario = example_scripts_dir("06-snapshot-flow");
        let input_scenario = example_scripts_dir("16-input-name");
        let text_scenario = example_scripts_dir("01-text-code");

        let start_state = temp_path("run-start-state.json");
        let choose_state = temp_path("run-choose-state.json");
        let input_state_1 = temp_path("run-input-state-1.json");
        let input_state_2 = temp_path("run-input-state-2.json");
        let tui_state = temp_path("run-tui-state.json");
        let tui_state_str = tui_state.to_string_lossy().to_string();

        let start_code = run(Cli {
            command: Mode::Agent(AgentArgs {
                command: AgentCommand::Start(StartArgs {
                    scripts_dir: choice_scenario.clone(),
                    entry_script: Some("main".to_string()),
                    state_out: start_state.to_string_lossy().to_string(),
                }),
            }),
        })
        .expect("agent start should pass");
        assert_eq!(start_code, 0);

        let choose_code = run_agent(AgentArgs {
            command: AgentCommand::Choose(ChooseArgs {
                state_in: start_state.to_string_lossy().to_string(),
                choice: 0,
                state_out: choose_state.to_string_lossy().to_string(),
            }),
        })
        .expect("agent choose should pass");
        assert_eq!(choose_code, 0);

        let input_start_code = agent::run_start(StartArgs {
            scripts_dir: input_scenario.clone(),
            entry_script: Some("main".to_string()),
            state_out: input_state_1.to_string_lossy().to_string(),
        })
        .expect("input scenario start should pass");
        assert_eq!(input_start_code, 0);

        let input_code = agent::run_input(InputArgs {
            state_in: input_state_1.to_string_lossy().to_string(),
            text: "Guild".to_string(),
            state_out: input_state_2.to_string_lossy().to_string(),
        })
        .expect("agent input should pass");
        assert_eq!(input_code, 0);

        let tui_code = run_tui(TuiArgs {
            scripts_dir: text_scenario,
            entry_script: Some("main".to_string()),
            state_file: Some(tui_state_str.clone()),
        })
        .expect("tui should pass in line mode");
        assert_eq!(tui_code, 0);

        let loaded = load_source_by_scripts_dir(&choice_scenario, "main").expect("load source");
        let mut engine = create_engine_from_xml(CreateEngineFromXmlOptions {
            scripts_xml: loaded.scripts_xml.clone(),
            entry_script: Some("main".to_string()),
            entry_args: None,
            host_functions: None,
            random_seed: Some(1),
            compiler_version: Some(DEFAULT_COMPILER_VERSION.to_string()),
        })
        .expect("engine should build");
        let _ = run_to_boundary(&mut engine).expect("boundary");

        let mut emitted = Vec::new();
        let mut emit = |line: String| emitted.push(line);
        let action = handle_tui_command(
            ":help",
            &tui_state_str,
            &loaded,
            "main",
            &mut engine,
            &mut emit,
        )
        .expect("command should pass");
        assert_eq!(action, TuiCommandAction::Continue);
        let context = TuiCommandContext {
            state_file: &tui_state_str,
            scenario: &loaded,
            entry_script: "main",
        };
        let action = handle_line_cmd(":quit", &context, &mut engine, &mut emit)
            .expect("line command should pass");
        assert_eq!(action, TuiCommandAction::Quit);
    }

    #[test]
    fn parse_error_and_error_mapper_helpers_are_covered() {
        let parse_code = run_cli_from_args(["scriptlang-player", "agent", "unknown"]);
        assert_ne!(parse_code, 0);

        let io_error = std::io::Error::other("io");
        assert_eq!(map_tui_io(io_error).code, "TUI_IO");

        let io_error = std::io::Error::other("path");
        assert_eq!(map_cli_source_path(io_error).code, "CLI_SOURCE_PATH");

        let strip_error = Path::new("/a")
            .strip_prefix("/b")
            .expect_err("strip prefix");
        assert_eq!(map_cli_source_scan(strip_error).code, "CLI_SOURCE_SCAN");

        let io_error = std::io::Error::other("read");
        assert_eq!(map_cli_source_read(io_error).code, "CLI_SOURCE_READ");

        let io_error = std::io::Error::other("write");
        assert_eq!(map_cli_state_write(io_error).code, "CLI_STATE_WRITE");

        let io_error = std::io::Error::other("read");
        assert_eq!(map_cli_state_read(io_error).code, "CLI_STATE_READ");

        let invalid = serde_json::from_str::<serde_json::Value>("{").expect_err("invalid json");
        assert_eq!(map_cli_state_invalid(invalid).code, "CLI_STATE_INVALID");
    }
}
