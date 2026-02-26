mod rng;

use rng::next_random_bounded;
#[cfg(test)]
use rng::{next_random_bounded_with, next_random_u32};

include!("lifecycle.rs");
include!("step.rs");
include!("boundary.rs");
include!("snapshot.rs");
include!("frame_stack.rs");
include!("callstack.rs");
include!("control_flow.rs");
include!("eval.rs");
include!("scope.rs");
include!("once_state.rs");
include!("../helpers/value_path.rs");
include!("../helpers/rhai_bridge.rs");

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
