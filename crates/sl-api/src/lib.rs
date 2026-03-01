use std::collections::BTreeMap;
use std::sync::Arc;

use sl_compiler::{
    compile_artifact_from_xml_map as compile_compiled_artifact_from_xml_map,
    compile_project_bundle_from_xml_map, compile_project_scripts_from_xml_map,
    CompileProjectBundleResult, DEFAULT_COMPILER_VERSION,
};
use sl_core::{CompileProjectResult, CompiledProjectArtifactV1, SlValue, SnapshotV3};
use sl_runtime::{HostFunctionRegistry, ScriptLangEngineOptions};

pub use sl_core::{ChoiceItem, EngineOutput, ScriptLangError};
pub use sl_runtime::ScriptLangEngine;

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
pub struct CreateEngineFromArtifactOptions {
    pub artifact: CompiledProjectArtifactV1,
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

#[derive(Clone)]
pub struct ResumeEngineFromArtifactOptions {
    pub artifact: CompiledProjectArtifactV1,
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
        defs_global_declarations,
        defs_global_init_order,
    } = compile_project_bundle_from_xml_map(xml_by_path)?;

    let entry_script = resolve_entry_script(&scripts, entry_script)?;

    Ok(CompileProjectResult {
        scripts,
        entry_script,
        global_json,
        defs_global_declarations,
        defs_global_init_order,
    })
}

pub fn compile_artifact_from_xml_map(
    xml_by_path: &BTreeMap<String, String>,
    entry_script: Option<String>,
) -> Result<CompiledProjectArtifactV1, ScriptLangError> {
    compile_compiled_artifact_from_xml_map(xml_by_path, entry_script)
}

pub fn create_engine_from_artifact(
    options: CreateEngineFromArtifactOptions,
) -> Result<ScriptLangEngine, ScriptLangError> {
    if !options
        .artifact
        .scripts
        .contains_key(&options.artifact.entry_script)
    {
        return Err(ScriptLangError::new(
            "API_ARTIFACT_ENTRY_NOT_FOUND",
            format!(
                "Artifact entry script \"{}\" is not registered.",
                options.artifact.entry_script
            ),
        ));
    }

    let compiler_version = options
        .compiler_version
        .or_else(|| Some(options.artifact.compiler_version.clone()));

    let mut engine = ScriptLangEngine::new(ScriptLangEngineOptions {
        scripts: options.artifact.scripts,
        global_json: options.artifact.global_json,
        defs_global_declarations: options.artifact.defs_global_declarations,
        defs_global_init_order: options.artifact.defs_global_init_order,
        host_functions: options.host_functions,
        random_seed: options.random_seed,
        compiler_version,
    })?;

    engine.start(&options.artifact.entry_script, options.entry_args)?;
    Ok(engine)
}

pub fn resume_engine_from_artifact(
    options: ResumeEngineFromArtifactOptions,
) -> Result<ScriptLangEngine, ScriptLangError> {
    let compiler_version = options
        .compiler_version
        .or_else(|| Some(options.artifact.compiler_version.clone()));

    let mut engine = ScriptLangEngine::new(ScriptLangEngineOptions {
        scripts: options.artifact.scripts,
        global_json: options.artifact.global_json,
        defs_global_declarations: options.artifact.defs_global_declarations,
        defs_global_init_order: options.artifact.defs_global_init_order,
        host_functions: options.host_functions,
        random_seed: None,
        compiler_version,
    })?;

    engine.resume(options.snapshot)?;
    Ok(engine)
}

pub fn create_engine_from_xml(
    options: CreateEngineFromXmlOptions,
) -> Result<ScriptLangEngine, ScriptLangError> {
    let artifact = compile_artifact_from_xml_map(&options.scripts_xml, options.entry_script)?;
    create_engine_from_artifact(CreateEngineFromArtifactOptions {
        artifact,
        entry_args: options.entry_args,
        host_functions: options.host_functions,
        random_seed: options.random_seed,
        compiler_version: options.compiler_version,
    })
}

