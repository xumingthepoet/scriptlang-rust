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
