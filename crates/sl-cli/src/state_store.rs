use std::fs;
use std::path::Path;

use sl_core::ScriptLangError;

use crate::{
    map_cli_state_invalid, map_cli_state_read, map_cli_state_write, PlayerStateV3,
    PLAYER_STATE_SCHEMA,
};

pub(crate) fn save_player_state(path: &Path, state: &PlayerStateV3) -> Result<(), ScriptLangError> {
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    fs::create_dir_all(parent).map_err(map_cli_state_write)?;

    let payload = serde_json::to_string(state).expect("player state should serialize");
    fs::write(path, payload).map_err(map_cli_state_write)
}

pub(crate) fn load_player_state(path: &Path) -> Result<PlayerStateV3, ScriptLangError> {
    if !path.exists() {
        return Err(ScriptLangError::new(
            "CLI_STATE_NOT_FOUND",
            format!("State file does not exist: {}", path.display()),
        ));
    }

    let raw = fs::read_to_string(path).map_err(map_cli_state_read)?;

    let state: PlayerStateV3 = serde_json::from_str(&raw).map_err(map_cli_state_invalid)?;

    if state.schema_version != PLAYER_STATE_SCHEMA {
        return Err(ScriptLangError::new(
            "CLI_STATE_SCHEMA",
            format!("Unsupported player state schema: {}", state.schema_version),
        ));
    }

    Ok(state)
}
