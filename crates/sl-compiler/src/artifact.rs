use std::fs;
use std::path::Path;

use crate::*;

pub const DEFAULT_COMPILER_VERSION: &str = "player";

pub fn compile_artifact_from_xml_map(
    xml_by_path: &BTreeMap<String, String>,
    entry_script: Option<String>,
) -> Result<CompiledProjectArtifact, ScriptLangError> {
    let CompileProjectBundleResult {
        scripts,
        global_json,
        defs_global_declarations,
        defs_global_init_order,
        defs_global_const_declarations,
        defs_global_const_init_order,
    } = compile_project_bundle_from_xml_map(xml_by_path)?;

    let entry_script = resolve_entry_script(&scripts, entry_script)?;

    Ok(CompiledProjectArtifact {
        schema_version: COMPILED_PROJECT_SCHEMA.to_string(),
        compiler_version: DEFAULT_COMPILER_VERSION.to_string(),
        entry_script,
        scripts,
        global_json,
        defs_global_declarations,
        defs_global_init_order,
        defs_global_const_declarations,
        defs_global_const_init_order,
    })
}

pub fn write_artifact_json(
    path: &Path,
    artifact: &CompiledProjectArtifact,
) -> Result<(), ScriptLangError> {
    if artifact.schema_version != COMPILED_PROJECT_SCHEMA {
        return Err(ScriptLangError::new(
            "ARTIFACT_SCHEMA_UNSUPPORTED",
            format!(
                "Unsupported compiled artifact schema \"{}\", expected \"{}\".",
                artifact.schema_version, COMPILED_PROJECT_SCHEMA
            ),
        ));
    }

    let encoded =
        serde_json::to_string_pretty(artifact).expect("compiled artifact should serialize");
    fs::write(path, encoded)
        .map_err(|error| ScriptLangError::new("ARTIFACT_IO_ERROR", error.to_string()))
}

pub fn read_artifact_json(path: &Path) -> Result<CompiledProjectArtifact, ScriptLangError> {
    let raw = fs::read_to_string(path)
        .map_err(|error| ScriptLangError::new("ARTIFACT_IO_ERROR", error.to_string()))?;
    let artifact: CompiledProjectArtifact = serde_json::from_str(&raw)
        .map_err(|error| ScriptLangError::new("ARTIFACT_PARSE_ERROR", error.to_string()))?;

    if artifact.schema_version != COMPILED_PROJECT_SCHEMA {
        return Err(ScriptLangError::new(
            "ARTIFACT_SCHEMA_UNSUPPORTED",
            format!(
                "Unsupported compiled artifact schema \"{}\", expected \"{}\".",
                artifact.schema_version, COMPILED_PROJECT_SCHEMA
            ),
        ));
    }

    Ok(artifact)
}

fn resolve_entry_script(
    scripts: &BTreeMap<String, ScriptIr>,
    explicit: Option<String>,
) -> Result<String, ScriptLangError> {
    if let Some(entry) = explicit {
        let Some(script) = scripts.get(&entry) else {
            return Err(ScriptLangError::new(
                "ARTIFACT_ENTRY_SCRIPT_NOT_FOUND",
                format!("Entry script \"{}\" is not registered.", entry),
            ));
        };
        validate_entry_script_access(script, &entry)?;
        return Ok(entry);
    }

    if scripts.contains_key("main.main") {
        let entry = "main.main".to_string();
        let script = scripts
            .get(&entry)
            .expect("main.main existence should be checked before retrieval");
        validate_entry_script_access(script, &entry)?;
        return Ok(entry);
    }

    Err(ScriptLangError::new(
        "ARTIFACT_ENTRY_MAIN_NOT_FOUND",
        "Expected script with name=\"main.main\" as default entry.",
    ))
}

fn validate_entry_script_access(script: &ScriptIr, entry: &str) -> Result<(), ScriptLangError> {
    if script.access != AccessLevel::Private {
        return Ok(());
    }
    Err(ScriptLangError::new(
        "ARTIFACT_ENTRY_SCRIPT_PRIVATE",
        format!(
            "Entry script \"{}\" is private and cannot be started by host.",
            entry
        ),
    ))
}

#[cfg(test)]
mod artifact_tests {
    use super::*;

    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(name: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should move forward")
            .as_nanos();
        std::env::temp_dir().join(format!("sl-compiler-artifact-{}-{}.json", name, nanos))
    }

    #[test]
    fn compile_artifact_from_xml_map_builds_v1_artifact() {
        let files = compiler_test_support::map(&[(
            "main.xml",
            r#"<module name="main" default_access="public">
<script name="main"><text>Hello</text></script>
</module>"#,
        )]);

        let artifact = compile_artifact_from_xml_map(&files, None).expect("compile artifact");
        assert_eq!(artifact.schema_version, COMPILED_PROJECT_SCHEMA);
        assert_eq!(artifact.compiler_version, DEFAULT_COMPILER_VERSION);
        assert_eq!(artifact.entry_script, "main.main");
        assert!(artifact.scripts.contains_key("main.main"));
    }

    #[test]
    fn compile_artifact_from_xml_map_validates_entry_script() {
        let files = compiler_test_support::map(&[(
            "main.xml",
            r#"<module name="main" default_access="public">
<script name="main"><text>Hello</text></script>
</module>"#,
        )]);

        let error = compile_artifact_from_xml_map(&files, Some("missing".to_string()))
            .expect_err("missing entry should fail");
        assert_eq!(error.code, "ARTIFACT_ENTRY_SCRIPT_NOT_FOUND");
    }

