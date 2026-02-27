#![allow(unused_imports)]

mod rng;

use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;
use std::sync::{Arc, OnceLock};

use crate::helpers::rhai_bridge::{
    defs_namespace_symbol, dynamic_to_slvalue, replace_defs_global_symbol,
    rewrite_defs_global_qualified_access, rewrite_function_calls, rhai_function_symbol,
    slvalue_to_dynamic, slvalue_to_rhai_literal, slvalue_to_text,
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
    default_value_from_type, is_type_compatible, ChoiceItem, ContinuationFrame, ContinueTarget,
    DefsGlobalVarDecl, EngineOutput, PendingBoundaryV3, ScriptIr, ScriptLangError, ScriptNode,
    ScriptType, SlValue, SnapshotCompletion, SnapshotFrameV3, SnapshotV3,
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
    EmptyHostFunctionRegistry, HostFunctionRegistry, ScriptLangEngine, ScriptLangEngineOptions,
    DEFAULT_COMPILER_VERSION, SNAPSHOT_SCHEMA_V3,
};

#[cfg(test)]
pub(super) mod runtime_test_support {
    use super::*;
    pub(super) use sl_compiler::compile_project_bundle_from_xml_map;

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
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    pub(super) fn engine_from_sources(files: BTreeMap<String, String>) -> ScriptLangEngine {
        let compiled = compile_project_bundle_from_xml_map(&files).expect("compile should pass");
        ScriptLangEngine::new(ScriptLangEngineOptions {
            scripts: compiled.scripts,
            global_json: compiled.global_json,
            defs_global_declarations: compiled.defs_global_declarations,
            defs_global_init_order: compiled.defs_global_init_order,
            host_functions: None,
            random_seed: Some(1),
            compiler_version: None,
        })
        .expect("engine should build")
    }

    pub(super) fn drive_engine_to_end(engine: &mut ScriptLangEngine) {
        for _ in 0..5_000usize {
            match engine.next_output().expect("next should pass") {
                EngineOutput::Text { .. } => {}
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
}
