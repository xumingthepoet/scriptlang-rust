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
    use std::fs;
    use std::path::{Path, PathBuf};

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

    pub(super) fn read_sources_recursive(
        root: &Path,
        current: &Path,
        out: &mut BTreeMap<String, String>,
    ) -> Result<(), std::io::Error> {
        for entry in fs::read_dir(current)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                read_sources_recursive(root, &path, out)?;
                continue;
            }
            let relative = path
                .strip_prefix(root)
                .expect("path should be under root")
                .to_string_lossy()
                .replace('\\', "/");
            out.insert(relative, fs::read_to_string(path)?);
        }
        Ok(())
    }

    pub(super) fn sources_from_example_dir(name: &str) -> BTreeMap<String, String> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("examples")
            .join("scripts-rhai")
            .join(name);
        let mut files = BTreeMap::new();
        read_sources_recursive(&root, &root, &mut files).expect("example should load");
        files
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
