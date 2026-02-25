use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use sl_core::ScriptLangError;
use walkdir::WalkDir;

use crate::{map_cli_source_path, map_cli_source_read, map_cli_source_scan, LoadedScenario};

pub(crate) fn load_source_by_scripts_dir(
    scripts_dir: &str,
    entry_script: &str,
) -> Result<LoadedScenario, ScriptLangError> {
    let scripts_root = resolve_scripts_dir(scripts_dir)?;
    let scripts_xml = read_scripts_xml_from_dir(&scripts_root)?;
    let scenario_id = make_scripts_dir_scenario_id(&scripts_root);
    let title = format!(
        "Scripts {}",
        scripts_root
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("unknown")
    );

    Ok(LoadedScenario {
        id: scenario_id,
        title,
        scripts_xml,
        entry_script: entry_script.to_string(),
    })
}

pub(crate) fn load_source_by_ref(scenario_ref: &str) -> Result<LoadedScenario, ScriptLangError> {
    let prefix = "scripts-dir:";
    if !scenario_ref.starts_with(prefix) {
        return Err(ScriptLangError::new(
            "CLI_SOURCE_REF_INVALID",
            format!("Unsupported scenario ref: {}", scenario_ref),
        ));
    }

    let raw = scenario_ref.trim_start_matches(prefix);
    load_source_by_scripts_dir(raw, "main")
}

pub(crate) fn resolve_scripts_dir(scripts_dir: &str) -> Result<PathBuf, ScriptLangError> {
    let path = PathBuf::from(scripts_dir);
    let absolute = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .map_err(map_cli_source_path)?
            .join(path)
    };

    if !absolute.exists() {
        return Err(ScriptLangError::new(
            "CLI_SOURCE_NOT_FOUND",
            format!("scripts-dir does not exist: {}", absolute.display()),
        ));
    }

    if !absolute.is_dir() {
        return Err(ScriptLangError::new(
            "CLI_SOURCE_NOT_DIR",
            format!("scripts-dir is not a directory: {}", absolute.display()),
        ));
    }

    Ok(absolute)
}

pub(crate) fn read_scripts_xml_from_dir(
    scripts_dir: &Path,
) -> Result<BTreeMap<String, String>, ScriptLangError> {
    let mut scripts = BTreeMap::new();

    for entry in WalkDir::new(scripts_dir)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        let path_str = path.to_string_lossy();

        if !(path_str.ends_with(".script.xml")
            || path_str.ends_with(".defs.xml")
            || path_str.ends_with(".json"))
        {
            continue;
        }

        let relative = path
            .strip_prefix(scripts_dir)
            .map_err(map_cli_source_scan)?
            .to_string_lossy()
            .replace('\\', "/");

        let content = fs::read_to_string(path).map_err(map_cli_source_read)?;
        scripts.insert(relative, content);
    }

    if scripts.is_empty() {
        return Err(ScriptLangError::new(
            "CLI_SOURCE_EMPTY",
            format!(
                "No .script.xml/.defs.xml/.json files under {}",
                scripts_dir.display()
            ),
        ));
    }

    Ok(scripts)
}

pub(crate) fn make_scripts_dir_scenario_id(scripts_dir: &Path) -> String {
    format!("scripts-dir:{}", scripts_dir.display())
}

#[cfg(test)]
mod source_loader_tests {
    use super::*;
    use crate::cli_test_support::*;

    #[test]
    fn load_source_by_ref_validates_ref_prefix() {
        let error = load_source_by_ref("unknown:main").expect_err("invalid ref should fail");
        assert_eq!(error.code, "CLI_SOURCE_REF_INVALID");
    }

