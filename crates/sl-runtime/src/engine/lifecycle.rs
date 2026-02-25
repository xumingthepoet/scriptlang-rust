use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;
use std::sync::Arc;

use regex::Regex;
use rhai::{
    Array, Dynamic, Engine, EvalAltResult, ImmutableString, Map, Position, Scope, FLOAT, INT,
};
use sl_core::{
    default_value_from_type, is_type_compatible, ChoiceItem, ContinuationFrame, ContinueTarget,
    EngineOutput, PendingBoundaryV3, ScriptIr, ScriptLangError, ScriptNode, ScriptType, SlValue,
    SnapshotCompletion, SnapshotFrameV3, SnapshotV3,
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
    visible_json_by_script: HashMap<String, BTreeSet<String>>,
    visible_function_symbols_by_script: HashMap<String, BTreeMap<String, String>>,
    initial_random_seed: u32,

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

        Ok(Self {
            scripts: options.scripts,
            host_functions,
            compiler_version: options
                .compiler_version
                .unwrap_or_else(|| DEFAULT_COMPILER_VERSION.to_string()),
            group_lookup,
            global_json: options.global_json,
            visible_json_by_script,
            visible_function_symbols_by_script,
            initial_random_seed,
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


}
