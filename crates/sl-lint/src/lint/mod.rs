use std::collections::BTreeMap;

use sl_compiler::CompileProjectBundleResult;

mod collector;
mod diagnostic;
mod render;
mod rules;

pub(crate) use diagnostic::LintReport;
pub(crate) use render::render_report;

pub(crate) fn run_lint(
    scripts_xml: &BTreeMap<String, String>,
    bundle: &CompileProjectBundleResult,
    entry_script: &str,
) -> LintReport {
    let context = collector::collect_context(scripts_xml, bundle, entry_script);
    let diagnostics = rules::run_rules(&context);
    LintReport::new(diagnostics)
}
