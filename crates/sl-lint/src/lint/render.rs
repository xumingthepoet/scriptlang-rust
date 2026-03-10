use crate::lint::diagnostic::LintReport;

pub(crate) fn render_report(report: &LintReport) {
    for diag in &report.diagnostics {
        let (line, col) = diag
            .span
            .as_ref()
            .map(|span| (span.start.line, span.start.column))
            .unwrap_or((0, 0));
        if line == 0 {
            println!(
                "[{}] {} {} {}",
                diag.level.as_str(),
                diag.code,
                diag.file,
                diag.message
            );
        } else {
            println!(
                "[{}] {} {}:{}:{} {}",
                diag.level.as_str(),
                diag.code,
                diag.file,
                line,
                col,
                diag.message
            );
        }
        if let Some(help) = &diag.help {
            println!("help: {}", help);
        }
    }

    println!(
        "{} errors, {} warnings",
        report.summary.errors, report.summary.warnings
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lint::diagnostic::{LintDiagnostic, LintReport};

    #[test]
    fn render_report_runs_with_empty_report() {
        let report = LintReport::default();
        render_report(&report);
    }

    #[test]
    fn render_report_runs_with_help_line() {
        let report = LintReport::new(vec![LintDiagnostic::warning(
            "LINT_TEST",
            "main.xml",
            None,
            "message",
            Some("try fix".to_string()),
        )]);
        render_report(&report);
    }
}
