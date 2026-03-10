use sl_core::SourceSpan;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LintLevel {
    Warning,
}

impl LintLevel {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Warning => "warning",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LintDiagnostic {
    pub(crate) level: LintLevel,
    pub(crate) code: &'static str,
    pub(crate) file: String,
    pub(crate) span: Option<SourceSpan>,
    pub(crate) message: String,
    pub(crate) help: Option<String>,
}

impl LintDiagnostic {
    pub(crate) fn warning(
        code: &'static str,
        file: impl Into<String>,
        span: Option<SourceSpan>,
        message: impl Into<String>,
        help: Option<String>,
    ) -> Self {
        Self {
            level: LintLevel::Warning,
            code,
            file: file.into(),
            span,
            message: message.into(),
            help,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct LintSummary {
    pub(crate) errors: usize,
    pub(crate) warnings: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct LintReport {
    pub(crate) diagnostics: Vec<LintDiagnostic>,
    pub(crate) summary: LintSummary,
}

impl LintReport {
    pub(crate) fn new(mut diagnostics: Vec<LintDiagnostic>) -> Self {
        diagnostics.sort_by(|a, b| {
            let a_line = a
                .span
                .as_ref()
                .map(|s| (s.start.line, s.start.column))
                .unwrap_or((usize::MAX, usize::MAX));
            let b_line = b
                .span
                .as_ref()
                .map(|s| (s.start.line, s.start.column))
                .unwrap_or((usize::MAX, usize::MAX));
            a.file
                .cmp(&b.file)
                .then_with(|| a_line.cmp(&b_line))
                .then_with(|| a.code.cmp(b.code))
                .then_with(|| a.message.cmp(&b.message))
        });
        let summary = diagnostics
            .iter()
            .fold(LintSummary::default(), |mut acc, item| {
                match item.level {
                    LintLevel::Warning => acc.warnings += 1,
                }
                acc
            });
        Self {
            diagnostics,
            summary,
        }
    }

    pub(crate) fn should_fail(&self) -> bool {
        self.summary.errors + self.summary.warnings > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sl_core::{SourceLocation, SourceSpan};

    #[test]
    fn lint_level_to_string() {
        assert_eq!(LintLevel::Warning.as_str(), "warning");
    }

    #[test]
    fn lint_report_sort_and_summary() {
        let span1 = SourceSpan {
            start: SourceLocation { line: 1, column: 1 },
            end: SourceLocation { line: 1, column: 2 },
        };
        let span2 = SourceSpan {
            start: SourceLocation { line: 2, column: 1 },
            end: SourceLocation { line: 2, column: 2 },
        };
        let report = LintReport::new(vec![
            LintDiagnostic {
                level: LintLevel::Warning,
                code: "L2",
                file: "b.xml".to_string(),
                span: Some(span2),
                message: "b".to_string(),
                help: None,
            },
            LintDiagnostic::warning("L1", "a.xml", Some(span1), "a", None),
        ]);
        assert_eq!(report.summary.errors, 0);
        assert_eq!(report.summary.warnings, 2);
        assert_eq!(report.diagnostics[0].file, "a.xml");
        assert!(report.should_fail());
    }
}
