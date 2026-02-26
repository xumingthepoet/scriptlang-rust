use sl_core::ScriptLangError;
use std::fmt::Display;

fn map_error(code: &'static str, error: impl Display) -> ScriptLangError {
    ScriptLangError::new(code, error.to_string())
}

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
    map_error("TUI_IO", error)
}

pub(crate) fn map_cli_source_path(error: std::io::Error) -> ScriptLangError {
    map_error("CLI_SOURCE_PATH", error)
}

pub(crate) fn map_cli_source_scan(error: std::path::StripPrefixError) -> ScriptLangError {
    map_error("CLI_SOURCE_SCAN", error)
}

pub(crate) fn map_cli_source_read(error: std::io::Error) -> ScriptLangError {
    map_error("CLI_SOURCE_READ", error)
}

pub(crate) fn map_cli_state_write(error: std::io::Error) -> ScriptLangError {
    map_error("CLI_STATE_WRITE", error)
}

pub(crate) fn map_cli_state_read(error: std::io::Error) -> ScriptLangError {
    map_error("CLI_STATE_READ", error)
}

pub(crate) fn map_cli_state_invalid(error: serde_json::Error) -> ScriptLangError {
    map_error("CLI_STATE_INVALID", error)
}

#[cfg(test)]
mod error_map_tests {
    use super::*;

    #[test]
    fn emit_error_returns_non_zero_exit_code() {
        let code = emit_error(ScriptLangError::new("ERR", "failed"));
        assert_eq!(code, 1);
    }

    #[test]
    fn mapping_helpers_keep_error_codes() {
        assert_eq!(map_tui_io(std::io::Error::other("io")).code, "TUI_IO");
        assert_eq!(
            map_cli_source_path(std::io::Error::other("path")).code,
            "CLI_SOURCE_PATH"
        );

        let strip_error = std::path::Path::new("/a")
            .strip_prefix("/b")
            .expect_err("strip prefix");
        assert_eq!(map_cli_source_scan(strip_error).code, "CLI_SOURCE_SCAN");

        assert_eq!(
            map_cli_source_read(std::io::Error::other("read")).code,
            "CLI_SOURCE_READ"
        );
        assert_eq!(
            map_cli_state_write(std::io::Error::other("write")).code,
            "CLI_STATE_WRITE"
        );
        assert_eq!(
            map_cli_state_read(std::io::Error::other("read")).code,
            "CLI_STATE_READ"
        );

        let invalid = serde_json::from_str::<serde_json::Value>("{").expect_err("invalid json");
        assert_eq!(map_cli_state_invalid(invalid).code, "CLI_STATE_INVALID");
    }
}
