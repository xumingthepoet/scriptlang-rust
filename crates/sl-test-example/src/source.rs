use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use walkdir::WalkDir;

use crate::{SlTestExampleError, TestCase, TESTCASE_SCHEMA};

pub fn read_scripts_xml_from_dir(
    example_dir: &Path,
) -> Result<BTreeMap<String, String>, SlTestExampleError> {
    let mut scripts = BTreeMap::new();

    for entry in WalkDir::new(example_dir)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        let path_str = path.to_string_lossy();
        if !path_str.ends_with(".xml") {
            continue;
        }

        let relative = path
            .strip_prefix(example_dir)
            .map_err(|source| SlTestExampleError::PathStrip {
                path: path.to_path_buf(),
                source,
            })?
            .to_string_lossy()
            .replace('\\', "/");

        let content = fs::read_to_string(path).map_err(|source| SlTestExampleError::ReadFile {
            path: path.to_path_buf(),
            source,
        })?;
        scripts.insert(relative, content);
    }

    if scripts.is_empty() {
        return Err(SlTestExampleError::SourceEmpty {
            path: example_dir.to_path_buf(),
        });
    }

    Ok(scripts)
}

pub fn read_test_case(case_path: &Path) -> Result<TestCase, SlTestExampleError> {
    let raw = fs::read_to_string(case_path).map_err(|source| SlTestExampleError::ReadFile {
        path: case_path.to_path_buf(),
        source,
    })?;
    let parsed: TestCase =
        serde_json::from_str(&raw).map_err(|source| SlTestExampleError::ParseCase {
            path: case_path.to_path_buf(),
            source,
        })?;

    if parsed.schema_version != TESTCASE_SCHEMA {
        return Err(SlTestExampleError::InvalidSchemaVersion {
            expected: TESTCASE_SCHEMA.to_string(),
            found: parsed.schema_version,
        });
    }

    Ok(parsed)
}
