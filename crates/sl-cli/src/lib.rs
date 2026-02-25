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
#[cfg(test)]
pub(crate) use source_loader::{
    make_scripts_dir_scenario_id, read_scripts_xml_from_dir, resolve_scripts_dir,
};
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
mod tests;
