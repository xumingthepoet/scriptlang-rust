use std::collections::BTreeMap;
use std::sync::Arc;

use sl_compiler::{
    compile_artifact_from_xml_map as compile_compiled_artifact_from_xml_map,
    compile_project_bundle_from_xml_map, compile_project_scripts_from_xml_map,
    CompileProjectBundleResult,
};
use sl_core::{CompileProjectResult, CompiledProjectArtifact};
use sl_runtime::{HostFunctionRegistry, ScriptLangEngineOptions};

pub use sl_compiler::write_artifact_json;
pub use sl_compiler::DEFAULT_COMPILER_VERSION;
pub use sl_core::{ChoiceItem, EngineOutput, PendingBoundary, ScriptLangError, SlValue, Snapshot};
pub use sl_runtime::{RandomStateView, ScriptLangEngine};

#[derive(Clone)]
pub struct CreateEngineFromXmlOptions {
    pub scripts_xml: BTreeMap<String, String>,
    pub entry_script: Option<String>,
    pub entry_args: Option<BTreeMap<String, SlValue>>,
    pub host_functions: Option<Arc<dyn HostFunctionRegistry>>,
    pub random_seed: Option<u32>,
    pub random_sequence: Option<Vec<u32>>,
    pub random_sequence_index: Option<usize>,
    pub compiler_version: Option<String>,
}

#[derive(Clone)]
pub struct CreateEngineFromArtifactOptions {
    pub artifact: CompiledProjectArtifact,
    pub entry_args: Option<BTreeMap<String, SlValue>>,
    pub host_functions: Option<Arc<dyn HostFunctionRegistry>>,
    pub random_seed: Option<u32>,
    pub random_sequence: Option<Vec<u32>>,
    pub random_sequence_index: Option<usize>,
    pub compiler_version: Option<String>,
}

#[derive(Clone)]
pub struct ResumeEngineFromXmlOptions {
    pub scripts_xml: BTreeMap<String, String>,
    pub snapshot: Snapshot,
    pub host_functions: Option<Arc<dyn HostFunctionRegistry>>,
    pub random_sequence: Option<Vec<u32>>,
    pub random_sequence_index: Option<usize>,
    pub compiler_version: Option<String>,
}

#[derive(Clone)]
pub struct ResumeEngineFromArtifactOptions {
    pub artifact: CompiledProjectArtifact,
    pub snapshot: Snapshot,
    pub host_functions: Option<Arc<dyn HostFunctionRegistry>>,
    pub random_sequence: Option<Vec<u32>>,
    pub random_sequence_index: Option<usize>,
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
) -> Result<CompiledProjectArtifact, ScriptLangError> {
    compile_compiled_artifact_from_xml_map(xml_by_path, entry_script)
}

pub fn create_engine_from_artifact(
    options: CreateEngineFromArtifactOptions,
) -> Result<ScriptLangEngine, ScriptLangError> {
    let Some(entry_script) = options.artifact.scripts.get(&options.artifact.entry_script) else {
        return Err(ScriptLangError::new(
            "API_ARTIFACT_ENTRY_NOT_FOUND",
            format!(
                "Artifact entry script \"{}\" is not registered.",
                options.artifact.entry_script
            ),
        ));
    };
    validate_entry_script_access(
        entry_script,
        &options.artifact.entry_script,
        "API_ARTIFACT_ENTRY_PRIVATE",
    )?;

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
        random_sequence: options.random_sequence,
        random_sequence_index: options.random_sequence_index,
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
        random_sequence: options.random_sequence,
        random_sequence_index: options.random_sequence_index,
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
        random_sequence: options.random_sequence,
        random_sequence_index: options.random_sequence_index,
        compiler_version: options.compiler_version,
    })
}

pub fn resume_engine_from_xml(
    options: ResumeEngineFromXmlOptions,
) -> Result<ScriptLangEngine, ScriptLangError> {
    let compiled = compile_project_bundle_from_xml_map(&options.scripts_xml)?;
    resume_engine_from_artifact(ResumeEngineFromArtifactOptions {
        artifact: CompiledProjectArtifact {
            schema_version: sl_core::COMPILED_PROJECT_SCHEMA.to_string(),
            compiler_version: sl_compiler::DEFAULT_COMPILER_VERSION.to_string(),
            entry_script: "main.main".to_string(),
            scripts: compiled.scripts,
            global_json: compiled.global_json,
            defs_global_declarations: compiled.defs_global_declarations,
            defs_global_init_order: compiled.defs_global_init_order,
        },
        snapshot: options.snapshot,
        host_functions: options.host_functions,
        random_sequence: options.random_sequence,
        random_sequence_index: options.random_sequence_index,
        compiler_version: options.compiler_version,
    })
}

