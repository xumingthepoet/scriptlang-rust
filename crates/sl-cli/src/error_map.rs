use sl_core::ScriptLangError;

pub(crate) fn emit_error(error: ScriptLangError) -> i32 {
    println!("RESULT:ERROR");
    println!("ERROR_CODE:{}", error.code);
    println!(
        "ERROR_MSG_JSON:{}",
        serde_json::to_string(&error.message).expect("string json")
    );
    1
}

pub(crate) fn map_tui_io(error: std::io::Error) -> ScriptLangError {
    ScriptLangError::new("TUI_IO", error.to_string())
}

pub(crate) fn map_cli_source_path(error: std::io::Error) -> ScriptLangError {
    ScriptLangError::new("CLI_SOURCE_PATH", error.to_string())
}

pub(crate) fn map_cli_source_scan(error: std::path::StripPrefixError) -> ScriptLangError {
    ScriptLangError::new("CLI_SOURCE_SCAN", error.to_string())
}

pub(crate) fn map_cli_source_read(error: std::io::Error) -> ScriptLangError {
    ScriptLangError::new("CLI_SOURCE_READ", error.to_string())
}

pub(crate) fn map_cli_state_write(error: std::io::Error) -> ScriptLangError {
    ScriptLangError::new("CLI_STATE_WRITE", error.to_string())
}

pub(crate) fn map_cli_state_read(error: std::io::Error) -> ScriptLangError {
    ScriptLangError::new("CLI_STATE_READ", error.to_string())
}

pub(crate) fn map_cli_state_invalid(error: serde_json::Error) -> ScriptLangError {
    ScriptLangError::new("CLI_STATE_INVALID", error.to_string())
}