pub fn resume_engine_from_xml(
    options: ResumeEngineFromXmlOptions,
) -> Result<ScriptLangEngine, ScriptLangError> {
    let compiled = compile_project_bundle_from_xml_map(&options.scripts_xml)?;
    resume_engine_from_artifact(ResumeEngineFromArtifactOptions {
        artifact: CompiledProjectArtifactV1 {
            schema_version: sl_core::COMPILED_PROJECT_SCHEMA_V1.to_string(),
            compiler_version: DEFAULT_COMPILER_VERSION.to_string(),
            entry_script: "main".to_string(),
            scripts: compiled.scripts,
            global_json: compiled.global_json,
            defs_global_declarations: compiled.defs_global_declarations,
            defs_global_init_order: compiled.defs_global_init_order,
        },
        snapshot: options.snapshot,
        host_functions: options.host_functions,
        compiler_version: options.compiler_version,
    })
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
    use std::sync::Arc;

    #[derive(Debug)]
    struct ReservedRegistry;

    impl HostFunctionRegistry for ReservedRegistry {
        fn call(
            &self,
            _name: &str,
            _args: &[sl_core::SlValue],
        ) -> Result<sl_core::SlValue, sl_core::ScriptLangError> {
            Ok(sl_core::SlValue::Bool(true))
        }

        fn names(&self) -> &[String] {
            static NAMES: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
            NAMES.get_or_init(|| vec!["random".to_string()])
        }
    }

    #[test]
    fn host_function_registry_mock_covers_all_methods() {
        let registry = ReservedRegistry;
        // Cover the names() method
        let _names = registry.names();
        // Cover the call() method
        let _result = registry.call("test", &[]);
    }

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
            r#"
<script name="main">
  <choice text="Pick">
    <option text="A"><text>A</text></option>
    <option text="B"><text>B</text></option>
  </choice>
</script>
"#,
        )]);
        let compiled = compile_scripts_from_xml_map(&scripts).expect("compile should pass");
        assert!(compiled.contains_key("main"));
    }

    #[test]
    fn compile_project_from_xml_map_uses_default_main_entry() {
        let scripts = map(&[(
            "main.script.xml",
            r#"
<script name="main">
  <choice text="Pick">
    <option text="A"><text>A</text></option>
    <option text="B"><text>B</text></option>
  </choice>
</script>
"#,
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
    fn compile_artifact_from_xml_map_builds_v1_artifact() {
        let scripts = map(&[(
            "main.script.xml",
            r#"<script name="main"><text>Main</text></script>"#,
        )]);
        let artifact = compile_artifact_from_xml_map(&scripts, None).expect("compile artifact");
        assert_eq!(artifact.schema_version, sl_core::COMPILED_PROJECT_SCHEMA_V1);
        assert_eq!(artifact.entry_script, "main");
    }

    #[test]
    fn create_engine_from_artifact_starts_engine_and_validates_entry() {
        let scripts = map(&[(
            "main.script.xml",
            r#"
<script name="main">
  <choice text="Pick"><option text="A"><text>A</text></option></choice>
</script>
"#,
        )]);
        let artifact = compile_artifact_from_xml_map(&scripts, None).expect("compile artifact");
        let mut engine = create_engine_from_artifact(CreateEngineFromArtifactOptions {
            artifact,
            entry_args: None,
            host_functions: None,
            random_seed: Some(1),
            compiler_version: None,
        })
        .expect("engine should build");
        assert!(matches!(
            engine.next_output().expect("next output"),
            EngineOutput::Choices { .. }
        ));

        let bad_artifact = CompiledProjectArtifactV1 {
            schema_version: sl_core::COMPILED_PROJECT_SCHEMA_V1.to_string(),
            compiler_version: "player.v1".to_string(),
            entry_script: "missing".to_string(),
            scripts: BTreeMap::new(),
            global_json: BTreeMap::new(),
            defs_global_declarations: BTreeMap::new(),
            defs_global_init_order: Vec::new(),
        };
        let error = create_engine_from_artifact(CreateEngineFromArtifactOptions {
            artifact: bad_artifact,
            entry_args: None,
            host_functions: None,
            random_seed: Some(1),
            compiler_version: None,
        })
        .err()
        .expect("missing artifact entry should fail");
        assert_eq!(error.code, "API_ARTIFACT_ENTRY_NOT_FOUND");
    }

    #[test]
    fn resume_engine_from_artifact_resumes_from_snapshot() {
        let scripts = map(&[(
            "main.script.xml",
            r#"
<script name="main">
  <choice text="Pick"><option text="A"><text>A</text></option></choice>
</script>
"#,
        )]);

        let artifact = compile_artifact_from_xml_map(&scripts, None).expect("compile artifact");
        let mut engine = create_engine_from_artifact(CreateEngineFromArtifactOptions {
            artifact: artifact.clone(),
            entry_args: None,
            host_functions: None,
            random_seed: Some(1),
            compiler_version: None,
        })
        .expect("engine should build");
        assert!(matches!(
            engine.next_output().expect("next output"),
            EngineOutput::Choices { .. }
        ));
        let snapshot = engine.snapshot().expect("snapshot should succeed");

        let mut resumed = resume_engine_from_artifact(ResumeEngineFromArtifactOptions {
            artifact,
            snapshot,
            host_functions: None,
            compiler_version: None,
        })
        .expect("resume from artifact");
        resumed.choose(0).expect("choose should succeed");
        assert!(matches!(
            resumed.next_output().expect("next output"),
            EngineOutput::Text { .. }
        ));
    }

    #[test]
    fn create_engine_from_xml_starts_engine() {
        let scripts = map(&[(
            "main.script.xml",
            r#"
<script name="main">
  <choice text="Pick">
    <option text="A"><text>A</text></option>
    <option text="B"><text>B</text></option>
  </choice>
</script>
"#,
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

        let first = engine.next_output().expect("next should succeed");
        assert!(matches!(first, EngineOutput::Choices { .. }));
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
        let first = engine.next_output().expect("next should succeed");
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
        let next = resumed.next_output().expect("next should succeed");
        assert!(matches!(next, EngineOutput::Text { .. }));
    }

    #[test]
    fn create_and_resume_engine_from_xml_propagate_engine_new_errors() {
        let scripts = map(&[(
            "main.script.xml",
            r#"
<script name="main">
  <choice text="Pick">
    <option text="A"><text>A</text></option>
    <option text="B"><text>B</text></option>
  </choice>
</script>
"#,
        )]);
        let error = create_engine_from_xml(CreateEngineFromXmlOptions {
            scripts_xml: scripts.clone(),
            entry_script: None,
            entry_args: None,
            host_functions: Some(Arc::new(ReservedRegistry)),
            random_seed: Some(1),
            compiler_version: Some("player.v1".to_string()),
        })
        .err()
        .expect("reserved host function should fail create");
        assert_eq!(error.code, "ENGINE_HOST_FUNCTION_RESERVED");

        let mut ok_engine = create_engine_from_xml(CreateEngineFromXmlOptions {
            scripts_xml: scripts.clone(),
            entry_script: None,
            entry_args: None,
            host_functions: None,
            random_seed: Some(1),
            compiler_version: Some("player.v1".to_string()),
        })
        .expect("engine should build");
        let output = ok_engine.next_output().expect("choice output");
        assert!(
            matches!(output, EngineOutput::Choices { .. }),
            "unexpected output: {:?}",
            output
        );
        let snapshot = ok_engine.snapshot().expect("snapshot should succeed");
        let error = resume_engine_from_xml(ResumeEngineFromXmlOptions {
            scripts_xml: scripts,
            snapshot,
            host_functions: Some(Arc::new(ReservedRegistry)),
            compiler_version: Some("player.v1".to_string()),
        })
        .err()
        .expect("reserved host function should fail resume");
        assert_eq!(error.code, "ENGINE_HOST_FUNCTION_RESERVED");
    }
}