    #[test]
    fn resolve_scripts_dir_validates_existence_and_directory() {
        let missing = temp_path("missing-dir");
        let missing_err = resolve_scripts_dir(missing.to_string_lossy().as_ref())
            .expect_err("missing path should fail");
        assert_eq!(missing_err.code, "CLI_SOURCE_NOT_FOUND");

        let file_path = temp_path("plain-file");
        write_file(&file_path, "x");
        let file_err = resolve_scripts_dir(file_path.to_string_lossy().as_ref())
            .expect_err("file path should fail");
        assert_eq!(file_err.code, "CLI_SOURCE_NOT_DIR");

        let cwd = std::env::current_dir().expect("cwd");
        let relative_root = temp_path("relative-scripts-dir");
        fs::create_dir_all(&relative_root).expect("root");
        let relative_child = relative_root.join("child");
        fs::create_dir_all(&relative_child).expect("child");
        write_file(
            &relative_child.join("main.script.xml"),
            "<script name=\"main\"></script>",
        );

        std::env::set_current_dir(&relative_root).expect("switch cwd");
        let resolved = resolve_scripts_dir("child").expect("relative dir should resolve");
        assert!(resolved.ends_with("child"));
        std::env::set_current_dir(cwd).expect("restore cwd");
    }

    #[test]
    fn read_scripts_xml_from_dir_filters_supported_extensions() {
        let root = temp_path("scripts-dir");
        fs::create_dir_all(&root).expect("root should be created");
        write_file(
            &root.join("main.script.xml"),
            "<script name=\"main\"></script>",
        );
        write_file(&root.join("defs.defs.xml"), "<defs name=\"d\"></defs>");
        write_file(&root.join("data.json"), "{\"ok\":true}");
        write_file(&root.join("skip.txt"), "ignored");

        let scripts = read_scripts_xml_from_dir(&root).expect("scan should pass");
        assert_eq!(scripts.len(), 3);
        assert!(scripts.contains_key("main.script.xml"));
        assert!(scripts.contains_key("defs.defs.xml"));
        assert!(scripts.contains_key("data.json"));
    }

    #[test]
    fn read_scripts_xml_from_dir_errors_when_no_source_files() {
        let root = temp_path("empty-scripts-dir");
        fs::create_dir_all(&root).expect("root should be created");
        write_file(&root.join("readme.txt"), "not source");

        let error =
            read_scripts_xml_from_dir(&root).expect_err("empty source set should return error");
        assert_eq!(error.code, "CLI_SOURCE_EMPTY");
    }

    #[test]
    fn load_source_by_scripts_dir_works() {
        let root = temp_path("scripts-dir-test");
        fs::create_dir_all(&root).expect("root");
        write_file(
            &root.join("main.script.xml"),
            r#"<script name="main"><text>Hello</text></script>"#,
        );

        let loaded =
            load_source_by_scripts_dir(&root.to_string_lossy(), "main").expect("load should pass");
        assert!(loaded.id.starts_with("scripts-dir:"));
        assert_eq!(loaded.entry_script, "main");
        assert!(loaded.scripts_xml.contains_key("main.script.xml"));
    }

    #[test]
    fn load_source_by_scripts_dir_with_nested_entry() {
        let root = temp_path("scripts-dir-nested");
        fs::create_dir_all(&root).expect("root");
        write_file(
            &root.join("game.script.xml"),
            r#"<script name="game"><text>Game</text></script>"#,
        );

        let loaded =
            load_source_by_scripts_dir(&root.to_string_lossy(), "game").expect("load should pass");
        assert_eq!(loaded.entry_script, "game");
    }

    #[test]
    fn load_source_by_ref_validates_prefix() {
        let error = load_source_by_ref("invalid").expect_err("no prefix should fail");
        assert_eq!(error.code, "CLI_SOURCE_REF_INVALID");

        let error = load_source_by_ref("").expect_err("empty ref should fail");
        assert_eq!(error.code, "CLI_SOURCE_REF_INVALID");
    }

    #[test]
    fn make_scripts_dir_scenario_id_generates_consistent_ids() {
        let root = temp_path("scenario-id-test");
        fs::create_dir_all(&root).expect("root");

        let id1 = make_scripts_dir_scenario_id(&root);
        let id2 = make_scripts_dir_scenario_id(&root);
        assert_eq!(id1, id2);
        assert!(id1.starts_with("scripts-dir:"));
    }
}
