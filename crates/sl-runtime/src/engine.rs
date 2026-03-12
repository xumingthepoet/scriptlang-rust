#![allow(unused_imports)]

mod rng;

use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, OnceLock};

use crate::helpers::rhai_bridge::{
    dynamic_to_slvalue, slvalue_to_dynamic, slvalue_to_dynamic_with_type, slvalue_to_rhai_literal,
    slvalue_to_text,
};
use crate::helpers::value_path::{assign_nested_path, parse_ref_path};
use regex::Regex;
use rhai::{
    Array, Dynamic, Engine, EvalAltResult, ImmutableString, Map, Position, Scope, FLOAT, INT,
};
use rng::next_random_bounded;
#[cfg(test)]
use rng::{next_random_bounded_with, next_random_u32};
use sl_core::{
    default_value_from_type, is_type_compatible, module_namespace_symbol, rewrite_function_calls,
    rewrite_module_global_qualified_access, rhai_function_symbol, AccessLevel, ChoiceEntry,
    ChoiceItem, ContinuationFrame, ContinueTarget, EngineOutput, ModuleConstDecl, ModuleVarDecl,
    PendingBoundary, PendingDynamicChoiceBinding, ScriptIr, ScriptKind, ScriptLangError,
    ScriptNode, ScriptTarget, ScriptType, SlValue, Snapshot, SnapshotCompletion, SnapshotFrame,
};

mod boundary;
mod callstack;
mod control_flow;
mod eval;
mod frame_stack;
mod lifecycle;
mod once_state;
mod scope;
mod snapshot;
mod step;

pub use lifecycle::{
    EmptyHostFunctionRegistry, HostFunctionRegistry, RandomStateView, ScriptLangEngine,
    ScriptLangEngineOptions, DEFAULT_COMPILER_VERSION, SNAPSHOT_SCHEMA,
};

#[cfg(test)]
pub(super) mod runtime_test_support {
    use super::*;

    #[derive(Debug)]
    pub(super) struct TestRegistry {
        pub(super) names: Vec<String>,
    }

    impl HostFunctionRegistry for TestRegistry {
        fn call(&self, _name: &str, _args: &[SlValue]) -> Result<SlValue, ScriptLangError> {
            Ok(SlValue::Bool(true))
        }

        fn names(&self) -> &[String] {
            &self.names
        }
    }

    pub(super) fn map(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries
            .iter()
            .map(|(key, value)| {
                let normalized_key = normalize_test_source_path(key);
                let normalized_value = normalize_test_source_content(value);
                (normalized_key, normalized_value)
            })
            .collect()
    }

    fn normalize_test_source_path(path: &str) -> String {
        path.replace(".script.xml", ".xml")
            .replace(".module.xml", ".xml")
    }

    fn normalize_test_source_content(source: &str) -> String {
        let mut normalized = source
            .replace(".script.xml", ".xml")
            .replace(".module.xml", ".xml");

        let trimmed = normalized.trim_start();
        if !trimmed.starts_with("<module")
            && normalized.trim_end().ends_with("</module>")
            && trimmed.starts_with("<script")
        {
            let end_regex =
                Regex::new(r"</module>\s*\z").expect("stray module close regex should compile");
            normalized = end_regex.replace(&normalized, "").into_owned();
        }

        if let Some(wrapped) = normalize_wrapped_root(&normalized, "module") {
            return wrapped;
        }

        if let Some(wrapped) = normalize_wrapped_root(&normalized, "script") {
            return wrapped;
        }

        normalized
    }

