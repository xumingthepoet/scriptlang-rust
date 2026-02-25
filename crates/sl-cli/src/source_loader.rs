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