    #[test]
    fn compile_artifact_from_xml_map_accepts_explicit_entry_and_reports_missing_main() {
        let files = compiler_test_support::map(&[
            (
                "main.xml",
                r#"<module name="main" default_access="public">
<script name="main"><text>Main</text></script>
</module>"#,
            ),
            (
                "alt.xml",
                r#"<module name="alt" default_access="public">
<script name="alt"><text>Alt</text></script>
</module>"#,
            ),
        ]);
        let artifact = compile_artifact_from_xml_map(&files, Some("alt.alt".to_string()))
            .expect("explicit entry should pass");
        assert_eq!(artifact.entry_script, "alt.alt");

        let no_main = compiler_test_support::map(&[(
            "alt.xml",
            r#"<module name="alt" default_access="public">
<script name="alt"><text>Alt</text></script>
</module>"#,
        )]);
        let error = compile_artifact_from_xml_map(&no_main, None)
            .expect_err("missing default main should fail");
        assert_eq!(error.code, "ARTIFACT_ENTRY_MAIN_NOT_FOUND");
    }

    #[test]
    fn compile_artifact_from_xml_map_rejects_private_entry_script() {
        let private_default = compiler_test_support::map(&[(
            "main.xml",
            r#"<module name="main">
<script name="main"><text>Main</text></script>
</module>"#,
        )]);
        let private_default_error = compile_artifact_from_xml_map(&private_default, None)
            .expect_err("private default entry should fail");
        assert_eq!(private_default_error.code, "ARTIFACT_ENTRY_SCRIPT_PRIVATE");

        let private_explicit = compiler_test_support::map(&[
            (
                "main.xml",
                r#"<module name="main" default_access="public"><script name="main"><text>Main</text></script></module>"#,
            ),
            (
                "hidden.xml",
                r#"<module name="hidden"><script name="entry"><text>Hidden</text></script></module>"#,
            ),
        ]);
        let private_explicit_error =
            compile_artifact_from_xml_map(&private_explicit, Some("hidden.entry".to_string()))
                .expect_err("private explicit entry should fail");
        assert_eq!(private_explicit_error.code, "ARTIFACT_ENTRY_SCRIPT_PRIVATE");
    }

    #[test]
    fn compile_artifact_from_xml_map_propagates_compile_errors() {
        let files = compiler_test_support::map(&[("main.xml", "<script>")]);
        let error = compile_artifact_from_xml_map(&files, None)
            .expect_err("invalid xml should fail compile");
        assert_eq!(error.code, "XML_PARSE_ERROR");
    }

    #[test]
    fn write_and_read_artifact_json_roundtrip() {
        let files = compiler_test_support::map(&[(
            "main.xml",
            r#"<module name="main" default_access="public">
<script name="main"><text>Hello</text></script>
</module>"#,
        )]);
        let artifact = compile_artifact_from_xml_map(&files, None).expect("compile artifact");

        let path = temp_path("ok");
        write_artifact_json(&path, &artifact).expect("write artifact");
        let decoded = read_artifact_json(&path).expect("read artifact");
        assert_eq!(decoded, artifact);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn write_and_read_artifact_json_reports_io_and_schema_errors() {
        let files = compiler_test_support::map(&[(
            "main.xml",
            r#"<module name="main" default_access="public">
<script name="main"><text>Hello</text></script>
</module>"#,
        )]);
        let artifact = compile_artifact_from_xml_map(&files, None).expect("compile artifact");

        let dir_path = temp_path("dir");
        fs::create_dir_all(&dir_path).expect("create dir");
        let write_error =
            write_artifact_json(&dir_path, &artifact).expect_err("writing to dir should fail");
        assert_eq!(write_error.code, "ARTIFACT_IO_ERROR");
        let _ = fs::remove_dir_all(&dir_path);

        let mut bad_schema_artifact = artifact.clone();
        bad_schema_artifact.schema_version = "compiled-project.v0".to_string();
        let schema_error = write_artifact_json(&temp_path("write-schema"), &bad_schema_artifact)
            .expect_err("unsupported schema should fail");
        assert_eq!(schema_error.code, "ARTIFACT_SCHEMA_UNSUPPORTED");

        let missing_path = temp_path("missing-read");
        let read_error = read_artifact_json(&missing_path).expect_err("missing file should fail");
        assert_eq!(read_error.code, "ARTIFACT_IO_ERROR");
    }

    #[test]
    fn read_artifact_json_reports_parse_and_schema_errors() {
        let parse_path = temp_path("parse");
        fs::write(&parse_path, "{").expect("write invalid json");
        let parse_error = read_artifact_json(&parse_path).expect_err("parse should fail");
        assert_eq!(parse_error.code, "ARTIFACT_PARSE_ERROR");

        let schema_path = temp_path("schema");
        fs::write(
            &schema_path,
            r#"{
  "schemaVersion": "compiled-project.v0",
  "compilerVersion": "player",
  "entryScript": "main",
  "scripts": {},
  "globalJson": {},
  "defsGlobalDeclarations": {},
  "defsGlobalInitOrder": []
}"#,
        )
        .expect("write bad schema artifact");
        let schema_error = read_artifact_json(&schema_path).expect_err("schema should fail");
        assert_eq!(schema_error.code, "ARTIFACT_SCHEMA_UNSUPPORTED");

        let _ = fs::remove_file(parse_path);
        let _ = fs::remove_file(schema_path);
    }
}
