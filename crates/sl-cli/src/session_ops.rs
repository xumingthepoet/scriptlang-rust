use std::path::Path;

use sl_api::{
    create_engine_from_xml, resume_engine_from_xml, CreateEngineFromXmlOptions,
    ResumeEngineFromXmlOptions,
};
use sl_core::ScriptLangError;
use sl_runtime::DEFAULT_COMPILER_VERSION;

use crate::{
    emit_boundary, load_player_state, load_source_by_ref, save_player_state, BoundaryEvent,
    BoundaryResult, LoadedScenario, PlayerStateV3, PLAYER_STATE_SCHEMA,
};

pub(crate) fn create_engine_for_scenario(
    scenario: &LoadedScenario,
    entry_script: &str,
) -> Result<sl_runtime::ScriptLangEngine, ScriptLangError> {
    create_engine_from_xml(CreateEngineFromXmlOptions {
        scripts_xml: scenario.scripts_xml.clone(),
        entry_script: Some(entry_script.to_string()),
        entry_args: None,
        host_functions: None,
        random_seed: None,
        compiler_version: Some(DEFAULT_COMPILER_VERSION.to_string()),
    })
}

pub(crate) fn resume_engine_for_state(
    scenario: &LoadedScenario,
    state: &PlayerStateV3,
) -> Result<sl_runtime::ScriptLangEngine, ScriptLangError> {
    resume_engine_from_xml(ResumeEngineFromXmlOptions {
        scripts_xml: scenario.scripts_xml.clone(),
        snapshot: state.snapshot.clone(),
        host_functions: None,
        compiler_version: Some(state.compiler_version.clone()),
    })
}

pub(crate) fn save_engine_state(
    path: &Path,
    engine: &sl_runtime::ScriptLangEngine,
    scenario_id: &str,
    compiler_version: &str,
) -> Result<(), ScriptLangError> {
    let snapshot = engine.snapshot()?;
    let state = PlayerStateV3 {
        schema_version: PLAYER_STATE_SCHEMA.to_string(),
        scenario_id: scenario_id.to_string(),
        compiler_version: compiler_version.to_string(),
        snapshot,
    };
    save_player_state(path, &state)
}

pub(crate) fn load_engine_from_state_for_ref(
    path: &Path,
) -> Result<(LoadedScenario, PlayerStateV3, sl_runtime::ScriptLangEngine), ScriptLangError> {
    let state = load_player_state(path)?;
    let scenario = load_source_by_ref(&state.scenario_id)?;
    let engine = resume_engine_for_state(&scenario, &state)?;
    Ok((scenario, state, engine))
}

pub(crate) fn load_engine_from_state_for_scenario(
    path: &Path,
    scenario: &LoadedScenario,
) -> Result<(PlayerStateV3, sl_runtime::ScriptLangEngine), ScriptLangError> {
    let state = load_player_state(path)?;
    if state.scenario_id != scenario.id {
        return Err(ScriptLangError::new(
            "TUI_STATE_SCENARIO_MISMATCH",
            format!(
                "State scenario mismatch. expected={} actual={}",
                scenario.id, state.scenario_id
            ),
        ));
    }
    let engine = resume_engine_for_state(scenario, &state)?;
    Ok((state, engine))
}

pub(crate) fn emit_boundary_with_saved_state(
    engine: &sl_runtime::ScriptLangEngine,
    boundary: BoundaryResult,
    state_out: &str,
    scenario_id: &str,
    compiler_version: &str,
) -> Result<i32, ScriptLangError> {
    if matches!(
        boundary.event,
        BoundaryEvent::Choices | BoundaryEvent::Input
    ) {
        save_engine_state(Path::new(state_out), engine, scenario_id, compiler_version)?;
        emit_boundary(boundary, Some(state_out.to_string()));
        return Ok(0);
    }

    emit_boundary(boundary, None);
    Ok(0)
}

#[cfg(test)]
mod session_ops_tests {
    use super::*;
    use crate::cli_test_support::*;
    use crate::run_to_boundary;

    #[test]
    fn session_helpers_cover_create_save_load_and_emit_paths() {
        let scripts_dir = example_scripts_dir("06-snapshot-flow");
        let scenario =
            crate::load_source_by_scripts_dir(&scripts_dir, "main").expect("scenario should load");

        let mut engine =
            create_engine_for_scenario(&scenario, "main").expect("engine should build");
        let boundary = run_to_boundary(&mut engine).expect("boundary should resolve");
        let state_file = temp_path("session-ops-state.json");
        save_engine_state(&state_file, &engine, &scenario.id, DEFAULT_COMPILER_VERSION)
            .expect("state save should pass");

        let (_loaded_scenario, state, mut resumed) =
            load_engine_from_state_for_ref(&state_file).expect("state ref load should pass");
        assert_eq!(state.scenario_id, scenario.id);
        assert!(matches!(
            resumed.next_output().expect("resume next should work"),
            sl_core::EngineOutput::Choices { .. }
        ));

        let (_state, mut resumed_for_scenario) =
            load_engine_from_state_for_scenario(&state_file, &scenario)
                .expect("state scenario load should pass");
        assert!(matches!(
            resumed_for_scenario
                .next_output()
                .expect("resume next should work"),
            sl_core::EngineOutput::Choices { .. }
        ));

        let emit_code = emit_boundary_with_saved_state(
            &engine,
            boundary,
            state_file.to_string_lossy().as_ref(),
            &scenario.id,
            DEFAULT_COMPILER_VERSION,
        )
        .expect("emit with save should pass");
        assert_eq!(emit_code, 0);
    }

    #[test]
    fn load_engine_from_state_for_scenario_rejects_mismatch() {
        let scenario_path = example_scripts_dir("06-snapshot-flow");
        let other_path = example_scripts_dir("16-input-name");
        let scenario =
            crate::load_source_by_scripts_dir(&scenario_path, "main").expect("scenario load");
        let other = crate::load_source_by_scripts_dir(&other_path, "main").expect("other load");

        let mut engine = create_engine_for_scenario(&other, "main").expect("engine build");
        let _ = run_to_boundary(&mut engine).expect("boundary");
        let state_file = temp_path("session-ops-mismatch-state.json");
        save_engine_state(&state_file, &engine, &other.id, DEFAULT_COMPILER_VERSION)
            .expect("state save");

        let error = match load_engine_from_state_for_scenario(&state_file, &scenario) {
            Ok(_) => panic!("mismatch should fail"),
            Err(error) => error,
        };
        assert_eq!(error.code, "TUI_STATE_SCENARIO_MISMATCH");
    }
}
