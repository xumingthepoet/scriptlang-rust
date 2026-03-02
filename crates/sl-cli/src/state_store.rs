use std::fs;
use std::path::Path;

use sl_api::ScriptLangError;
use sl_api::SnapshotV3;

use crate::{
    map_cli_state_invalid, map_cli_state_read, map_cli_state_write, PlayerRandomMode,
    PlayerStateV4, PLAYER_STATE_SCHEMA,
};

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlayerStateLegacyV3 {
    scenario_id: String,
    compiler_version: String,
    snapshot: SnapshotV3,
}

pub(crate) fn save_player_state(path: &Path, state: &PlayerStateV4) -> Result<(), ScriptLangError> {
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    fs::create_dir_all(parent).map_err(map_cli_state_write)?;

    let payload = serde_json::to_string(state).expect("player state should serialize");
    fs::write(path, payload).map_err(map_cli_state_write)
}

pub(crate) fn load_player_state(path: &Path) -> Result<PlayerStateV4, ScriptLangError> {
    if !path.exists() {
        return Err(ScriptLangError::new(
            "CLI_STATE_NOT_FOUND",
            format!("State file does not exist: {}", path.display()),
        ));
    }

    let raw = fs::read_to_string(path).map_err(map_cli_state_read)?;

    let value: serde_json::Value = serde_json::from_str(&raw).map_err(map_cli_state_invalid)?;
    let schema_version = value
        .get("schemaVersion")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ScriptLangError::new("CLI_STATE_SCHEMA", "Missing state schemaVersion."))?;

    if schema_version == PLAYER_STATE_SCHEMA {
        let state: PlayerStateV4 = serde_json::from_value(value).map_err(map_cli_state_invalid)?;
        return Ok(state);
    }

    if schema_version == "player-state.v3" {
        let legacy: PlayerStateLegacyV3 =
            serde_json::from_value(value).map_err(map_cli_state_invalid)?;
        return Ok(PlayerStateV4 {
            schema_version: PLAYER_STATE_SCHEMA.to_string(),
            scenario_id: legacy.scenario_id,
            compiler_version: legacy.compiler_version,
            snapshot: legacy.snapshot.clone(),
            random_mode: PlayerRandomMode::Seeded,
            random_seed_state: Some(legacy.snapshot.rng_state),
            random_sequence: Vec::new(),
            random_sequence_index: None,
        });
    }

    Err(ScriptLangError::new(
        "CLI_STATE_SCHEMA",
        format!("Unsupported player state schema: {}", schema_version),
    ))
}

#[cfg(test)]
mod state_store_tests {
    use super::*;
    use crate::cli_test_support::*;
    use sl_api::DEFAULT_COMPILER_VERSION;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::Path;

    #[test]
    fn save_and_load_player_state_roundtrip_and_schema_validation() {
        let state_path = temp_path("player-state.json");
        let state = PlayerStateV4 {
            schema_version: PLAYER_STATE_SCHEMA.to_string(),
            scenario_id: "scripts-dir:/tmp/demo".to_string(),
            compiler_version: DEFAULT_COMPILER_VERSION.to_string(),
            snapshot: SnapshotV3 {
                schema_version: "snapshot.v3".to_string(),
                compiler_version: DEFAULT_COMPILER_VERSION.to_string(),
                runtime_frames: Vec::new(),
                rng_state: 1,
                pending_boundary: sl_api::PendingBoundaryV3::Choice {
                    node_id: "n1".to_string(),
                    items: Vec::new(),
                    prompt_text: None,
                },
                defs_globals: BTreeMap::new(),
                once_state_by_script: BTreeMap::new(),
            },
            random_mode: PlayerRandomMode::Seeded,
            random_seed_state: Some(1),
            random_sequence: Vec::new(),
            random_sequence_index: None,
        };
        save_player_state(&state_path, &state).expect("save should pass");
        let loaded = load_player_state(&state_path).expect("load should pass");
        assert_eq!(loaded.schema_version, PLAYER_STATE_SCHEMA);
        assert_eq!(loaded.scenario_id, state.scenario_id);

        let bad_path = temp_path("bad-player-state.json");
        let mut bad_json: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&state_path).expect("read state"))
                .expect("state json should parse");
        bad_json["schemaVersion"] = serde_json::Value::String("player-state.bad".to_string());
        write_file(
            &bad_path,
            &serde_json::to_string(&bad_json).expect("json should serialize"),
        );
        let error = load_player_state(&bad_path).expect_err("bad schema should fail");
        assert_eq!(error.code, "CLI_STATE_SCHEMA");

        let not_found = temp_path("missing-player-state.json");
        let error = load_player_state(&not_found).expect_err("missing file should fail");
        assert_eq!(error.code, "CLI_STATE_NOT_FOUND");

        let write_root_error =
            save_player_state(Path::new("/"), &state).expect_err("writing root should fail");
        assert_eq!(write_root_error.code, "CLI_STATE_WRITE");

        let v3_path = temp_path("legacy-player-state-v3.json");
        write_file(
            &v3_path,
            r#"{
  "schemaVersion":"player-state.v3",
  "scenarioId":"scripts-dir:/tmp/legacy",
  "compilerVersion":"player.v1",
  "snapshot":{
    "schemaVersion":"snapshot.v3",
    "compilerVersion":"player.v1",
    "runtimeFrames":[],
    "rngState":7,
    "pendingBoundary":{"kind":"Choice","nodeId":"n1","items":[],"promptText":null},
    "defsGlobals":{},
    "onceStateByScript":{}
  }
}"#,
        );
        let loaded_v3 = load_player_state(&v3_path).expect("legacy v3 should be accepted");
        assert_eq!(loaded_v3.schema_version, PLAYER_STATE_SCHEMA);
        assert_eq!(loaded_v3.random_mode, PlayerRandomMode::Seeded);
        assert_eq!(loaded_v3.random_seed_state, Some(7));
    }
}
