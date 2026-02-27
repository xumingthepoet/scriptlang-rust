use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;
use std::sync::{Arc, OnceLock};

use regex::Regex;
use rhai::{
    Array, Dynamic, Engine, EvalAltResult, ImmutableString, Map, Position, Scope, FLOAT, INT,
};
use sl_core::{
    default_value_from_type, is_type_compatible, ChoiceItem, ContinuationFrame, ContinueTarget,
    DefsGlobalVarDecl, EngineOutput, PendingBoundaryV3, ScriptIr, ScriptLangError, ScriptNode,
    ScriptType, SlValue, SnapshotCompletion, SnapshotFrameV3, SnapshotV3,
};

pub const DEFAULT_COMPILER_VERSION: &str = "player.v1";
pub const SNAPSHOT_SCHEMA_V3: &str = "snapshot.v3";

pub trait HostFunctionRegistry: Send + Sync {
    fn call(&self, name: &str, args: &[SlValue]) -> Result<SlValue, ScriptLangError>;
    fn names(&self) -> &[String];
}

#[derive(Debug, Default)]
pub struct EmptyHostFunctionRegistry {
    names: Vec<String>,
}

impl HostFunctionRegistry for EmptyHostFunctionRegistry {
    fn call(&self, _name: &str, _args: &[SlValue]) -> Result<SlValue, ScriptLangError> {
        Err(ScriptLangError::new(
            "ENGINE_HOST_FUNCTION_MISSING",
            "Host function registry is empty.",
        ))
    }

    fn names(&self) -> &[String] {
        &self.names
    }
}

#[derive(Clone)]
pub struct ScriptLangEngineOptions {
    pub scripts: BTreeMap<String, ScriptIr>,
    pub global_json: BTreeMap<String, SlValue>,
    pub defs_global_declarations: BTreeMap<String, DefsGlobalVarDecl>,
    pub defs_global_init_order: Vec<String>,
    pub host_functions: Option<Arc<dyn HostFunctionRegistry>>,
    pub random_seed: Option<u32>,
    pub compiler_version: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompletionKind {
    None,
    WhileBody,
    ResumeAfterChild,
}

#[derive(Debug, Clone)]
struct RuntimeFrame {
    frame_id: u64,
    group_id: String,
    node_index: usize,
    scope: BTreeMap<String, SlValue>,
    completion: CompletionKind,
    script_root: bool,
    return_continuation: Option<ContinuationFrame>,
    var_types: BTreeMap<String, ScriptType>,
}

#[derive(Debug, Clone)]
enum PendingBoundary {
    Choice {
        frame_id: u64,
        node_id: String,
        options: Vec<ChoiceItem>,
        prompt_text: Option<String>,
    },
    Input {
        frame_id: u64,
        node_id: String,
        target_var: String,
        prompt_text: String,
        default_text: String,
    },
}

#[derive(Debug, Clone)]
struct GroupLookup {
    script_name: String,
    group_id: String,
}

type ScopeInit = (BTreeMap<String, SlValue>, BTreeMap<String, ScriptType>);

pub struct ScriptLangEngine {
    scripts: BTreeMap<String, ScriptIr>,
    host_functions: Arc<dyn HostFunctionRegistry>,
    compiler_version: String,
    group_lookup: HashMap<String, GroupLookup>,
    global_json: BTreeMap<String, SlValue>,
    defs_global_declarations: BTreeMap<String, DefsGlobalVarDecl>,
    defs_global_init_order: Vec<String>,
    defs_globals_value: BTreeMap<String, SlValue>,
    defs_globals_type: BTreeMap<String, ScriptType>,
    visible_json_by_script: HashMap<String, BTreeSet<String>>,
    visible_defs_by_script: HashMap<String, BTreeSet<String>>,
    defs_global_alias_by_script: HashMap<String, BTreeMap<String, String>>,
    visible_function_symbols_by_script: HashMap<String, BTreeMap<String, String>>,
    defs_prelude_by_script: HashMap<String, String>,
    initial_random_seed: u32,
    rhai_engine: Engine,
    shared_rng_state: Rc<RefCell<u32>>,

