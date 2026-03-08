use sl_api::ScriptLangError;
use std::fmt::Display;

fn map_error(code: &'static str, error: impl Display) -> ScriptLangError {
    ScriptLangError::new(code, error.to_string())
}

fn hint_for_error(code: &str, message: &str) -> Option<&'static str> {
    if code == "XML_PARSE_ERROR" && message.contains("invalid name token") {
        return Some(
            "Hint: ScriptLang expressions no longer use XML escape operators. Write LT/LTE/AND instead of <, <=, &&. In XML attributes use 'text'; in <code>/<function>/<var>/<temp> bodies use \"text\".",
        );
    }

    if code.starts_with("RHAI_PREPROCESS_FORBIDDEN_")
        || code == "RHAI_PREPROCESS_STRING_UNTERMINATED"
    {
        return Some(
            "Hint: use ScriptLang expr keywords LT/LTE/AND. In XML attributes, write strings with single quotes like 'text'; in <code>/<function>/<var>/<temp> bodies, use double quotes like \"text\". Raw <, <=, && are forbidden.",
        );
    }

    if code == "TYPE_UNKNOWN" && message.contains("Unknown custom type") {
        return Some(
            "Hint: custom types are visible by import-closure, not auto inheritance. Add the required `<!-- import ... from ... -->` directive in each module that references the type.",
        );
    }

    if message.contains("Data type incorrect: f64 (expecting i64)") {
        return Some(
            "Hint: this can indicate a runtime int-index stability bug on some paths (for example ref:int across call + array index). If this script should be int-safe, upgrade to a version containing the fix or report a minimal repro.",
        );
    }

    None
}

fn with_hint(error: ScriptLangError) -> ScriptLangError {
    let mut message = error.message;
    if let Some(hint) = hint_for_error(&error.code, &message) {
        message.push('\n');
        message.push_str(hint);
    }
    ScriptLangError::new(error.code, message)
}

pub(crate) fn emit_error(error: ScriptLangError) -> i32 {
    let error = with_hint(error);
    println!("RESULT:ERROR");
    println!("ERROR_CODE:{}", error.code);
    println!(
        "ERROR_MSG_JSON:{}",
        serde_json::Value::String(error.message)
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

    #[test]
    fn with_hint_adds_xml_escape_suggestion() {
        let enriched = with_hint(ScriptLangError::new(
            "XML_PARSE_ERROR",
            "invalid name token at 1:23",
        ));
        assert!(enriched.message.contains("Write LT/LTE/AND"));
        assert!(enriched.message.contains("XML attributes use 'text'"));
    }

    #[test]
    fn with_hint_adds_type_visibility_suggestion() {
        let enriched = with_hint(ScriptLangError::new(
            "TYPE_UNKNOWN",
            "Unknown custom type \"game.WorldState\"",
        ));
        assert!(enriched.message.contains("not auto inheritance"));
        assert!(enriched.message.contains("`<!-- import ... from ... -->`"));
    }

    #[test]
    fn with_hint_adds_runtime_numeric_drift_suggestion() {
        let enriched = with_hint(ScriptLangError::new(
            "RUNTIME_ERROR",
            "Data type incorrect: f64 (expecting i64)",
        ));
        assert!(enriched.message.contains("runtime int-index stability bug"));
        assert!(enriched.message.contains("report a minimal repro"));
    }

    #[test]
    fn with_hint_keeps_message_when_no_rule_matches() {
        let enriched = with_hint(ScriptLangError::new("ANY", "plain"));
        assert_eq!(enriched.message, "plain");
    }

    #[test]
    fn with_hint_adds_preprocess_suggestion() {
        let enriched = with_hint(ScriptLangError::new("RHAI_PREPROCESS_FORBIDDEN_AND", "bad"));
        assert!(enriched.message.contains("LT/LTE/AND"));
        assert!(enriched.message.contains("XML attributes"));
        assert!(enriched.message.contains("<code>/<function>/<var>/<temp>"));
    }
}
