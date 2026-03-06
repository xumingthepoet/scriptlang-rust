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
        if !(path_str.ends_with(".script.xml")
            || path_str.ends_with(".defs.xml")
            || path_str.ends_with(".module.xml")
            || path_str.ends_with(".json"))
        {
            continue;
        }

        let relative = path
            .strip_prefix(example_dir)
            .expect("walkdir path should start with example dir")
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

#[cfg(test)]
mod source_tests {
    use super::*;

    use std::time::{SystemTime, UNIX_EPOCH};
    #[cfg(unix)]
    use std::{fs::Permissions, os::unix::fs::PermissionsExt};

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should move forward")
            .as_nanos();
        std::env::temp_dir().join(format!("sl-test-example-{}-{}", name, nanos))
    }

    fn write_file(path: &Path, content: &str) {
        let parent = path.parent().expect("path should have parent");
        fs::create_dir_all(parent).expect("parent dir should be created");
        fs::write(path, content).expect("file should be written");
    }

    #[test]
    fn read_scripts_xml_from_dir_collects_supported_extensions() {
        let root = temp_dir("scripts");
        fs::create_dir_all(&root).expect("root should be created");

        write_file(
            &root.join("main.script.xml"),
            "<script name=\"main\"><text>x</text></script>",
        );
        write_file(
            &root.join("shared.defs.xml"),
            "<defs name=\"shared\"></defs>",
        );
        write_file(
            &root.join("battle.module.xml"),
            "<module name=\"battle\"></module>",
        );
        write_file(&root.join("game.json"), "{}");
        write_file(&root.join("ignore.txt"), "skip");

        let files = read_scripts_xml_from_dir(&root).expect("scan should pass");
        assert_eq!(files.len(), 4);
        assert!(files.contains_key("main.script.xml"));
        assert!(files.contains_key("shared.defs.xml"));
        assert!(files.contains_key("battle.module.xml"));
        assert!(files.contains_key("game.json"));
    }

    #[test]
    fn read_scripts_xml_from_dir_fails_when_no_supported_files() {
        let root = temp_dir("empty-scripts");
        fs::create_dir_all(&root).expect("root should be created");
        write_file(&root.join("ignore.txt"), "skip");

        let error = read_scripts_xml_from_dir(&root).expect_err("empty source should fail");
        assert!(matches!(error, SlTestExampleError::SourceEmpty { .. }));
    }

    #[cfg(unix)]
    #[test]
    fn read_scripts_xml_from_dir_reports_read_errors() {
        let root = temp_dir("read-error");
        fs::create_dir_all(&root).expect("root should be created");
        let script_path = root.join("main.script.xml");
        write_file(
            &script_path,
            "<script name=\"main\"><text>x</text></script>",
        );

        let mut perms = fs::metadata(&script_path)
            .expect("metadata should exist")
            .permissions();
        perms.set_mode(0o000);
        fs::set_permissions(&script_path, perms).expect("permissions should update");

        let error = read_scripts_xml_from_dir(&root).expect_err("read error should surface");

        fs::set_permissions(&script_path, Permissions::from_mode(0o644))
            .expect("permissions should reset");
        assert!(matches!(error, SlTestExampleError::ReadFile { .. }));
    }

    #[test]
    fn read_test_case_parses_valid_json() {
        let root = temp_dir("case-ok");
        fs::create_dir_all(&root).expect("root should be created");
        let case_path = root.join("testcase.json");
        write_file(
            &case_path,
            r#"{
  "schemaVersion":"sl-tool-case",
  "entryScript":"main",
  "actions":[],
  "expectedEvents":[{"kind":"end"}]
}"#,
        );

        let parsed = read_test_case(&case_path).expect("case should parse");
        assert_eq!(parsed.schema_version, TESTCASE_SCHEMA);
        assert_eq!(parsed.entry_script, "main");
        assert_eq!(parsed.expected_events.len(), 1);
    }

    #[test]
    fn read_test_case_reports_read_error() {
        let root = temp_dir("case-read-error");
        fs::create_dir_all(&root).expect("root should be created");
        let missing_path = root.join("missing.json");
        let error = read_test_case(&missing_path).expect_err("missing case should fail");
        assert!(matches!(error, SlTestExampleError::ReadFile { .. }));
    }

    #[test]
    fn read_test_case_reports_parse_and_schema_errors() {
        let root = temp_dir("case-errors");
        fs::create_dir_all(&root).expect("root should be created");

        let bad_json_path = root.join("bad.json");
        write_file(&bad_json_path, "{");
        let parse_error = read_test_case(&bad_json_path).expect_err("parse should fail");
        assert!(matches!(parse_error, SlTestExampleError::ParseCase { .. }));

        let bad_schema_path = root.join("bad-schema.json");
        write_file(
            &bad_schema_path,
            r#"{
  "schemaVersion":"v0",
  "actions":[],
  "expectedEvents":[{"kind":"end"}]
}"#,
        );
        let schema_error = read_test_case(&bad_schema_path).expect_err("schema should fail");
        assert!(matches!(
            schema_error,
            SlTestExampleError::InvalidSchemaVersion { .. }
        ));
    }
}
