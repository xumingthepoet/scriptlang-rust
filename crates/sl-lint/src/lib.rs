use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::Path;

use clap::Parser;
use sl_compiler::compile_project_bundle_from_xml_map;
use sl_core::ScriptLangError;
use walkdir::WalkDir;

mod lint;

#[derive(Debug, Parser)]
#[command(name = "sl-lint")]
#[command(about = "ScriptLang lint tool")]
struct Cli {
    #[arg(long = "scripts-dir")]
    scripts_dir: String,
    #[arg(long = "entry-script", default_value = "main.main")]
    entry_script: String,
}

pub fn run_from_args<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(err) => {
            let code = err.exit_code();
            let _ = err.print();
            return code;
        }
    };

    match run(cli) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("{}: {}", err.code, err.message);
            1
        }
    }
}

fn run(cli: Cli) -> Result<i32, ScriptLangError> {
    let scripts = read_scripts_xml_from_dir(Path::new(&cli.scripts_dir))?;
    let bundle = compile_project_bundle_from_xml_map(&scripts)?;
    let report = lint::run_lint(&scripts, &bundle, &cli.entry_script);
    lint::render_report(&report);
    Ok(if report.should_fail() { 1 } else { 0 })
}

fn read_scripts_xml_from_dir(root: &Path) -> Result<BTreeMap<String, String>, ScriptLangError> {
    if !root.exists() {
        return Err(ScriptLangError::new(
            "LINT_SOURCE_NOT_FOUND",
            format!("scripts-dir does not exist: {}", root.display()),
        ));
    }
    if !root.is_dir() {
        return Err(ScriptLangError::new(
            "LINT_SOURCE_NOT_DIR",
            format!("scripts-dir is not a directory: {}", root.display()),
        ));
    }

    let mut scripts = BTreeMap::new();
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if !path.to_string_lossy().ends_with(".xml") {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|error| ScriptLangError::new("LINT_SOURCE_SCAN", error.to_string()))?
            .to_string_lossy()
            .replace('\\', "/");
        let content = std::fs::read_to_string(path)
            .map_err(|error| ScriptLangError::new("LINT_SOURCE_READ", error.to_string()))?;
        scripts.insert(relative, content);
    }

    if scripts.is_empty() {
        return Err(ScriptLangError::new(
            "LINT_SOURCE_EMPTY",
            format!("No .xml files under {}", root.display()),
        ));
    }

    Ok(scripts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sl_test_example::example_dir;

    #[test]
    fn run_from_args_help_returns_zero() {
        let code = run_from_args(["sl-lint", "--help"]);
        assert_eq!(code, 0);
    }

    #[test]
    fn lint_can_run_against_sl_test_example_fixture() {
        let dir = example_dir("01-text-code");
        let scripts = read_scripts_xml_from_dir(&dir).expect("example scripts should load");
        let bundle = compile_project_bundle_from_xml_map(&scripts).expect("bundle should compile");
        let report = crate::lint::run_lint(&scripts, &bundle, "main.main");
        assert!(report.diagnostics.iter().all(|item| !item.code.is_empty()));
    }

    #[test]
    fn lint_tracks_alias_and_function_script_literals() {
        let dir = example_dir("37-lint-function-script-literal");
        let scripts = read_scripts_xml_from_dir(&dir).expect("example scripts should load");
        let bundle = compile_project_bundle_from_xml_map(&scripts).expect("bundle should compile");
        let report = crate::lint::run_lint(&scripts, &bundle, "main.main");
        assert!(
            report
                .diagnostics
                .iter()
                .all(|item| item.code != "unused-import"),
            "unexpected unused-import diagnostics: {:?}",
            report.diagnostics
        );
        assert!(
            report
                .diagnostics
                .iter()
                .all(|item| item.code != "unused-script"),
            "unexpected unused-script diagnostics: {:?}",
            report.diagnostics
        );
        assert!(
            report
                .diagnostics
                .iter()
                .all(|item| item.code != "unused-module"),
            "unexpected unused-module diagnostics: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn lint_does_not_flag_root_const_as_unused_when_read_in_submodule_function_path() {
        let dir = example_dir("48-sub-module-complex");
        let scripts = read_scripts_xml_from_dir(&dir).expect("example scripts should load");
        let bundle = compile_project_bundle_from_xml_map(&scripts).expect("bundle should compile");
        let report = crate::lint::run_lint(&scripts, &bundle, "root.main");
        assert!(
            !report.diagnostics.iter().any(|item| {
                item.code == "unused-module-const" && item.message.contains("m.vals")
            }),
            "unexpected unused-module-const for m.vals: {:?}",
            report.diagnostics
        );
    }
}
