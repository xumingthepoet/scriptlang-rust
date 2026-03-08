use crate::*;

pub(crate) fn with_file_context_shared(error: ScriptLangError, file_path: &str) -> ScriptLangError {
    let message = format!("In file \"{}\": {}", file_path, error.message);
    ScriptLangError::with_span(
        error.code,
        message,
        error.span.unwrap_or(SourceSpan::synthetic()),
    )
}
