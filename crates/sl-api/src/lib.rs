use std::collections::BTreeMap;
use std::sync::Arc;

use sl_compiler::{
    compile_project_bundle_from_xml_map, compile_project_scripts_from_xml_map,
    CompileProjectBundleResult,
};
use sl_core::{CompileProjectResult, ScriptLangError, SlValue, SnapshotV3};
use sl_runtime::{HostFunctionRegistry, ScriptLangEngine, ScriptLangEngineOptions};

#[derive(Clone)]
pub struct CreateEngineFromXmlOptions {
    pub scripts_xml: BTreeMap<String, String>,
    pub entry_script: Option<String>,
    pub entry_args: Option<BTreeMap<String, SlValue>>,
    pub host_functions: Option<Arc<dyn HostFunctionRegistry>>,
    pub random_seed: Option<u32>,
    pub compiler_version: Option<String>,
}

#[derive(Clone)]
pub struct ResumeEngineFromXmlOptions {
    pub scripts_xml: BTreeMap<String, String>,
    pub snapshot: SnapshotV3,
    pub host_functions: Option<Arc<dyn HostFunctionRegistry>>,
    pub compiler_version: Option<String>,
}

pub fn compile_scripts_from_xml_map(
    scripts_xml: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, sl_core::ScriptIr>, ScriptLangError> {
    compile_project_scripts_from_xml_map(scripts_xml)
}

pub fn compile_project_from_xml_map(
    xml_by_path: &BTreeMap<String, String>,
    entry_script: Option<String>,
) -> Result<CompileProjectResult, ScriptLangError> {
    let CompileProjectBundleResult {
        scripts,
        global_json,
    } = compile_project_bundle_from_xml_map(xml_by_path)?;

    let entry_script = resolve_entry_script(&scripts, entry_script)?;

    Ok(CompileProjectResult {
        scripts,
        entry_script,
        global_json,
    })
}

pub fn create_engine_from_xml(
    options: CreateEngineFromXmlOptions,
) -> Result<ScriptLangEngine, ScriptLangError> {
    let compiled = compile_project_from_xml_map(&options.scripts_xml, options.entry_script)?;

    let mut engine = ScriptLangEngine::new(ScriptLangEngineOptions {
        scripts: compiled.scripts,
        global_json: compiled.global_json,
        host_functions: options.host_functions,
        random_seed: options.random_seed,
        compiler_version: options.compiler_version,
    })?;

    engine.start(&compiled.entry_script, options.entry_args)?;
    Ok(engine)
}

pub fn resume_engine_from_xml(
    options: ResumeEngineFromXmlOptions,
) -> Result<ScriptLangEngine, ScriptLangError> {
    let compiled = compile_project_bundle_from_xml_map(&options.scripts_xml)?;

    let mut engine = ScriptLangEngine::new(ScriptLangEngineOptions {
        scripts: compiled.scripts,
        global_json: compiled.global_json,
        host_functions: options.host_functions,
        random_seed: None,
        compiler_version: options.compiler_version,
    })?;

    engine.resume(options.snapshot)?;
    Ok(engine)
}

fn resolve_entry_script(
    scripts: &BTreeMap<String, sl_core::ScriptIr>,
    explicit: Option<String>,
) -> Result<String, ScriptLangError> {
    if let Some(entry) = explicit {
        if !scripts.contains_key(&entry) {
            return Err(ScriptLangError::new(
                "API_ENTRY_SCRIPT_NOT_FOUND",
                format!("Entry script \"{}\" is not registered.", entry),
            ));
        }
        return Ok(entry);
    }

    if scripts.contains_key("main") {
        return Ok("main".to_string());
    }

    Err(ScriptLangError::new(
        "API_ENTRY_MAIN_NOT_FOUND",
        "Expected script with name=\"main\" as default entry.",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sl_core::EngineOutput;

    fn map(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn compile_scripts_from_xml_map_compiles_single_script() {
        let scripts = map(&[(
            "main.script.xml",
            r#"<script name="main"><text>Hello</text></script>"#,
        )]);
        let compiled = compile_scripts_from_xml_map(&scripts).expect("compile should pass");
        assert!(compiled.contains_key("main"));
    }

    #[test]
    fn compile_project_from_xml_map_uses_default_main_entry() {
        let scripts = map(&[(
            "main.script.xml",
            r#"<script name="main"><text>Hello</text></script>"#,
        )]);
        let project = compile_project_from_xml_map(&scripts, None).expect("compile should pass");
        assert_eq!(project.entry_script, "main");
        assert!(project.scripts.contains_key("main"));
    }

    #[test]
    fn compile_project_from_xml_map_accepts_explicit_entry() {
        let scripts = map(&[
            (
                "main.script.xml",
                r#"<script name="main"><text>Main</text></script>"#,
            ),
            (
                "alt.script.xml",
                r#"<script name="alt"><text>Alt</text></script>"#,
            ),
        ]);
        let project = compile_project_from_xml_map(&scripts, Some("alt".to_string()))
            .expect("compile should pass with explicit entry");
        assert_eq!(project.entry_script, "alt");
        assert!(project.scripts.contains_key("alt"));
    }

    #[test]
    fn compile_project_from_xml_map_returns_error_for_missing_explicit_entry() {
        let scripts = map(&[(
            "foo.script.xml",
            r#"<script name="foo"><text>Hello</text></script>"#,
        )]);
        let error = compile_project_from_xml_map(&scripts, Some("missing".to_string()))
            .expect_err("missing entry should fail");
        assert_eq!(error.code, "API_ENTRY_SCRIPT_NOT_FOUND");
    }

    #[test]
    fn compile_project_from_xml_map_returns_error_without_main_when_entry_missing() {
        let scripts = map(&[(
            "foo.script.xml",
            r#"<script name="foo"><text>Hello</text></script>"#,
        )]);
        let error =
            compile_project_from_xml_map(&scripts, None).expect_err("default main should fail");
        assert_eq!(error.code, "API_ENTRY_MAIN_NOT_FOUND");
    }

    #[test]
    fn create_engine_from_xml_starts_engine() {
        let scripts = map(&[(
            "main.script.xml",
            r#"<script name="main"><text>Hello</text></script>"#,
        )]);
        let mut engine = create_engine_from_xml(CreateEngineFromXmlOptions {
            scripts_xml: scripts,
            entry_script: None,
            entry_args: None,
            host_functions: None,
            random_seed: Some(7),
            compiler_version: Some("player.v1".to_string()),
        })
        .expect("engine should build");

        let first = engine.next().expect("next should succeed");
        assert!(matches!(first, EngineOutput::Text { .. }));
    }

    #[test]
    fn resume_engine_from_xml_resumes_from_snapshot() {
        let scripts = map(&[(
            "main.script.xml",
            r#"
<script name="main">
  <choice text="Pick">
    <option text="A"><text>A</text></option>
  </choice>
</script>
"#,
        )]);

        let mut engine = create_engine_from_xml(CreateEngineFromXmlOptions {
            scripts_xml: scripts.clone(),
            entry_script: None,
            entry_args: None,
            host_functions: None,
            random_seed: Some(1),
            compiler_version: Some("player.v1".to_string()),
        })
        .expect("engine should build");
        let first = engine.next().expect("next should succeed");
        assert!(matches!(first, EngineOutput::Choices { .. }));
        let snapshot = engine.snapshot().expect("snapshot should succeed");

        let mut resumed = resume_engine_from_xml(ResumeEngineFromXmlOptions {
            scripts_xml: scripts,
            snapshot,
            host_functions: None,
            compiler_version: Some("player.v1".to_string()),
        })
        .expect("resume should succeed");
        resumed.choose(0).expect("choose should succeed");
        let next = resumed.next().expect("next should succeed");
        assert!(matches!(next, EngineOutput::Text { .. }));
    }
}
