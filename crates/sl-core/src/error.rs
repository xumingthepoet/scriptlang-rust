use crate::types::SourceSpan;
use thiserror::Error;

#[derive(Debug, Error, Clone)]
#[error("{code}: {message}")]
pub struct ScriptLangError {
    pub code: String,
    pub message: String,
    pub span: Option<SourceSpan>,
}

impl ScriptLangError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            span: None,
        }
    }

    pub fn with_span(
        code: impl Into<String>,
        message: impl Into<String>,
        span: SourceSpan,
    ) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            span: Some(span),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_builds_error_without_span() {
        let error = ScriptLangError::new("E_CODE", "message");
        assert_eq!(error.code, "E_CODE");
        assert_eq!(error.message, "message");
        assert!(error.span.is_none());
        assert_eq!(format!("{}", error), "E_CODE: message");
    }

    #[test]
    fn with_span_builds_error_with_span() {
        let span = SourceSpan::synthetic();
        let error = ScriptLangError::with_span("E_SPAN", "has-span", span.clone());
        assert_eq!(error.code, "E_SPAN");
        assert_eq!(error.message, "has-span");
        assert_eq!(error.span, Some(span));
    }
}
