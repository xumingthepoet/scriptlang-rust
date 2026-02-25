use std::path::Path;

use sl_api::{
    create_engine_from_xml, resume_engine_from_xml, CreateEngineFromXmlOptions,
    ResumeEngineFromXmlOptions,
};
use sl_core::ScriptLangError;
use sl_runtime::DEFAULT_COMPILER_VERSION;

use crate::{
    emit_boundary, load_player_state, load_source_by_ref, load_source_by_scripts_dir,
    run_to_boundary, save_player_state, AgentArgs, AgentCommand, BoundaryEvent, ChooseArgs,
    InputArgs, PlayerStateV3, StartArgs, PLAYER_STATE_SCHEMA,
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

    let mut engine = create_engine_from_xml(CreateEngineFromXmlOptions {
        scripts_xml: scenario.scripts_xml.clone(),
        entry_script: Some(scenario.entry_script.clone()),
        entry_args: None,
        host_functions: None,
        random_seed: None,
        compiler_version: Some(DEFAULT_COMPILER_VERSION.to_string()),
    })?;

    let boundary = run_to_boundary(&mut engine)?;

    if matches!(
        boundary.event,
        BoundaryEvent::Choices | BoundaryEvent::Input
    ) {
        let snapshot = engine.snapshot()?;
        let state = PlayerStateV3 {
            schema_version: PLAYER_STATE_SCHEMA.to_string(),
            scenario_id: scenario.id,
            compiler_version: DEFAULT_COMPILER_VERSION.to_string(),
            snapshot,
        };
        save_player_state(Path::new(&args.state_out), &state)?;
        emit_boundary(boundary, Some(args.state_out));
        return Ok(0);
    }

    emit_boundary(boundary, None);
    Ok(0)
}

pub(super) fn run_choose(args: ChooseArgs) -> Result<i32, ScriptLangError> {
    let state = load_player_state(Path::new(&args.state_in))?;
    let scenario = load_source_by_ref(&state.scenario_id)?;

    let mut engine = resume_engine_from_xml(ResumeEngineFromXmlOptions {
        scripts_xml: scenario.scripts_xml.clone(),
        snapshot: state.snapshot,
        host_functions: None,
        compiler_version: Some(state.compiler_version.clone()),
    })?;

    engine.choose(args.choice)?;
    let boundary = run_to_boundary(&mut engine)?;

    if matches!(
        boundary.event,
        BoundaryEvent::Choices | BoundaryEvent::Input
    ) {
        let next_state = PlayerStateV3 {
            schema_version: PLAYER_STATE_SCHEMA.to_string(),
            scenario_id: state.scenario_id,
            compiler_version: state.compiler_version,
            snapshot: engine.snapshot()?,
        };
        save_player_state(Path::new(&args.state_out), &next_state)?;
        emit_boundary(boundary, Some(args.state_out));
        return Ok(0);
    }

    emit_boundary(boundary, None);
    Ok(0)
}

pub(super) fn run_input(args: InputArgs) -> Result<i32, ScriptLangError> {
    let state = load_player_state(Path::new(&args.state_in))?;
    let scenario = load_source_by_ref(&state.scenario_id)?;

    let mut engine = resume_engine_from_xml(ResumeEngineFromXmlOptions {
        scripts_xml: scenario.scripts_xml.clone(),
        snapshot: state.snapshot,
        host_functions: None,
        compiler_version: Some(state.compiler_version.clone()),
    })?;

    engine.submit_input(&args.text)?;
    let boundary = run_to_boundary(&mut engine)?;

    if matches!(
        boundary.event,
        BoundaryEvent::Choices | BoundaryEvent::Input
    ) {
        let next_state = PlayerStateV3 {
            schema_version: PLAYER_STATE_SCHEMA.to_string(),
            scenario_id: state.scenario_id,
            compiler_version: state.compiler_version,
            snapshot: engine.snapshot()?,
        };
        save_player_state(Path::new(&args.state_out), &next_state)?;
        emit_boundary(boundary, Some(args.state_out));
        return Ok(0);
    }

    emit_boundary(boundary, None);
    Ok(0)
}
