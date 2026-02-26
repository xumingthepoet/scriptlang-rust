use std::path::Path;

use sl_core::ScriptLangError;
use sl_runtime::DEFAULT_COMPILER_VERSION;

use crate::{
    create_engine_for_scenario, emit_boundary_with_saved_state, load_player_state,
    load_source_by_ref, load_source_by_scripts_dir, resume_engine_for_state, run_to_boundary,
    AgentArgs, AgentCommand, ChooseArgs, InputArgs, StartArgs,
};

pub(super) fn run_agent(args: AgentArgs) -> Result<i32, ScriptLangError> {
    match args.command {
        AgentCommand::Start(args) => run_start(args),
        AgentCommand::Choose(args) => run_choose(args),
        AgentCommand::Input(args) => run_input(args),
    }
}

pub(super) fn run_start(args: StartArgs) -> Result<i32, ScriptLangError> {
    let scenario = load_source_by_scripts_dir(
        &args.scripts_dir,
        args.entry_script.as_deref().unwrap_or("main"),
    )?;
    let mut engine = create_engine_for_scenario(&scenario, &scenario.entry_script)?;

    let boundary = run_to_boundary(&mut engine)?;
    emit_boundary_with_saved_state(
        &engine,
        boundary,
        &args.state_out,
        &scenario.id,
        DEFAULT_COMPILER_VERSION,
    )
}

pub(super) fn run_choose(args: ChooseArgs) -> Result<i32, ScriptLangError> {
    run_state_transition(&args.state_in, &args.state_out, |engine| {
        engine.choose(args.choice)
    })
}

pub(super) fn run_input(args: InputArgs) -> Result<i32, ScriptLangError> {
    run_state_transition(&args.state_in, &args.state_out, |engine| {
        engine.submit_input(&args.text)
    })
}

fn run_state_transition(
    state_in: &str,
    state_out: &str,
    transition: impl FnOnce(&mut sl_runtime::ScriptLangEngine) -> Result<(), ScriptLangError>,
) -> Result<i32, ScriptLangError> {
    let state = load_player_state(Path::new(state_in))?;
    let scenario = load_source_by_ref(&state.scenario_id)?;
    let mut engine = resume_engine_for_state(&scenario, &state)?;
    transition(&mut engine)?;
    let boundary = run_to_boundary(&mut engine)?;
    emit_boundary_with_saved_state(
        &engine,
        boundary,
        state_out,
        &state.scenario_id,
        &state.compiler_version,
    )
}
