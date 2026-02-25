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