    fn normalize_wrapped_root(source: &str, root_name: &str) -> Option<String> {
        let pattern = format!(r#"\A(\s*(?:<!--.*?-->\s*)*)<{root_name}\b([^>]*)>"#);
        let regex = Regex::new(&pattern).expect("test root regex should compile");
        let captures = regex.captures(source)?;
        if root_name == "module" {
            return Some(source.to_string());
        }
        let prefix = captures.get(1).map(|m| m.as_str()).unwrap_or_default();
        let attrs = captures.get(2).map(|m| m.as_str()).unwrap_or_default();
        let attr_regex = Regex::new(r#"name="([^"]+)""#).expect("attribute regex should compile");
        let module_name = attr_regex
            .captures(attrs)
            .and_then(|caps| caps.get(1).map(|m| m.as_str().to_string()))?;
        let replaced_open = regex.replace(
            source,
            format!(
                r#"{prefix}<module name="{module_name}" export="script:{module_name}">
<{root_name}{attrs}>"#
            ),
        );
        let closing = format!("</{root_name}>");
        let end_regex =
            Regex::new(&format!(r"{closing}\s*\z")).expect("closing regex should compile");
        Some(
            end_regex
                .replace(replaced_open.as_ref(), format!("{closing}\n</module>"))
                .into_owned(),
        )
    }

    pub(super) fn engine_from_sources(files: BTreeMap<String, String>) -> ScriptLangEngine {
        let compiled = compile_project_from_sources(files);
        ScriptLangEngine::new(ScriptLangEngineOptions {
            scripts: compiled.scripts,
            global_data: compiled.global_data,
            module_var_declarations: compiled.module_var_declarations,
            module_var_init_order: compiled.module_var_init_order,
            module_const_declarations: compiled.module_const_declarations,
            module_const_init_order: compiled.module_const_init_order,
            host_functions: None,
            random_seed: Some(1),
            random_sequence: None,
            random_sequence_index: None,
            compiler_version: None,
        })
        .expect("engine should build")
    }

    pub(super) fn engine_from_sources_with_global_data(
        files: BTreeMap<String, String>,
        global_data: BTreeMap<String, SlValue>,
        visible_global_symbols: &[&str],
    ) -> ScriptLangEngine {
        let mut compiled = compile_project_from_sources(files);
        let visible_globals = visible_global_symbols
            .iter()
            .map(|value| (*value).to_string())
            .collect::<Vec<_>>();
        for script in compiled.scripts.values_mut() {
            script.visible_globals = visible_globals.clone();
        }
        ScriptLangEngine::new(ScriptLangEngineOptions {
            scripts: compiled.scripts,
            global_data,
            module_var_declarations: compiled.module_var_declarations,
            module_var_init_order: compiled.module_var_init_order,
            module_const_declarations: compiled.module_const_declarations,
            module_const_init_order: compiled.module_const_init_order,
            host_functions: None,
            random_seed: Some(1),
            random_sequence: None,
            random_sequence_index: None,
            compiler_version: None,
        })
        .expect("engine should build")
    }

    pub(super) fn compile_project_from_sources(
        files: BTreeMap<String, String>,
    ) -> sl_core::CompileProjectResult {
        let bundle = sl_compiler::compile_project_bundle_from_xml_map(&files)
            .expect("compile project should pass");
        let mut scripts = bundle.scripts;
        let mut local_candidates: BTreeMap<String, Vec<ScriptIr>> = BTreeMap::new();
        scripts
            .values()
            .filter_map(|script| {
                script
                    .local_script_name
                    .clone()
                    .map(|local_name| (local_name, script.clone()))
            })
            .for_each(|(local_name, script)| {
                local_candidates.entry(local_name).or_default().push(script);
            });
        for (local_name, candidates) in local_candidates {
            if candidates.len() == 1 && !scripts.contains_key(&local_name) {
                let mut alias = candidates[0].clone();
                alias.script_name = local_name.clone();
                alias.module_name = None;
                alias.local_script_name = Some(local_name.clone());
                scripts.insert(local_name, alias);
            }
        }
        sl_core::CompileProjectResult {
            scripts,
            entry_script: "main.main".to_string(),
            global_data: bundle.global_data,
            module_var_declarations: bundle.module_var_declarations,
            module_var_init_order: bundle.module_var_init_order,
            module_const_declarations: bundle.module_const_declarations,
            module_const_init_order: bundle.module_const_init_order,
        }
    }

    pub(super) fn drive_engine_to_end(engine: &mut ScriptLangEngine) {
        for _ in 0..5_000usize {
            match engine.next_output().expect("next should pass") {
                EngineOutput::Text { .. } => {}
                EngineOutput::Debug { .. } => {}
                EngineOutput::Choices { items, .. } => {
                    let index = items.first().map(|item| item.index).unwrap_or(0);
                    engine.choose(index).expect("choose should pass");
                }
                EngineOutput::Input { .. } => {
                    engine.submit_input("").expect("input should pass");
                }
                EngineOutput::End => return,
            }
        }
    }

    #[test]
    fn test_source_normalization_covers_stray_closing_and_missing_name_paths() {
        let normalized_script =
            normalize_test_source_content("<script name=\"main\"><text>x</text></script></module>");
        assert!(normalized_script.contains("<module name=\"main\" export=\"script:main\">"));
        assert!(!normalized_script.contains("</module></module>"));

        let normalized_module = normalize_test_source_content("<module name=\"shared\"></module>");
        assert_eq!(normalized_module, "<module name=\"shared\"></module>");
        assert_eq!(
            normalize_test_source_content("<other></module>"),
            "<other></module>"
        );

        assert_eq!(
            normalize_wrapped_root("<module name=\"main\"></module>", "module"),
            Some("<module name=\"main\"></module>".to_string())
        );
        assert_eq!(normalize_wrapped_root("<script></script>", "script"), None);
    }

    #[test]
    fn compile_project_from_sources_adds_unique_local_alias_only_once() {
        let unique = compile_project_from_sources(map(&[(
            "battle.module.xml",
            r#"<module name="battle" export="script:main"><script name="main"><text>x</text></script></module>"#,
        )]));
        assert!(unique.scripts.contains_key("battle.main"));
        assert!(unique.scripts.contains_key("main"));
        assert_eq!(unique.entry_script, "main.main");

        let duplicate = compile_project_from_sources(map(&[
            (
                "a.module.xml",
                r#"<module name="a" export="script:main"><script name="main"><text>a</text></script></module>"#,
            ),
            (
                "b.module.xml",
                r#"<module name="b" export="script:main"><script name="main"><text>b</text></script></module>"#,
            ),
        ]));
        assert!(duplicate.scripts.contains_key("a.main"));
        assert!(duplicate.scripts.contains_key("b.main"));
        assert!(!duplicate.scripts.contains_key("main"));
    }
}