fn resolve_entry_script(
    scripts: &BTreeMap<String, sl_core::ScriptIr>,
    explicit: Option<String>,
) -> Result<String, ScriptLangError> {
    if let Some(entry) = explicit {
        let Some(script) = scripts.get(&entry) else {
            return Err(ScriptLangError::new(
                "API_ENTRY_SCRIPT_NOT_FOUND",
                format!("Entry script \"{}\" is not registered.", entry),
            ));
        };
        validate_entry_script_access(script, &entry, "API_ENTRY_SCRIPT_PRIVATE")?;
        return Ok(entry);
    }

    if scripts.contains_key("main.main") {
        let entry = "main.main".to_string();
        let script = scripts
            .get(&entry)
            .expect("main.main existence should be checked before retrieval");
        validate_entry_script_access(script, &entry, "API_ENTRY_SCRIPT_PRIVATE")?;
        return Ok(entry);
    }

    Err(ScriptLangError::new(
        "API_ENTRY_MAIN_NOT_FOUND",
        "Expected script with name=\"main.main\" as default entry.",
    ))
}

fn validate_entry_script_access(
    script: &sl_core::ScriptIr,
    entry: &str,
    code: &'static str,
) -> Result<(), ScriptLangError> {
    if script.access != sl_core::AccessLevel::Private {
        return Ok(());
    }
    Err(ScriptLangError::new(
        code,
        format!(
            "Entry script \"{}\" is private and cannot be started by host.",
            entry
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output_kind(output: &EngineOutput) -> &'static str {
        match output {
            EngineOutput::Text { .. } => "text",
            EngineOutput::Debug { .. } => "debug",
            EngineOutput::Choices { .. } => "choices",
            EngineOutput::Input { .. } => "input",
            EngineOutput::End => "end",
        }
    }

    #[test]
    fn output_kind_supports_debug_variant() {
        let kind = output_kind(&EngineOutput::Debug {
            text: "dbg".to_string(),
        });
        assert_eq!(kind, "debug");
    }
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
            "main.xml",
            r#"
<module name="main" default_access="public">
<script name="main">
  <choice text="Pick">
    <option text="A"><text>A</text></option>
    <option text="B"><text>B</text></option>
  </choice>
</script>
</module>
"#,
        )]);
        let compiled = compile_scripts_from_xml_map(&scripts).expect("compile should pass");
        assert!(compiled.contains_key("main.main"));
    }

    #[test]
    fn compile_project_from_xml_map_uses_default_main_entry() {
        let scripts = map(&[(
            "main.xml",
            r#"
<module name="main" default_access="public">
<script name="main">
  <choice text="Pick">
    <option text="A"><text>A</text></option>
    <option text="B"><text>B</text></option>
  </choice>
</script>
</module>
"#,
        )]);
        let project = compile_project_from_xml_map(&scripts, None).expect("compile should pass");
        assert_eq!(project.entry_script, "main.main");
        assert!(project.scripts.contains_key("main.main"));
    }

    #[test]
    fn compile_project_from_xml_map_accepts_explicit_entry() {
        let scripts = map(&[
            (
                "main.xml",
                r#"<module name="main" default_access="public"><script name="main"><text>Main</text></script></module>"#,
            ),
            (
                "alt.xml",
                r#"<module name="alt" default_access="public"><script name="alt"><text>Alt</text></script></module>"#,
            ),
        ]);
        let project = compile_project_from_xml_map(&scripts, Some("alt.alt".to_string()))
            .expect("compile should pass with explicit entry");
        assert_eq!(project.entry_script, "alt.alt");
        assert!(project.scripts.contains_key("alt.alt"));
    }

    #[test]
    fn compile_project_from_xml_map_supports_module_entry() {
        let scripts = map(&[(
            "battle.xml",
            r#"
<module name="battle" default_access="public">
  <script name="main"><text>Battle</text></script>
</module>
"#,
        )]);
        let project = compile_project_from_xml_map(&scripts, Some("battle.main".to_string()))
            .expect("module entry should compile");
        assert_eq!(project.entry_script, "battle.main");
        assert!(project.scripts.contains_key("battle.main"));
    }

    #[test]
    fn compile_project_from_xml_map_returns_error_for_missing_explicit_entry() {
        let scripts = map(&[(
            "foo.xml",
            r#"<module name="foo" default_access="public"><script name="foo"><text>Hello</text></script></module>"#,
        )]);
        let error = compile_project_from_xml_map(&scripts, Some("missing".to_string()))
            .expect_err("missing entry should fail");
        assert_eq!(error.code, "API_ENTRY_SCRIPT_NOT_FOUND");
    }

    #[test]
    fn compile_project_from_xml_map_returns_error_without_main_when_entry_missing() {
        let scripts = map(&[(
            "foo.xml",
            r#"<module name="foo" default_access="public"><script name="foo"><text>Hello</text></script></module>"#,
        )]);
        let error =
            compile_project_from_xml_map(&scripts, None).expect_err("default main should fail");
        assert_eq!(error.code, "API_ENTRY_MAIN_NOT_FOUND");
    }

    #[test]
    fn compile_project_from_xml_map_rejects_private_entry_script() {
        let scripts = map(&[(
            "main.xml",
            r#"<module name="main"><script name="main"><text>Main</text></script></module>"#,
        )]);
        let default_error = compile_project_from_xml_map(&scripts, None)
            .expect_err("private default entry should fail");
        assert_eq!(default_error.code, "API_ENTRY_SCRIPT_PRIVATE");

        let explicit_scripts = map(&[
            (
                "main.xml",
                r#"<module name="main" default_access="public"><script name="main"><text>Main</text></script></module>"#,
            ),
            (
                "hidden.xml",
                r#"<module name="hidden"><script name="entry"><text>Hidden</text></script></module>"#,
            ),
        ]);
        let explicit_error =
            compile_project_from_xml_map(&explicit_scripts, Some("hidden.entry".to_string()))
                .expect_err("private explicit entry should fail");
        assert_eq!(explicit_error.code, "API_ENTRY_SCRIPT_PRIVATE");
    }

    #[test]
    fn compile_artifact_from_xml_map_builds_v1_artifact() {
        let scripts = map(&[(
            "main.xml",
            r#"<module name="main" default_access="public"><script name="main"><text>Main</text></script></module>"#,
        )]);
        let artifact = compile_artifact_from_xml_map(&scripts, None).expect("compile artifact");
        assert_eq!(artifact.schema_version, sl_core::COMPILED_PROJECT_SCHEMA);
        assert_eq!(artifact.entry_script, "main.main");
    }

    #[test]
    fn create_engine_from_artifact_starts_engine_and_validates_entry() {
        let scripts = map(&[(
            "main.xml",
            r#"
<module name="main" default_access="public">
<script name="main">
  <choice text="Pick"><option text="A"><text>A</text></option></choice>
</script>
</module>
"#,
        )]);
        let artifact = compile_artifact_from_xml_map(&scripts, None).expect("compile artifact");
        let mut engine = create_engine_from_artifact(CreateEngineFromArtifactOptions {
            artifact,
            entry_args: None,
            host_functions: None,
            random_seed: Some(1),
            random_sequence: None,
            random_sequence_index: None,
            compiler_version: None,
        })
        .expect("engine should build");
        let output = engine.next_output().expect("next output");
        assert_eq!(output_kind(&output), "choices");

        let bad_artifact = CompiledProjectArtifact {
            schema_version: sl_core::COMPILED_PROJECT_SCHEMA.to_string(),
            compiler_version: "player".to_string(),
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
            random_sequence: None,
            random_sequence_index: None,
            compiler_version: None,
        })
        .err()
        .expect("missing artifact entry should fail");
        assert_eq!(error.code, "API_ARTIFACT_ENTRY_NOT_FOUND");
    }

    #[test]
    fn create_engine_from_artifact_rejects_private_entry_script() {
        let scripts = map(&[(
            "main.xml",
            r#"<module name="main"><script name="main"><text>Main</text></script></module>"#,
        )]);
        let bundle = compile_project_bundle_from_xml_map(&scripts).expect("bundle should compile");
        let error = create_engine_from_artifact(CreateEngineFromArtifactOptions {
            artifact: CompiledProjectArtifact {
                schema_version: sl_core::COMPILED_PROJECT_SCHEMA.to_string(),
                compiler_version: sl_compiler::DEFAULT_COMPILER_VERSION.to_string(),
                entry_script: "main.main".to_string(),
                scripts: bundle.scripts,
                global_json: bundle.global_json,
                defs_global_declarations: bundle.defs_global_declarations,
                defs_global_init_order: bundle.defs_global_init_order,
            },
            entry_args: None,
            host_functions: None,
            random_seed: Some(1),
            random_sequence: None,
            random_sequence_index: None,
            compiler_version: None,
        })
        .err()
        .expect("private artifact entry should fail");
        assert_eq!(error.code, "API_ARTIFACT_ENTRY_PRIVATE");
    }

    #[test]
    fn resume_engine_from_artifact_resumes_from_snapshot() {
        let scripts = map(&[(
            "main.xml",
            r#"
<module name="main" default_access="public">
<script name="main">
  <choice text="Pick"><option text="A"><text>A</text></option></choice>
</script>
</module>
"#,
        )]);

        let artifact = compile_artifact_from_xml_map(&scripts, None).expect("compile artifact");
        let mut engine = create_engine_from_artifact(CreateEngineFromArtifactOptions {
            artifact: artifact.clone(),
            entry_args: None,
            host_functions: None,
            random_seed: Some(1),
            random_sequence: None,
            random_sequence_index: None,
            compiler_version: None,
        })
        .expect("engine should build");
        let output = engine.next_output().expect("next output");
        assert_eq!(output_kind(&output), "choices");
        let snapshot = engine.snapshot().expect("snapshot should succeed");

        let mut resumed = resume_engine_from_artifact(ResumeEngineFromArtifactOptions {
            artifact,
            snapshot,
            host_functions: None,
            random_sequence: None,
            random_sequence_index: None,
            compiler_version: None,
        })
        .expect("resume from artifact");
        resumed.choose(0).expect("choose should succeed");
        let output = resumed.next_output().expect("next output");
        assert_eq!(output_kind(&output), "text");
    }

    #[test]
    fn create_engine_from_xml_starts_engine() {
        let scripts = map(&[(
            "main.xml",
            r#"
<module name="main" default_access="public">
<script name="main">
  <choice text="Pick">
    <option text="A"><text>A</text></option>
    <option text="B"><text>B</text></option>
  </choice>
</script>
</module>
"#,
        )]);
        let mut engine = create_engine_from_xml(CreateEngineFromXmlOptions {
            scripts_xml: scripts,
            entry_script: None,
            entry_args: None,
            host_functions: None,
            random_seed: Some(7),
            random_sequence: None,
            random_sequence_index: None,
            compiler_version: Some("player".to_string()),
        })
        .expect("engine should build");

        let first = engine.next_output().expect("next should succeed");
        assert_eq!(output_kind(&first), "choices");
    }

    #[test]
    fn create_engine_from_xml_supports_random_sequence_override() {
        let scripts = map(&[(
            "main.xml",
            r#"
<module name="main" default_access="public">
<script name="main">
  <temp name="a" type="int">random(5)</temp>
  <text>${a}</text>
  <temp name="b" type="int">random(5)</temp>
  <text>${b}</text>
</script>
</module>
"#,
        )]);
        let mut engine = create_engine_from_xml(CreateEngineFromXmlOptions {
            scripts_xml: scripts,
            entry_script: None,
            entry_args: None,
            host_functions: None,
            random_seed: Some(7),
            random_sequence: Some(vec![12]),
            random_sequence_index: Some(0),
            compiler_version: Some("player".to_string()),
        })
        .expect("engine should build");

        assert_eq!(
            engine.next_output().expect("first output"),
            EngineOutput::Text {
                text: "2".to_string(),
                tag: None
            }
        );
        assert_eq!(
            engine.next_output().expect("second output"),
            EngineOutput::Text {
                text: "0".to_string(),
                tag: None
            }
        );
    }

    #[test]
    fn resume_engine_from_xml_resumes_from_snapshot() {
        let scripts = map(&[(
            "main.xml",
            r#"
<module name="main" default_access="public">
<script name="main">
  <choice text="Pick">
    <option text="A"><text>A</text></option>
  </choice>
</script>
</module>
"#,
        )]);

        let mut engine = create_engine_from_xml(CreateEngineFromXmlOptions {
            scripts_xml: scripts.clone(),
            entry_script: None,
            entry_args: None,
            host_functions: None,
            random_seed: Some(1),
            random_sequence: None,
            random_sequence_index: None,
            compiler_version: Some("player".to_string()),
        })
        .expect("engine should build");
        let first = engine.next_output().expect("next should succeed");
        assert_eq!(output_kind(&first), "choices");
        let snapshot = engine.snapshot().expect("snapshot should succeed");

        let mut resumed = resume_engine_from_xml(ResumeEngineFromXmlOptions {
            scripts_xml: scripts,
            snapshot,
            host_functions: None,
            random_sequence: None,
            random_sequence_index: None,
            compiler_version: Some("player".to_string()),
        })
        .expect("resume should succeed");
        resumed.choose(0).expect("choose should succeed");
        let next = resumed.next_output().expect("next should succeed");
        assert_eq!(output_kind(&next), "text");
    }

    #[test]
    fn create_and_resume_engine_from_xml_support_module_scripts_and_globals() {
        let scripts = map(&[(
            "battle.xml",
            r#"
<module name="battle" default_access="public">
  <var name="score" type="int">1</var>
  <function name="boost" args="int:x" return="int:out">out = x + 1;</function>
  <script name="main">
    <temp name="cmd" type="string">""</temp>
    <input var="cmd" text="Go"/>
    <code>score = boost(score);</code>
    <call script="next"/>
  </script>
  <script name="next">
    <text>${score}</text>
  </script>
</module>
"#,
        )]);

        let mut engine = create_engine_from_xml(CreateEngineFromXmlOptions {
            scripts_xml: scripts.clone(),
            entry_script: Some("battle.main".to_string()),
            entry_args: None,
            host_functions: None,
            random_seed: Some(1),
            random_sequence: None,
            random_sequence_index: None,
            compiler_version: Some("player".to_string()),
        })
        .expect("module engine should build");
        let first = engine.next_output().expect("input output");
        assert_eq!(output_kind(&first), "input");
        let snapshot = engine.snapshot().expect("snapshot should succeed");

        let mut resumed = resume_engine_from_xml(ResumeEngineFromXmlOptions {
            scripts_xml: scripts,
            snapshot,
            host_functions: None,
            random_sequence: None,
            random_sequence_index: None,
            compiler_version: Some("player".to_string()),
        })
        .expect("resume should succeed");
        resumed.submit_input("go").expect("input should succeed");
        assert_eq!(
            resumed.next_output().expect("text output"),
            EngineOutput::Text {
                text: "2".to_string(),
                tag: None,
            }
        );
    }

    #[test]
    fn create_and_resume_engine_from_xml_propagate_engine_new_errors() {
        let scripts = map(&[(
            "main.xml",
            r#"
<module name="main" default_access="public">
<script name="main">
  <choice text="Pick">
    <option text="A"><text>A</text></option>
    <option text="B"><text>B</text></option>
  </choice>
</script>
</module>
"#,
        )]);
        let error = create_engine_from_xml(CreateEngineFromXmlOptions {
            scripts_xml: scripts.clone(),
            entry_script: None,
            entry_args: None,
            host_functions: Some(Arc::new(ReservedRegistry)),
            random_seed: Some(1),
            random_sequence: None,
            random_sequence_index: None,
            compiler_version: Some("player".to_string()),
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
            random_sequence: None,
            random_sequence_index: None,
            compiler_version: Some("player".to_string()),
        })
        .expect("engine should build");
        let output = ok_engine.next_output().expect("choice output");
        assert_eq!(
            output_kind(&output),
            "choices",
            "unexpected output: {:?}",
            output
        );
        let snapshot = ok_engine.snapshot().expect("snapshot should succeed");
        let error = resume_engine_from_xml(ResumeEngineFromXmlOptions {
            scripts_xml: scripts,
            snapshot,
            host_functions: Some(Arc::new(ReservedRegistry)),
            random_sequence: None,
            random_sequence_index: None,
            compiler_version: Some("player".to_string()),
        })
        .err()
        .expect("reserved host function should fail resume");
        assert_eq!(error.code, "ENGINE_HOST_FUNCTION_RESERVED");
    }

    #[test]
    fn api_error_propagation_paths_are_covered() {
        let bad_xml = map(&[("main.xml", "<module>")]);
        let compile_error =
            compile_project_from_xml_map(&bad_xml, None).expect_err("compile should fail");
        assert_eq!(compile_error.code, "XML_PARSE_ERROR");

        let scripts = map(&[(
            "main.xml",
            r#"
<module name="main" default_access="public">
<script name="main">
  <temp name="x" type="int">1</temp>
</script>
</module>
"#,
        )]);
        let artifact = compile_artifact_from_xml_map(&scripts, None).expect("artifact");
        let create_error = create_engine_from_artifact(CreateEngineFromArtifactOptions {
            artifact: artifact.clone(),
            entry_args: Some(BTreeMap::from([(
                "x".to_string(),
                SlValue::String("bad".to_string()),
            )])),
            host_functions: None,
            random_seed: Some(1),
            random_sequence: None,
            random_sequence_index: None,
            compiler_version: None,
        })
        .err()
        .expect("start arg type mismatch should fail");
        assert_eq!(create_error.code, "ENGINE_CALL_ARG_UNKNOWN");

        let mut ok_engine = create_engine_from_artifact(CreateEngineFromArtifactOptions {
            artifact: artifact.clone(),
            entry_args: None,
            host_functions: None,
            random_seed: Some(1),
            random_sequence: None,
            random_sequence_index: None,
            compiler_version: None,
        })
        .expect("engine");
        let out = ok_engine.next_output().expect("next");
        assert_eq!(output_kind(&out), "end");
        let bad_snapshot = ok_engine.snapshot().expect_err("no pending should fail");
        assert_eq!(bad_snapshot.code, "SNAPSHOT_NOT_ALLOWED");
        let snapshot = Snapshot {
            schema_version: sl_runtime::SNAPSHOT_SCHEMA.to_string(),
            compiler_version: "player.bad".to_string(),
            runtime_frames: Vec::new(),
            rng_state: 1,
            pending_boundary: PendingBoundary::Input {
                node_id: "n".to_string(),
                target_var: "x".to_string(),
                prompt_text: "p".to_string(),
                default_text: "d".to_string(),
            },
            defs_globals: BTreeMap::new(),
            once_state_by_script: BTreeMap::new(),
        };
        let resume_error = resume_engine_from_artifact(ResumeEngineFromArtifactOptions {
            artifact: artifact.clone(),
            snapshot,
            host_functions: None,
            random_sequence: None,
            random_sequence_index: None,
            compiler_version: None,
        })
        .err()
        .expect("resume should fail");
        assert_eq!(resume_error.code, "SNAPSHOT_COMPILER_VERSION");

        let create_xml_error = create_engine_from_xml(CreateEngineFromXmlOptions {
            scripts_xml: bad_xml.clone(),
            entry_script: None,
            entry_args: None,
            host_functions: None,
            random_seed: Some(1),
            random_sequence: None,
            random_sequence_index: None,
            compiler_version: None,
        })
        .err()
        .expect("create from xml should fail");
        assert_eq!(create_xml_error.code, "XML_PARSE_ERROR");

        let resume_xml_error = resume_engine_from_xml(ResumeEngineFromXmlOptions {
            scripts_xml: bad_xml,
            snapshot: Snapshot {
                schema_version: sl_runtime::SNAPSHOT_SCHEMA.to_string(),
                compiler_version: "player".to_string(),
                runtime_frames: Vec::new(),
                rng_state: 1,
                pending_boundary: PendingBoundary::Input {
                    node_id: "n".to_string(),
                    target_var: "x".to_string(),
                    prompt_text: "p".to_string(),
                    default_text: "d".to_string(),
                },
                defs_globals: BTreeMap::new(),
                once_state_by_script: BTreeMap::new(),
            },
            host_functions: None,
            random_sequence: None,
            random_sequence_index: None,
            compiler_version: None,
        })
        .err()
        .expect("resume from xml should fail");
        assert_eq!(resume_xml_error.code, "XML_PARSE_ERROR");
    }

    #[test]
    fn create_engine_from_xml_can_emit_input_output() {
        let scripts = map(&[(
            "main.xml",
            r#"
<module name="main" default_access="public">
<script name="main">
  <temp name="name" type="string">"Traveler"</temp>
  <input var="name" text="Name"/>
</script>
</module>
"#,
        )]);
        let mut engine = create_engine_from_xml(CreateEngineFromXmlOptions {
            scripts_xml: scripts,
            entry_script: None,
            entry_args: None,
            host_functions: None,
            random_seed: Some(1),
            random_sequence: None,
            random_sequence_index: None,
            compiler_version: None,
        })
        .expect("engine should build");
        let out = engine.next_output().expect("input output");
        assert_eq!(output_kind(&out), "input");
    }
}