    frames: Vec<RuntimeFrame>,
    pending_boundary: Option<PendingBoundary>,
    waiting_choice: bool,
    ended: bool,
    frame_counter: u64,
    rng_state: u32,
    once_state_by_script: BTreeMap<String, BTreeSet<String>>,
}

impl ScriptLangEngine {
    pub fn new(options: ScriptLangEngineOptions) -> Result<Self, ScriptLangError> {
        let host_functions: Arc<dyn HostFunctionRegistry> = options
            .host_functions
            .unwrap_or_else(|| Arc::new(EmptyHostFunctionRegistry::default()));

        if host_functions.names().iter().any(|name| name == "random") {
            return Err(ScriptLangError::new(
                "ENGINE_HOST_FUNCTION_RESERVED",
                "hostFunctions cannot register reserved builtin name \"random\".",
            ));
        }

        let mut group_lookup = HashMap::new();
        let mut visible_json_by_script = HashMap::new();
        let mut visible_defs_by_script = HashMap::new();
        let mut defs_global_alias_by_script = HashMap::new();
        let mut visible_function_symbols_by_script = HashMap::new();

        for (script_name, script) in &options.scripts {
            for group_id in script.groups.keys() {
                group_lookup.insert(
                    group_id.clone(),
                    GroupLookup {
                        script_name: script_name.clone(),
                        group_id: group_id.clone(),
                    },
                );
            }
            visible_json_by_script.insert(
                script_name.clone(),
                script.visible_json_globals.iter().cloned().collect(),
            );
            let mut defs_aliases = BTreeMap::new();
            let mut visible_defs = BTreeSet::new();
            for (public_name, decl) in &script.visible_defs_globals {
                defs_aliases.insert(public_name.clone(), decl.qualified_name.clone());
                visible_defs.insert(decl.qualified_name.clone());
            }
            visible_defs_by_script.insert(script_name.clone(), visible_defs);
            defs_global_alias_by_script.insert(script_name.clone(), defs_aliases);

            for function_name in script.visible_functions.keys() {
                if host_functions
                    .names()
                    .iter()
                    .any(|name| name == function_name)
                {
                    return Err(ScriptLangError::new(
                        "ENGINE_HOST_FUNCTION_CONFLICT",
                        format!(
                            "hostFunctions cannot register \"{}\" because it conflicts with defs function.",
                            function_name
                        ),
                    ));
                }
            }

            let mut symbol_to_public = BTreeMap::new();
            let mut public_to_symbol = BTreeMap::new();
            for function_name in script.visible_functions.keys() {
                let symbol = rhai_function_symbol(function_name);
                if let Some(existing) = symbol_to_public.get(&symbol) {
                    if existing != function_name {
                        return Err(ScriptLangError::new(
                            "ENGINE_DEFS_FUNCTION_SYMBOL_CONFLICT",
                            format!(
                                "Defs function \"{}\" conflicts with \"{}\" after Rhai symbol normalization.",
                                function_name, existing
                            ),
                        ));
                    }
                }
                symbol_to_public.insert(symbol.clone(), function_name.clone());
                public_to_symbol.insert(function_name.clone(), symbol);
            }
            visible_function_symbols_by_script.insert(script_name.clone(), public_to_symbol);
        }

        let initial_random_seed = options.random_seed.unwrap_or(1);
        let shared_rng_state = Rc::new(RefCell::new(initial_random_seed));
        let mut rhai_engine = Engine::new();
        rhai_engine.set_strict_variables(true);
        let rng_for_builtin = Rc::clone(&shared_rng_state);
        rhai_engine.register_fn(
            "random",
            move |bound: INT| -> Result<INT, Box<EvalAltResult>> {
                if bound <= 0 {
                    return Err(Box::new(EvalAltResult::ErrorRuntime(
                        Dynamic::from("random(n) expects positive integer n."),
                        Position::NONE,
                    )));
                }

                let mut state = rng_for_builtin.borrow_mut();
                let value = next_random_bounded(&mut state, bound as u32);
                Ok(value as INT)
            },
        );

        let mut defs_globals_type = BTreeMap::new();
        for (qualified_name, decl) in &options.defs_global_declarations {
            defs_globals_type.insert(qualified_name.clone(), decl.r#type.clone());
        }

        Ok(Self {
            scripts: options.scripts,
            host_functions,
            compiler_version: options
                .compiler_version
                .unwrap_or_else(|| DEFAULT_COMPILER_VERSION.to_string()),
            group_lookup,
            global_json: options.global_json,
            defs_global_declarations: options.defs_global_declarations,
            defs_global_init_order: options.defs_global_init_order,
            defs_globals_value: BTreeMap::new(),
            defs_globals_type,
            visible_json_by_script,
            visible_defs_by_script,
            defs_global_alias_by_script,
            visible_function_symbols_by_script,
            defs_prelude_by_script: HashMap::new(),
            initial_random_seed,
            rhai_engine,
            shared_rng_state,
            frames: Vec::new(),
            pending_boundary: None,
            waiting_choice: false,
            ended: false,
            frame_counter: 1,
            rng_state: initial_random_seed,
            once_state_by_script: BTreeMap::new(),
        })
    }

    pub fn compiler_version(&self) -> &str {
        &self.compiler_version
    }

    pub fn waiting_choice(&self) -> bool {
        self.waiting_choice
    }

    pub fn start(
        &mut self,
        entry_script_name: &str,
        entry_args: Option<BTreeMap<String, SlValue>>,
    ) -> Result<(), ScriptLangError> {
        self.reset();
        self.initialize_defs_globals()?;
        let Some(script) = self.scripts.get(entry_script_name) else {
            return Err(ScriptLangError::new(
                "ENGINE_SCRIPT_NOT_FOUND",
                format!("Entry script \"{}\" is not registered.", entry_script_name),
            ));
        };
        let root_group_id = script.root_group_id.clone();

        let (scope, var_types) =
            self.create_script_root_scope(entry_script_name, entry_args.unwrap_or_default())?;
        self.push_root_frame(&root_group_id, scope, None, var_types);
        Ok(())
    }

    fn initialize_defs_globals(&mut self) -> Result<(), ScriptLangError> {
        self.defs_globals_value.clear();

        for qualified_name in self.defs_global_init_order.clone() {
            let decl = self
                .defs_global_declarations
                .get(&qualified_name)
                .cloned()
                .ok_or_else(|| {
                    ScriptLangError::new(
                        "ENGINE_DEFS_GLOBAL_DECL_MISSING",
                        format!(
                            "Defs global \"{}\" is present in init order but missing from declarations.",
                            qualified_name
                        ),
                    )
                })?;

            let mut value = default_value_from_type(&decl.r#type);
            if let Some(expr) = &decl.initial_value_expr {
                value = self.eval_defs_global_initializer(expr)?;
            }
            if !is_type_compatible(&value, &decl.r#type) {
                return Err(ScriptLangError::new(
                    "ENGINE_TYPE_MISMATCH",
                    format!(
                        "Defs global \"{}\" does not match declared type.",
                        qualified_name
                    ),
                ));
            }
            self.defs_globals_value.insert(qualified_name, value);
        }

        for (qualified_name, decl) in &self.defs_global_declarations {
            if self.defs_globals_value.contains_key(qualified_name) {
                continue;
            }
            self.defs_globals_value
                .insert(qualified_name.clone(), default_value_from_type(&decl.r#type));
        }

        Ok(())
    }


}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use super::runtime_test_support::*;
    
    #[derive(Debug)]
    struct TestRegistry {
        names: Vec<String>,
    }
    
    impl HostFunctionRegistry for TestRegistry {
        fn call(&self, _name: &str, _args: &[SlValue]) -> Result<SlValue, ScriptLangError> {
            Ok(SlValue::Bool(true))
        }
    
        fn names(&self) -> &[String] {
            &self.names
        }
    }

    #[test]
    fn new_rejects_reserved_host_function_name_random() {
        let files = map(&[(
            "main.script.xml",
            r#"<script name="main"><text>Hello</text></script>"#,
        )]);
        let compiled = compile_project_bundle_from_xml_map(&files).expect("compile should pass");
        let result = ScriptLangEngine::new(ScriptLangEngineOptions {
            scripts: compiled.scripts,
            global_json: compiled.global_json,
            defs_global_declarations: compiled.defs_global_declarations,
            defs_global_init_order: compiled.defs_global_init_order,
            host_functions: Some(Arc::new(TestRegistry {
                names: vec!["random".to_string()],
            })),
            random_seed: Some(1),
            compiler_version: Some(DEFAULT_COMPILER_VERSION.to_string()),
        });
        assert!(result.is_err());
        let error = result.err().expect("reserved random name should fail");
        assert_eq!(error.code, "ENGINE_HOST_FUNCTION_RESERVED");
    }

    #[test]
    fn new_rejects_host_function_conflicting_with_defs_function() {
        let files = map(&[
            (
                "main.script.xml",
                r#"
    <!-- include: shared.defs.xml -->
    <script name="main"><text>Hello</text></script>
    "#,
            ),
            (
                "shared.defs.xml",
                r#"
    <defs name="shared">
      <function name="addWithGameBonus" args="int:a1,int:a2" return="int:out">
        out = a1 + a2;
      </function>
    </defs>
    "#,
            ),
        ]);
        let compiled = compile_project_bundle_from_xml_map(&files).expect("compile should pass");
        let result = ScriptLangEngine::new(ScriptLangEngineOptions {
            scripts: compiled.scripts,
            global_json: compiled.global_json,
            defs_global_declarations: compiled.defs_global_declarations,
            defs_global_init_order: compiled.defs_global_init_order,
            host_functions: Some(Arc::new(TestRegistry {
                names: vec!["addWithGameBonus".to_string()],
            })),
            random_seed: Some(1),
            compiler_version: Some(DEFAULT_COMPILER_VERSION.to_string()),
        });
        assert!(result.is_err());
        let error = result.err().expect("conflicting defs function should fail");
        assert_eq!(error.code, "ENGINE_HOST_FUNCTION_CONFLICT");
    }

    #[test]
    fn new_rejects_defs_function_symbol_conflict_after_normalization() {
        let files = map(&[
            (
                "main.script.xml",
                r#"
    <!-- include: a.defs.xml -->
    <!-- include: x.defs.xml -->
    <script name="main"><text>Hello</text></script>
    "#,
            ),
            (
                "a.defs.xml",
                r#"
    <defs name="a">
      <function name="b" return="int:out">out = 1;</function>
    </defs>
    "#,
            ),
            (
                "x.defs.xml",
                r#"
    <defs name="x">
      <function name="a_b" return="int:out">out = 2;</function>
    </defs>
    "#,
            ),
        ]);
        let compiled = compile_project_bundle_from_xml_map(&files).expect("compile should pass");
        let result = ScriptLangEngine::new(ScriptLangEngineOptions {
            scripts: compiled.scripts,
            global_json: compiled.global_json,
            defs_global_declarations: compiled.defs_global_declarations,
            defs_global_init_order: compiled.defs_global_init_order,
            host_functions: None,
            random_seed: Some(1),
            compiler_version: Some(DEFAULT_COMPILER_VERSION.to_string()),
        });
        let error = match result {
            Ok(_) => panic!("normalized symbol conflict should fail"),
            Err(error) => error,
        };
        assert_eq!(error.code, "ENGINE_DEFS_FUNCTION_SYMBOL_CONFLICT");
    }

    #[test]
    fn start_returns_error_for_missing_script() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>Hello</text></script>"#,
        )]));
        let error = engine
            .start("missing", None)
            .expect_err("unknown entry should fail");
        assert_eq!(error.code, "ENGINE_SCRIPT_NOT_FOUND");
    }

    #[test]
    fn start_rejects_defs_global_initializer_type_mismatch() {
        let mut engine = engine_from_sources(map(&[
            (
                "shared.defs.xml",
                r#"
<defs name="shared">
  <var name="hp" type="int">"bad"</var>
</defs>
"#,
            ),
            (
                "main.script.xml",
                r#"
<!-- include: shared.defs.xml -->
<script name="main"><text>ok</text></script>
"#,
            ),
        ]));

        let error = engine
            .start("main", None)
            .expect_err("type mismatch should fail");
        assert_eq!(error.code, "ENGINE_TYPE_MISMATCH");
    }

    #[test]
    fn start_rejects_missing_defs_global_decl_in_init_order() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>ok</text></script>"#,
        )]));
        engine.defs_global_init_order = vec!["shared.hp".to_string()];
        let error = engine
            .start("main", None)
            .expect_err("missing decl in init order should fail");
        assert_eq!(error.code, "ENGINE_DEFS_GLOBAL_DECL_MISSING");
    }

    #[test]
    fn start_fills_default_for_defs_global_not_present_in_init_order() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>ok</text></script>"#,
        )]));
        engine.defs_global_declarations.insert(
            "shared.hp".to_string(),
            DefsGlobalVarDecl {
                namespace: "shared".to_string(),
                name: "hp".to_string(),
                qualified_name: "shared.hp".to_string(),
                r#type: ScriptType::Primitive {
                    name: "int".to_string(),
                },
                initial_value_expr: None,
                location: sl_core::SourceSpan::synthetic(),
            },
        );
        engine.defs_global_init_order.clear();

        engine.start("main", None).expect("start");
        assert_eq!(
            engine.defs_globals_value.get("shared.hp"),
            Some(&SlValue::Number(0.0))
        );
    }

    #[test]
    fn public_state_accessors_and_empty_registry_are_covered() {
        let registry = EmptyHostFunctionRegistry::default();
        assert!(registry.names().is_empty());
        let call_error = registry
            .call("noop", &[])
            .expect_err("empty registry call should fail");
        assert_eq!(call_error.code, "ENGINE_HOST_FUNCTION_MISSING");
    
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <choice text="Pick">
        <option text="A"><text>A</text></option>
      </choice>
    </script>
    "#,
        )]));
        assert_eq!(engine.compiler_version(), DEFAULT_COMPILER_VERSION);
        assert!(!engine.waiting_choice());
        engine.start("main", None).expect("start");
        let next = engine.next_output().expect("next");
        assert!(matches!(next, EngineOutput::Choices { .. }));
        assert!(engine.waiting_choice());
    }

    #[test]
    fn random_function_error_path_is_covered() {
        // Test that random(n) with n <= 0 returns error (covers lifecycle.rs lines 218, 220-222)
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <code>let x = random(0);</code>
      <text>done</text>
    </script>
    "#,
        )]));
        engine.start("main", None).expect("start");
        let error = engine.next_output().expect_err("random(0) should fail");
        assert!(error.code == "ENGINE_EVAL_ERROR" || error.code == "ENGINE_RANDOM_ERROR");
    }

}
