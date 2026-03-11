use super::*;
use sl_core::FunctionDecl;

pub const DEFAULT_COMPILER_VERSION: &str = "player";
pub const SNAPSHOT_SCHEMA: &str = "snapshot";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RandomStateView {
    Seeded { state: u32 },
    Sequence { values: Vec<u32>, index: usize },
}

#[derive(Debug, Clone)]
pub(super) enum RuntimeRandomState {
    Seeded(u32),
    Sequence { values: Vec<u32>, index: usize },
}

pub trait HostFunctionRegistry: Send + Sync {
    fn call(&self, name: &str, args: &[SlValue]) -> Result<SlValue, ScriptLangError>;
    fn names(&self) -> &[String];
}

#[derive(Debug, Default)]
pub struct EmptyHostFunctionRegistry {
    pub(super) names: Vec<String>,
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
    pub global_data: BTreeMap<String, SlValue>,
    pub module_var_declarations: BTreeMap<String, ModuleVarDecl>,
    pub module_var_init_order: Vec<String>,
    pub module_const_declarations: BTreeMap<String, ModuleConstDecl>,
    pub module_const_init_order: Vec<String>,
    pub host_functions: Option<Arc<dyn HostFunctionRegistry>>,
    pub random_seed: Option<u32>,
    pub random_sequence: Option<Vec<u32>>,
    pub random_sequence_index: Option<usize>,
    pub compiler_version: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CompletionKind {
    None,
    WhileBody,
    ResumeAfterChild,
}

#[derive(Debug, Clone)]
pub(super) struct RuntimeFrame {
    pub(super) frame_id: u64,
    pub(super) group_id: String,
    pub(super) node_index: usize,
    pub(super) scope: BTreeMap<String, SlValue>,
    pub(super) completion: CompletionKind,
    pub(super) script_root: bool,
    pub(super) return_continuation: Option<ContinuationFrame>,
    pub(super) var_types: BTreeMap<String, ScriptType>,
}

#[derive(Debug, Clone)]
pub(super) struct PendingChoiceOption {
    pub(super) item: ChoiceItem,
    pub(super) dynamic_binding: Option<PendingDynamicChoiceBinding>,
}

#[derive(Debug, Clone)]
pub(super) enum PendingBoundary {
    Choice {
        frame_id: u64,
        node_id: String,
        options: Vec<PendingChoiceOption>,
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
pub(super) struct GroupLookup {
    pub(super) script_name: String,
    pub(super) group_id: String,
}

pub(super) type ScopeInit = (BTreeMap<String, SlValue>, BTreeMap<String, ScriptType>);

pub struct ScriptLangEngine {
    pub(super) scripts: BTreeMap<String, ScriptIr>,
    pub(super) host_functions: Arc<dyn HostFunctionRegistry>,
    pub(super) compiler_version: String,
    pub(super) group_lookup: HashMap<String, GroupLookup>,
    pub(super) global_data: BTreeMap<String, SlValue>,
    pub(super) module_var_declarations: BTreeMap<String, ModuleVarDecl>,
    pub(super) module_var_init_order: Vec<String>,
    pub(super) module_const_declarations: BTreeMap<String, ModuleConstDecl>,
    pub(super) module_const_init_order: Vec<String>,
    pub(super) module_vars_value: BTreeMap<String, SlValue>,
    pub(super) module_vars_type: BTreeMap<String, ScriptType>,
    pub(super) module_consts_value: BTreeMap<String, SlValue>,
    pub(super) module_consts_type: BTreeMap<String, ScriptType>,
    pub(super) visible_globals_by_script: HashMap<String, BTreeSet<String>>,
    pub(super) visible_module_by_script: HashMap<String, BTreeSet<String>>,
    pub(super) module_global_alias_by_script: HashMap<String, BTreeMap<String, String>>,
    pub(super) visible_consts_by_script: HashMap<String, BTreeSet<String>>,
    pub(super) module_const_alias_by_script: HashMap<String, BTreeMap<String, String>>,
    pub(super) visible_function_symbols_by_script: HashMap<String, BTreeMap<String, String>>,
    pub(super) invoke_all_functions: BTreeMap<String, FunctionDecl>,
    pub(super) invoke_public_functions: BTreeSet<String>,
    pub(super) invoke_function_symbols: BTreeMap<String, String>,
    pub(super) module_prelude_by_script: HashMap<String, String>,
    pub(super) initial_random_seed: u32,
    pub(super) initial_random_sequence: Option<Vec<u32>>,
    pub(super) rhai_engine: Engine,
    pub(super) shared_rng_state: Rc<RefCell<RuntimeRandomState>>,

    pub(super) frames: Vec<RuntimeFrame>,
    pub(super) pending_boundary: Option<PendingBoundary>,
    pub(super) waiting_choice: bool,
    pub(super) ended: bool,
    pub(super) frame_counter: u64,
    pub(super) seeded_rng_state: u32,
    pub(super) once_state_by_script: BTreeMap<String, BTreeSet<String>>,
}

impl ScriptLangEngine {
    pub fn new(options: ScriptLangEngineOptions) -> Result<Self, ScriptLangError> {
        const RESERVED_HOST_BUILTINS: [&str; 4] =
            ["random", "invoke", "enum_to_string", "all_enum_members"];
        let host_functions: Arc<dyn HostFunctionRegistry> = options
            .host_functions
            .unwrap_or_else(|| Arc::new(EmptyHostFunctionRegistry::default()));

        if host_functions
            .names()
            .iter()
            .any(|name| RESERVED_HOST_BUILTINS.contains(&name.as_str()))
        {
            return Err(ScriptLangError::new(
                "ENGINE_HOST_FUNCTION_RESERVED",
                "hostFunctions cannot register reserved builtin names.",
            ));
        }

        let mut group_lookup: HashMap<String, GroupLookup> = HashMap::new();
        let mut visible_globals_by_script = HashMap::new();
        let mut visible_module_by_script = HashMap::new();
        let mut module_global_alias_by_script = HashMap::new();
        let mut visible_consts_by_script = HashMap::new();
        let mut module_const_alias_by_script = HashMap::new();
        let mut visible_function_symbols_by_script = HashMap::new();

        let mut invoke_all_functions = BTreeMap::new();
        let mut invoke_public_functions = BTreeSet::new();
        let mut invoke_function_symbols = BTreeMap::new();

        for (script_name, script) in &options.scripts {
            if script.visible_functions.contains_key("invoke") {
                return Err(ScriptLangError::new(
                    "ENGINE_MODULE_FUNCTION_RESERVED",
                    "Module function name \"invoke\" is reserved for runtime builtin.",
                ));
            }
            for group_id in script.groups.keys() {
                let should_replace = match group_lookup.get(group_id) {
                    None => true,
                    Some(existing_lookup) => options
                        .scripts
                        .get(&existing_lookup.script_name)
                        .is_none_or(|existing_script| {
                            script.module_name.is_some() || existing_script.module_name.is_none()
                        }),
                };
                if should_replace {
                    group_lookup.insert(
                        group_id.clone(),
                        GroupLookup {
                            script_name: script_name.clone(),
                            group_id: group_id.clone(),
                        },
                    );
                }
            }
            visible_globals_by_script.insert(
                script_name.clone(),
                script.visible_globals.iter().cloned().collect(),
            );
            let mut module_aliases = BTreeMap::new();
            let mut visible_module = BTreeSet::new();
            for (public_name, decl) in &script.visible_module_vars {
                module_aliases.insert(public_name.clone(), decl.qualified_name.clone());
                visible_module.insert(decl.qualified_name.clone());
            }
            visible_module_by_script.insert(script_name.clone(), visible_module);
            module_global_alias_by_script.insert(script_name.clone(), module_aliases);

            let mut const_aliases = BTreeMap::new();
            let mut visible_consts = BTreeSet::new();
            for (public_name, decl) in &script.visible_module_consts {
                const_aliases.insert(public_name.clone(), decl.qualified_name.clone());
                visible_consts.insert(decl.qualified_name.clone());
            }
            visible_consts_by_script.insert(script_name.clone(), visible_consts);
            module_const_alias_by_script.insert(script_name.clone(), const_aliases);

            for function_name in script.visible_functions.keys() {
                if host_functions
                    .names()
                    .iter()
                    .any(|name| name == function_name)
                {
                    return Err(ScriptLangError::new(
                        "ENGINE_HOST_FUNCTION_CONFLICT",
                        format!(
                            "hostFunctions cannot register \"{}\" because it conflicts with module function.",
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
                    return Err(ScriptLangError::new(
                        "ENGINE_MODULE_FUNCTION_SYMBOL_CONFLICT",
                        format!(
                            "Module function \"{}\" conflicts with \"{}\" after Rhai symbol normalization.",
                            function_name, existing
                        ),
                    ));
                }
                symbol_to_public.insert(symbol.clone(), function_name.clone());
                public_to_symbol.insert(function_name.clone(), symbol);
            }
            visible_function_symbols_by_script.insert(script_name.clone(), public_to_symbol);

            for (qualified_name, decl) in &script.invoke_all_functions {
                if !invoke_all_functions.contains_key(qualified_name) {
                    invoke_all_functions.insert(qualified_name.clone(), decl.clone());
                }
            }
            invoke_public_functions.extend(script.invoke_public_functions.iter().cloned());
        }
        for qualified_name in invoke_all_functions.keys() {
            let symbol = rhai_function_symbol(qualified_name);
            invoke_function_symbols.insert(qualified_name.clone(), symbol);
        }
        let initial_random_seed = options.random_seed.unwrap_or(1);
        let initial_random_sequence = options.random_sequence.clone();
        let random_sequence_index = options.random_sequence_index.unwrap_or(0);
        let shared_rng_state = Rc::new(RefCell::new(match options.random_sequence {
            Some(values) => RuntimeRandomState::Sequence {
                values,
                index: random_sequence_index,
            },
            None => RuntimeRandomState::Seeded(initial_random_seed),
        }));
        let mut rhai_engine = Engine::new();
        rhai_engine.set_strict_variables(false);
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
                let value = match &mut *state {
                    RuntimeRandomState::Seeded(seed_state) => {
                        next_random_bounded(seed_state, bound as u32)
                    }
                    RuntimeRandomState::Sequence { values, index } => {
                        if *index >= values.len() {
                            0
                        } else {
                            let value = values[*index] % (bound as u32);
                            *index += 1;
                            value
                        }
                    }
                };
                Ok(value as INT)
            },
        );
        rhai_engine.register_fn(
            "enum_to_string",
            |value: ImmutableString| -> ImmutableString { value },
        );
        fn collect_enum_members_from_type(
            ty: &ScriptType,
            out: &mut BTreeMap<String, Vec<String>>,
        ) {
            match ty {
                ScriptType::Enum { type_name, members } => {
                    out.entry(type_name.clone())
                        .or_insert_with(|| members.clone());
                    if let Some((_, short)) = type_name.rsplit_once('.') {
                        out.entry(short.to_string())
                            .or_insert_with(|| members.clone());
                    }
                }
                ScriptType::Array { element_type } => {
                    collect_enum_members_from_type(element_type, out);
                }
                ScriptType::Map { value_type, .. } => {
                    collect_enum_members_from_type(value_type, out);
                }
                ScriptType::Object { fields, .. } => {
                    for field_type in fields.values() {
                        collect_enum_members_from_type(field_type, out);
                    }
                }
                ScriptType::Primitive { .. } | ScriptType::Script | ScriptType::Function => {}
            }
        }

        let mut enum_members_by_name: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for script in options.scripts.values() {
            for param in &script.params {
                collect_enum_members_from_type(&param.r#type, &mut enum_members_by_name);
            }
            for group in script.groups.values() {
                for node in &group.nodes {
                    if let ScriptNode::Var { declaration, .. } = node {
                        collect_enum_members_from_type(
                            &declaration.r#type,
                            &mut enum_members_by_name,
                        );
                    }
                }
            }
            for function in script.visible_functions.values() {
                for param in &function.params {
                    collect_enum_members_from_type(&param.r#type, &mut enum_members_by_name);
                }
                collect_enum_members_from_type(
                    &function.return_binding.r#type,
                    &mut enum_members_by_name,
                );
            }
            for decl in script.visible_module_vars.values() {
                collect_enum_members_from_type(&decl.r#type, &mut enum_members_by_name);
            }
            for decl in script.visible_module_consts.values() {
                collect_enum_members_from_type(&decl.r#type, &mut enum_members_by_name);
            }
        }
        for decl in options.module_var_declarations.values() {
            collect_enum_members_from_type(&decl.r#type, &mut enum_members_by_name);
        }
        for decl in options.module_const_declarations.values() {
            collect_enum_members_from_type(&decl.r#type, &mut enum_members_by_name);
        }
        rhai_engine.register_fn(
            "all_enum_members",
            move |enum_name: ImmutableString| -> Result<Array, Box<EvalAltResult>> {
                let Some(members) = enum_members_by_name.get(enum_name.as_str()) else {
                    return Err(Box::new(EvalAltResult::ErrorRuntime(
                        Dynamic::from(format!(
                            "all_enum_members(enumName) unknown enum type \"{}\".",
                            enum_name
                        )),
                        Position::NONE,
                    )));
                };
                Ok(members
                    .iter()
                    .map(|member| Dynamic::from(member.clone()))
                    .collect())
            },
        );

        let module_vars_type = options
            .module_var_declarations
            .iter()
            .map(|(qualified_name, decl)| (qualified_name.clone(), decl.r#type.clone()))
            .collect();
        let module_consts_type = options
            .module_const_declarations
            .iter()
            .map(|(qualified_name, decl)| (qualified_name.clone(), decl.r#type.clone()))
            .collect();
        Ok(Self {
            scripts: options.scripts,
            host_functions,
            compiler_version: options
                .compiler_version
                .unwrap_or_else(|| DEFAULT_COMPILER_VERSION.to_string()),
            group_lookup,
            global_data: options.global_data,
            module_var_declarations: options.module_var_declarations,
            module_var_init_order: options.module_var_init_order,
            module_const_declarations: options.module_const_declarations,
            module_const_init_order: options.module_const_init_order,
            module_vars_value: BTreeMap::new(),
            module_vars_type,
            module_consts_value: BTreeMap::new(),
            module_consts_type,
            visible_globals_by_script,
            visible_module_by_script,
            module_global_alias_by_script,
            visible_consts_by_script,
            module_const_alias_by_script,
            visible_function_symbols_by_script,
            invoke_all_functions,
            invoke_public_functions,
            invoke_function_symbols,
            module_prelude_by_script: HashMap::new(),
            initial_random_seed,
            initial_random_sequence,
            rhai_engine,
            shared_rng_state,
            frames: Vec::new(),
            pending_boundary: None,
            waiting_choice: false,
            ended: false,
            frame_counter: 1,
            seeded_rng_state: initial_random_seed,
            once_state_by_script: BTreeMap::new(),
        })
    }

    pub fn random_state_snapshot(&self) -> RandomStateView {
        match &*self.shared_rng_state.borrow() {
            RuntimeRandomState::Seeded(state) => RandomStateView::Seeded { state: *state },
            RuntimeRandomState::Sequence { values, index } => RandomStateView::Sequence {
                values: values.clone(),
                index: *index,
            },
        }
    }

    pub(super) fn current_seeded_rng_state(&self) -> u32 {
        match &*self.shared_rng_state.borrow() {
            RuntimeRandomState::Seeded(state) => *state,
            RuntimeRandomState::Sequence { .. } => self.seeded_rng_state,
        }
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
        self.initialize_module_consts()?;
        self.initialize_module_vars()?;
        let Some(script) = self.scripts.get(entry_script_name) else {
            return Err(ScriptLangError::new(
                "ENGINE_SCRIPT_NOT_FOUND",
                format!("Entry script \"{}\" is not registered.", entry_script_name),
            ));
        };
        if script.access == AccessLevel::Private {
            return Err(ScriptLangError::new(
                "ENGINE_ENTRY_SCRIPT_PRIVATE",
                format!(
                    "Entry script \"{}\" is private and cannot be started by host.",
                    entry_script_name
                ),
            ));
        }
        let root_group_id = script.root_group_id.clone();
        let (scope, var_types) =
            self.create_script_root_scope(entry_script_name, entry_args.unwrap_or_default())?;
        self.push_root_frame(&root_group_id, scope, None, var_types);
        Ok(())
    }

    pub(super) fn initialize_module_vars(&mut self) -> Result<(), ScriptLangError> {
        self.module_vars_value.clear();
        for qualified_name in self.module_var_init_order.clone() {
            let decl = self
                .module_var_declarations
                .get(&qualified_name)
                .cloned()
                .ok_or_else(|| ScriptLangError::new(
                        "ENGINE_MODULE_GLOBAL_DECL_MISSING",
                        format!(
                            "Module global \"{}\" is present in init order but missing from declarations.",
                            qualified_name
                        ),
                    ))?;
            let mut value = default_value_from_type(&decl.r#type);
            if let Some(expr) = &decl.initial_value_expr {
                value = self.eval_module_global_initializer(expr, &decl.namespace)?;
            } else if matches!(decl.r#type, ScriptType::Enum { .. }) {
                return Err(ScriptLangError::new(
                    "ENGINE_ENUM_INIT_REQUIRED",
                    format!(
                        "Module global \"{}\" with enum type requires explicit Type.Member initializer.",
                        qualified_name
                    ),
                ));
            }
            if !is_type_compatible(&value, &decl.r#type) {
                return Err(ScriptLangError::new(
                    "ENGINE_TYPE_MISMATCH",
                    format!(
                        "Module global \"{}\" does not match declared type.",
                        qualified_name
                    ),
                ));
            }
            self.module_vars_value.insert(qualified_name, value);
        }
        for (qualified_name, decl) in &self.module_var_declarations {
            if self.module_vars_value.contains_key(qualified_name) {
                continue;
            }
            self.module_vars_value.insert(
                qualified_name.clone(),
                default_value_from_type(&decl.r#type),
            );
        }
        Ok(())
    }

    pub(super) fn initialize_module_consts(&mut self) -> Result<(), ScriptLangError> {
        self.module_consts_value.clear();
        for qualified_name in self.module_const_init_order.clone() {
            let decl = self
                .module_const_declarations
                .get(&qualified_name)
                .cloned()
                .ok_or_else(|| {
                    ScriptLangError::new(
                        "ENGINE_MODULE_CONST_DECL_MISSING",
                        format!(
                            "Module const \"{}\" is present in init order but missing from declarations.",
                            qualified_name
                        ),
                    )
                })?;
            let mut value = default_value_from_type(&decl.r#type);
            if let Some(expr) = &decl.initial_value_expr {
                value = self.eval_module_const_initializer(expr, &decl.namespace)?;
            } else if matches!(decl.r#type, ScriptType::Enum { .. }) {
                return Err(ScriptLangError::new(
                    "ENGINE_ENUM_INIT_REQUIRED",
                    format!(
                        "Module const \"{}\" with enum type requires explicit Type.Member initializer.",
                        qualified_name
                    ),
                ));
            }
            if !is_type_compatible(&value, &decl.r#type) {
                return Err(ScriptLangError::new(
                    "ENGINE_TYPE_MISMATCH",
                    format!(
                        "Module const \"{}\" does not match declared type.",
                        qualified_name
                    ),
                ));
            }
            self.module_consts_value.insert(qualified_name, value);
        }
        for (qualified_name, decl) in &self.module_const_declarations {
            if self.module_consts_value.contains_key(qualified_name) {
                continue;
            }
            self.module_consts_value.insert(
                qualified_name.clone(),
                default_value_from_type(&decl.r#type),
            );
        }
        Ok(())
    }
}
#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use crate::engine::runtime_test_support::*;
    use sl_core::SourceSpan;

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

    fn random_state_kind(view: &RandomStateView) -> &'static str {
        match view {
            RandomStateView::Seeded { .. } => "seeded",
            RandomStateView::Sequence { .. } => "sequence",
        }
    }

    #[test]
    pub(super) fn new_rejects_reserved_host_function_name_random() {
        let files = map(&[(
            "main.script.xml",
            r#"<script name="main"><text>Hello</text></script>"#,
        )]);
        let compiled = compile_project_from_sources(files);
        let result = ScriptLangEngine::new(ScriptLangEngineOptions {
            scripts: compiled.scripts,
            global_data: compiled.global_data,
            module_var_declarations: compiled.module_var_declarations,
            module_var_init_order: compiled.module_var_init_order,
            module_const_declarations: compiled.module_const_declarations,
            module_const_init_order: compiled.module_const_init_order,
            host_functions: Some(Arc::new(TestRegistry {
                names: vec!["random".to_string()],
            })),
            random_seed: Some(1),
            random_sequence: None,
            random_sequence_index: None,
            compiler_version: Some(DEFAULT_COMPILER_VERSION.to_string()),
        });
        assert!(result.is_err());
        let error = result.err().expect("reserved random name should fail");
        assert_eq!(error.code, "ENGINE_HOST_FUNCTION_RESERVED");
    }

    #[test]
    pub(super) fn new_rejects_reserved_host_function_name_invoke() {
        let files = map(&[(
            "main.script.xml",
            r#"<script name="main"><text>Hello</text></script>"#,
        )]);
        let compiled = compile_project_from_sources(files);
        let result = ScriptLangEngine::new(ScriptLangEngineOptions {
            scripts: compiled.scripts,
            global_data: compiled.global_data,
            module_var_declarations: compiled.module_var_declarations,
            module_var_init_order: compiled.module_var_init_order,
            module_const_declarations: compiled.module_const_declarations,
            module_const_init_order: compiled.module_const_init_order,
            host_functions: Some(Arc::new(TestRegistry {
                names: vec!["invoke".to_string()],
            })),
            random_seed: Some(1),
            random_sequence: None,
            random_sequence_index: None,
            compiler_version: Some(DEFAULT_COMPILER_VERSION.to_string()),
        });
        assert!(result.is_err());
        let error = result.err().expect("reserved invoke name should fail");
        assert_eq!(error.code, "ENGINE_HOST_FUNCTION_RESERVED");
    }

    #[test]
    pub(super) fn new_rejects_reserved_host_function_name_enum_to_string() {
        let files = map(&[(
            "main.script.xml",
            r#"<script name="main"><text>Hello</text></script>"#,
        )]);
        let compiled = compile_project_from_sources(files);
        let result = ScriptLangEngine::new(ScriptLangEngineOptions {
            scripts: compiled.scripts,
            global_data: compiled.global_data,
            module_var_declarations: compiled.module_var_declarations,
            module_var_init_order: compiled.module_var_init_order,
            module_const_declarations: compiled.module_const_declarations,
            module_const_init_order: compiled.module_const_init_order,
            host_functions: Some(Arc::new(TestRegistry {
                names: vec!["enum_to_string".to_string()],
            })),
            random_seed: Some(1),
            random_sequence: None,
            random_sequence_index: None,
            compiler_version: Some(DEFAULT_COMPILER_VERSION.to_string()),
        });
        assert!(result.is_err());
        let error = result
            .err()
            .expect("reserved enum_to_string name should fail");
        assert_eq!(error.code, "ENGINE_HOST_FUNCTION_RESERVED");
    }

    #[test]
    pub(super) fn new_rejects_reserved_host_function_name_all_enum_members() {
        let files = map(&[(
            "main.script.xml",
            r#"<script name="main"><text>Hello</text></script>"#,
        )]);
        let compiled = compile_project_from_sources(files);
        let result = ScriptLangEngine::new(ScriptLangEngineOptions {
            scripts: compiled.scripts,
            global_data: compiled.global_data,
            module_var_declarations: compiled.module_var_declarations,
            module_var_init_order: compiled.module_var_init_order,
            module_const_declarations: compiled.module_const_declarations,
            module_const_init_order: compiled.module_const_init_order,
            host_functions: Some(Arc::new(TestRegistry {
                names: vec!["all_enum_members".to_string()],
            })),
            random_seed: Some(1),
            random_sequence: None,
            random_sequence_index: None,
            compiler_version: Some(DEFAULT_COMPILER_VERSION.to_string()),
        });
        assert!(result.is_err());
        let error = result
            .err()
            .expect("reserved all_enum_members name should fail");
        assert_eq!(error.code, "ENGINE_HOST_FUNCTION_RESERVED");
    }

    #[test]
    pub(super) fn new_rejects_host_function_conflicting_with_module_function() {
        let files = map(&[
            (
                "main.script.xml",
                r#"
    <!-- import shared from shared.xml -->
    <script name="main"><text>Hello</text></script>
    "#,
            ),
            (
                "shared.xml",
                r#"
    <module name="shared" default_access="public">
      <function name="addWithGameBonus" args="int:a1,int:a2" returnType="int">
        return a1 + a2;
      </function>
    </module>
    "#,
            ),
        ]);
        let compiled = compile_project_from_sources(files);
        let result = ScriptLangEngine::new(ScriptLangEngineOptions {
            scripts: compiled.scripts,
            global_data: compiled.global_data,
            module_var_declarations: compiled.module_var_declarations,
            module_var_init_order: compiled.module_var_init_order,
            module_const_declarations: compiled.module_const_declarations,
            module_const_init_order: compiled.module_const_init_order,
            host_functions: Some(Arc::new(TestRegistry {
                names: vec!["shared.addWithGameBonus".to_string()],
            })),
            random_seed: Some(1),
            random_sequence: None,
            random_sequence_index: None,
            compiler_version: Some(DEFAULT_COMPILER_VERSION.to_string()),
        });
        assert!(result.is_err());
        let error = result
            .err()
            .expect("conflicting module function should fail");
        assert_eq!(error.code, "ENGINE_HOST_FUNCTION_CONFLICT");
    }

    #[test]
    pub(super) fn new_rejects_module_function_named_invoke() {
        let files = map(&[(
            "main.xml",
            r#"
<module name="main" default_access="public">
  <function name="invoke" returnType="int">return 1;</function>
  <script name="main"><text>ok</text></script>
</module>
"#,
        )]);
        let compiled = compile_project_from_sources(files);
        let result = ScriptLangEngine::new(ScriptLangEngineOptions {
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
            compiler_version: Some(DEFAULT_COMPILER_VERSION.to_string()),
        });
        assert!(result.is_err());
        let error = result
            .err()
            .expect("module function invoke should be reserved");
        assert_eq!(error.code, "ENGINE_MODULE_FUNCTION_RESERVED");
    }

    #[test]
    pub(super) fn new_rejects_module_function_symbol_conflict_after_normalization() {
        // Test lines 277-287: when two functions have names that normalize to the same Rhai symbol
        // "foo-bar" and "foo_bar" both become "foo_bar" after symbol normalization
        // We use same module prefix so they normalize to the same symbol
        let files = map(&[(
            "main.xml",
            r#"
    <module name="main" default_access="public">
      <function name="foo-bar" returnType="int">return 1;</function>
      <function name="foo_bar" returnType="int">return 2;</function>
      <script name="main"><text>Hello</text></script>
    </module>
    "#,
        )]);
        let compiled = compile_project_from_sources(files);
        let result = ScriptLangEngine::new(ScriptLangEngineOptions {
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
            compiler_version: Some(DEFAULT_COMPILER_VERSION.to_string()),
        });
        let error = result
            .err()
            .expect("normalized symbol conflict should fail");
        assert_eq!(error.code, "ENGINE_MODULE_FUNCTION_SYMBOL_CONFLICT");
    }

    #[test]
    pub(super) fn random_function_success_and_registry_call_path_are_covered() {
        let files = map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <temp name="n" type="int">random(5)</temp>
      <text>${n}</text>
    </script>
    "#,
        )]);
        let mut engine = engine_from_sources(files);
        engine.start("main", None).expect("start");
        let output = engine.next_output().expect("next");
        assert_eq!(output_kind(&output), "text");

        let registry = TestRegistry {
            names: vec!["ok".to_string()],
        };
        let value = registry.call("ok", &[]).expect("call should succeed");
        assert_eq!(value, SlValue::Bool(true));
    }

    #[test]
    pub(super) fn enum_builtin_functions_are_available() {
        let files = map(&[(
            "main.xml",
            r#"
    <module name="main" default_access="public">
      <enum name="State">
        <member name="Idle"/>
        <member name="Run"/>
      </enum>
      <script name="main">
        <temp name="state" type="State">State.Run</temp>
        <temp name="label" type="string">enum_to_string(state)</temp>
        <temp name="members" type="string[]">all_enum_members("State")</temp>
        <text>${label}:${members[0]},${members[1]}</text>
      </script>
    </module>
    "#,
        )]);
        let mut engine = engine_from_sources(files);
        engine.start("main.main", None).expect("start");
        let output = engine.next_output().expect("next");
        assert!(matches!(output, EngineOutput::Text { text, .. } if text == "Run:Idle,Run"));
    }

    #[test]
    pub(super) fn random_sequence_returns_values_in_order_and_modulo_bound() {
        let files = map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <temp name="a" type="int">random(5)</temp>
      <text>${a}</text>
      <temp name="b" type="int">random(5)</temp>
      <text>${b}</text>
      <temp name="c" type="int">random(5)</temp>
      <text>${c}</text>
    </script>
    "#,
        )]);
        let compiled = compile_project_from_sources(files);
        let mut engine = ScriptLangEngine::new(ScriptLangEngineOptions {
            scripts: compiled.scripts,
            global_data: compiled.global_data,
            module_var_declarations: compiled.module_var_declarations,
            module_var_init_order: compiled.module_var_init_order,
            module_const_declarations: compiled.module_const_declarations,
            module_const_init_order: compiled.module_const_init_order,
            host_functions: None,
            random_seed: Some(1),
            random_sequence: Some(vec![12, 3, 1]),
            random_sequence_index: Some(0),
            compiler_version: Some(DEFAULT_COMPILER_VERSION.to_string()),
        })
        .expect("new engine");
        engine.start("main", None).expect("start");
        let a = engine.next_output().expect("a");
        let b = engine.next_output().expect("b");
        let c = engine.next_output().expect("c");
        assert_eq!(
            a,
            EngineOutput::Text {
                text: "2".to_string(),
                tag: None
            }
        );
        assert_eq!(
            b,
            EngineOutput::Text {
                text: "3".to_string(),
                tag: None
            }
        );
        assert_eq!(
            c,
            EngineOutput::Text {
                text: "1".to_string(),
                tag: None
            }
        );
    }

    #[test]
    pub(super) fn random_sequence_returns_zero_after_exhausted() {
        let files = map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <temp name="a" type="int">random(7)</temp>
      <text>${a}</text>
      <temp name="b" type="int">random(7)</temp>
      <text>${b}</text>
      <temp name="c" type="int">random(7)</temp>
      <text>${c}</text>
    </script>
    "#,
        )]);
        let compiled = compile_project_from_sources(files);
        let mut engine = ScriptLangEngine::new(ScriptLangEngineOptions {
            scripts: compiled.scripts,
            global_data: compiled.global_data,
            module_var_declarations: compiled.module_var_declarations,
            module_var_init_order: compiled.module_var_init_order,
            module_const_declarations: compiled.module_const_declarations,
            module_const_init_order: compiled.module_const_init_order,
            host_functions: None,
            random_seed: Some(1),
            random_sequence: Some(vec![5]),
            random_sequence_index: Some(0),
            compiler_version: Some(DEFAULT_COMPILER_VERSION.to_string()),
        })
        .expect("new engine");
        engine.start("main", None).expect("start");
        let a = engine.next_output().expect("a");
        let b = engine.next_output().expect("b");
        let c = engine.next_output().expect("c");
        assert_eq!(
            a,
            EngineOutput::Text {
                text: "5".to_string(),
                tag: None
            }
        );
        assert_eq!(
            b,
            EngineOutput::Text {
                text: "0".to_string(),
                tag: None
            }
        );
        assert_eq!(
            c,
            EngineOutput::Text {
                text: "0".to_string(),
                tag: None
            }
        );
    }

    #[test]
    pub(super) fn random_state_snapshot_covers_seeded_and_sequence_modes() {
        let mut seeded = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>x</text></script>"#,
        )]));
        seeded.start("main", None).expect("start");
        let seeded_view = seeded.random_state_snapshot();
        assert!(matches!(seeded_view, RandomStateView::Seeded { state } if state == 1));
        assert_eq!(seeded.current_seeded_rng_state(), 1);

        let files = map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <temp name="a" type="int">random(5)</temp>
      <text>${a}</text>
    </script>
    "#,
        )]);
        let compiled = compile_project_from_sources(files);
        let mut sequence = ScriptLangEngine::new(ScriptLangEngineOptions {
            scripts: compiled.scripts,
            global_data: compiled.global_data,
            module_var_declarations: compiled.module_var_declarations,
            module_var_init_order: compiled.module_var_init_order,
            module_const_declarations: compiled.module_const_declarations,
            module_const_init_order: compiled.module_const_init_order,
            host_functions: None,
            random_seed: Some(9),
            random_sequence: Some(vec![12, 3]),
            random_sequence_index: Some(1),
            compiler_version: Some(DEFAULT_COMPILER_VERSION.to_string()),
        })
        .expect("new");
        sequence.start("main", None).expect("start");
        let sequence_view = sequence.random_state_snapshot();
        assert!(matches!(
            &sequence_view,
            RandomStateView::Sequence { values, index } if values == &vec![12, 3] && *index == 0
        ));
        let sequence_view_for_seeded = sequence.random_state_snapshot();
        assert_eq!(random_state_kind(&sequence_view_for_seeded), "sequence");
        let seeded_view_for_fallback = seeded.random_state_snapshot();
        assert_eq!(random_state_kind(&seeded_view_for_fallback), "seeded");
        assert_eq!(sequence.current_seeded_rng_state(), 9);
    }

    #[test]
    pub(super) fn new_success_path_initializes_module_and_function_symbols() {
        let files = map(&[
            (
                "main.script.xml",
                r#"
    <!-- import shared from shared.xml -->
    <script name="main"><text>ok</text></script>
    "#,
            ),
            (
                "shared.xml",
                r#"
    <module name="shared" default_access="public">
      <var name="hp" type="int">1</var>
      <function name="addWithGameBonus" args="int:a1,int:a2" returnType="int">
    return a1 + a2;
      </function>
    </module>
    "#,
            ),
        ]);
        let compiled = compile_project_from_sources(files);
        let mut engine = ScriptLangEngine::new(ScriptLangEngineOptions {
            scripts: compiled.scripts,
            global_data: compiled.global_data,
            module_var_declarations: compiled.module_var_declarations,
            module_var_init_order: compiled.module_var_init_order,
            module_const_declarations: compiled.module_const_declarations,
            module_const_init_order: compiled.module_const_init_order,
            host_functions: None,
            random_seed: Some(7),
            random_sequence: None,
            random_sequence_index: None,
            compiler_version: None,
        })
        .expect("new should succeed");

        assert_eq!(engine.compiler_version(), DEFAULT_COMPILER_VERSION);
        assert!(!engine.waiting_choice());
        assert!(!engine.ended);
        assert!(
            engine.module_vars_type.contains_key("shared.hp"),
            "module global type should be initialized"
        );
        assert_eq!(
            engine
                .visible_function_symbols_by_script
                .get("main")
                .and_then(|m| m.get("shared.addWithGameBonus"))
                .map(String::as_str),
            Some("shared_addWithGameBonus")
        );

        engine.start("main", None).expect("start");
        assert_eq!(
            engine.module_vars_value.get("shared.hp"),
            Some(&SlValue::Number(1.0))
        );
    }

    #[test]
    pub(super) fn start_returns_error_for_missing_script() {
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
    pub(super) fn start_rejects_private_entry_script() {
        let mut engine = engine_from_sources(map(&[(
            "main.xml",
            r#"<module name="main"><script name="main"><text>Hello</text></script></module>"#,
        )]));
        let error = engine
            .start("main.main", None)
            .expect_err("private entry should fail");
        assert_eq!(error.code, "ENGINE_ENTRY_SCRIPT_PRIVATE");
    }

    #[test]
    pub(super) fn start_rejects_module_global_initializer_type_mismatch() {
        let mut engine = engine_from_sources(map(&[
            (
                "shared.xml",
                r#"
    <module name="shared" default_access="public">
      <var name="hp" type="int">"bad"</var>
    </module>
    "#,
            ),
            (
                "main.script.xml",
                r#"
    <!-- import shared from shared.xml -->
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
    pub(super) fn start_rejects_module_const_initializer_type_mismatch() {
        let mut engine = engine_from_sources(map(&[(
            "main.xml",
            r#"<module name="main" default_access="public">
  <const name="base" type="int">"bad"</const>
  <script name="main"><text>ok</text></script>
</module>"#,
        )]));
        let error = engine
            .start("main.main", None)
            .expect_err("const type mismatch should fail");
        assert_eq!(error.code, "ENGINE_TYPE_MISMATCH");
    }

    #[test]
    pub(super) fn initialize_module_consts_reports_missing_decl() {
        let mut engine = engine_from_sources(map(&[(
            "main.xml",
            r#"<module name="main" default_access="public"><script name="main"><text>ok</text></script></module>"#,
        )]));
        engine.module_const_init_order = vec!["main.base".to_string()];
        let error = engine
            .initialize_module_consts()
            .expect_err("missing decl should fail");
        assert_eq!(error.code, "ENGINE_MODULE_CONST_DECL_MISSING");
    }

    #[test]
    pub(super) fn start_rejects_missing_module_global_decl_in_init_order() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>ok</text></script>"#,
        )]));
        engine.module_var_init_order = vec!["shared.hp".to_string()];
        let error = engine
            .start("main", None)
            .expect_err("missing decl in init order should fail");
        assert_eq!(error.code, "ENGINE_MODULE_GLOBAL_DECL_MISSING");
    }

    #[test]
    pub(super) fn start_fills_default_for_module_global_not_present_in_init_order() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>ok</text></script>"#,
        )]));
        engine.module_var_declarations.insert(
            "shared.hp".to_string(),
            ModuleVarDecl {
                namespace: "shared".to_string(),
                name: "hp".to_string(),
                qualified_name: "shared.hp".to_string(),
                access: AccessLevel::Private,
                r#type: ScriptType::Primitive {
                    name: "int".to_string(),
                },
                initial_value_expr: None,
                location: sl_core::SourceSpan::synthetic(),
            },
        );
        engine.module_var_init_order.clear();

        engine.start("main", None).expect("start");
        assert_eq!(
            engine.module_vars_value.get("shared.hp"),
            Some(&SlValue::Number(0.0))
        );
    }

    #[test]
    pub(super) fn public_state_accessors_and_empty_registry_are_covered() {
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
        assert_eq!(output_kind(&next), "choices");
        assert!(engine.waiting_choice());
        assert_eq!(
            output_kind(&EngineOutput::Input {
                prompt_text: "p".to_string(),
                default_text: "d".to_string()
            }),
            "input"
        );
        assert_eq!(output_kind(&EngineOutput::End), "end");
    }

    #[test]
    pub(super) fn random_function_error_path_is_covered() {
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
        assert_eq!(error.code, "ENGINE_EVAL_ERROR");
    }

    #[test]
    pub(super) fn start_accepts_explicit_entry_args_map() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main" args="int:x"><text>${x}</text></script>"#,
        )]));
        engine
            .start(
                "main",
                Some(BTreeMap::from([("x".to_string(), SlValue::Number(7.0))])),
            )
            .expect("start with explicit args should succeed");
        let output = engine.next_output().expect("text");
        assert_eq!(
            output,
            EngineOutput::Text {
                text: "7".to_string(),
                tag: None
            }
        );
    }

    #[test]
    pub(super) fn start_rejects_explicit_entry_args_type_mismatch() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main" args="int:x"><text>${x}</text></script>"#,
        )]));
        let error = engine
            .start(
                "main",
                Some(BTreeMap::from([(
                    "x".to_string(),
                    SlValue::String("bad".to_string()),
                )])),
            )
            .expect_err("start with invalid arg type should fail");
        assert_eq!(error.code, "ENGINE_TYPE_MISMATCH");
    }

    #[test]
    pub(super) fn initialize_module_consts_uses_default_when_no_initializer() {
        // Test line 442: when module const has no initial_value_expr, uses default value
        let mut engine = engine_from_sources(map(&[(
            "main.xml",
            r#"<module name="main" default_access="public"><script name="main"><text>ok</text></script></module>"#,
        )]));
        // Manually add a module const without initial_value_expr
        engine.module_const_declarations.insert(
            "main.count".to_string(),
            ModuleConstDecl {
                namespace: "main".to_string(),
                name: "count".to_string(),
                qualified_name: "main.count".to_string(),
                r#type: ScriptType::Primitive {
                    name: "int".to_string(),
                },
                initial_value_expr: None, // No initializer - should use default
                access: AccessLevel::Public,
                location: SourceSpan::synthetic(),
            },
        );
        engine.module_const_init_order = vec!["main.count".to_string()];
        engine
            .initialize_module_consts()
            .expect("init should succeed");
        // Verify default value is 0
        let value = engine
            .module_consts_value
            .get("main.count")
            .expect("should exist");
        assert_eq!(*value, SlValue::Number(0.0));
    }

    #[test]
    pub(super) fn initialize_module_consts_requires_enum_initializer() {
        let mut engine = engine_from_sources(map(&[(
            "main.xml",
            r#"<module name="main" default_access="public"><script name="main"><text>ok</text></script></module>"#,
        )]));
        engine.module_const_declarations.insert(
            "main.state".to_string(),
            ModuleConstDecl {
                namespace: "main".to_string(),
                name: "state".to_string(),
                qualified_name: "main.state".to_string(),
                r#type: ScriptType::Enum {
                    type_name: "State".to_string(),
                    members: vec!["Idle".to_string(), "Run".to_string()],
                },
                initial_value_expr: None,
                access: AccessLevel::Public,
                location: SourceSpan::synthetic(),
            },
        );
        engine.module_const_init_order = vec!["main.state".to_string()];
        let error = engine
            .initialize_module_consts()
            .expect_err("enum const without initializer should fail");
        assert_eq!(error.code, "ENGINE_ENUM_INIT_REQUIRED");
    }

    #[test]
    pub(super) fn initialize_module_vars_requires_enum_initializer() {
        let mut engine = engine_from_sources(map(&[(
            "main.xml",
            r#"<module name="main" default_access="public"><script name="main"><text>ok</text></script></module>"#,
        )]));
        engine.module_var_declarations.insert(
            "main.state".to_string(),
            ModuleVarDecl {
                namespace: "main".to_string(),
                name: "state".to_string(),
                qualified_name: "main.state".to_string(),
                r#type: ScriptType::Enum {
                    type_name: "State".to_string(),
                    members: vec!["Idle".to_string(), "Run".to_string()],
                },
                initial_value_expr: None,
                access: AccessLevel::Public,
                location: SourceSpan::synthetic(),
            },
        );
        engine.module_var_init_order = vec!["main.state".to_string()];
        let error = engine
            .initialize_module_vars()
            .expect_err("enum global without initializer should fail");
        assert_eq!(error.code, "ENGINE_ENUM_INIT_REQUIRED");
    }

    #[test]
    pub(super) fn initialize_module_consts_handles_missing_const_in_order() {
        // Test line 441: when module const initializer references missing variable
        let mut engine = engine_from_sources(map(&[(
            "main.xml",
            r#"<module name="main" default_access="public"><script name="main"><text>ok</text></script></module>"#,
        )]));
        // Add module const with invalid initializer
        engine.module_const_declarations.insert(
            "main.bad".to_string(),
            ModuleConstDecl {
                namespace: "main".to_string(),
                name: "bad".to_string(),
                qualified_name: "main.bad".to_string(),
                r#type: ScriptType::Primitive {
                    name: "int".to_string(),
                },
                initial_value_expr: Some("nonexistent + 1".to_string()), // Invalid: references undefined
                access: AccessLevel::Public,
                location: SourceSpan::synthetic(),
            },
        );
        engine.module_const_init_order = vec!["main.bad".to_string()];
        let error = engine
            .initialize_module_consts()
            .expect_err("invalid initializer should fail");
        assert_eq!(error.code, "ENGINE_EVAL_ERROR");
    }

    #[test]
    pub(super) fn initialize_module_consts_with_const_not_in_init_order() {
        // Test lines 457-460: module const not in init_order uses default value
        let mut engine = engine_from_sources(map(&[(
            "main.xml",
            r#"<module name="main" default_access="public"><script name="main"><text>ok</text></script></module>"#,
        )]));
        // Add two module consts: one in init_order, one not
        engine.module_const_declarations.insert(
            "main.uninitialized".to_string(),
            ModuleConstDecl {
                namespace: "main".to_string(),
                name: "uninitialized".to_string(),
                qualified_name: "main.uninitialized".to_string(),
                r#type: ScriptType::Primitive {
                    name: "int".to_string(),
                },
                initial_value_expr: None, // Not in init_order - should use default
                access: AccessLevel::Public,
                location: SourceSpan::synthetic(),
            },
        );
        engine.module_const_declarations.insert(
            "main.initialized".to_string(),
            ModuleConstDecl {
                namespace: "main".to_string(),
                name: "initialized".to_string(),
                qualified_name: "main.initialized".to_string(),
                r#type: ScriptType::Primitive {
                    name: "int".to_string(),
                },
                initial_value_expr: Some("42".to_string()),
                access: AccessLevel::Public,
                location: SourceSpan::synthetic(),
            },
        );
        // Only initialize the second one
        engine.module_const_init_order = vec!["main.initialized".to_string()];
        engine
            .initialize_module_consts()
            .expect("init should succeed");
        // uninitialized should be 0 (default), initialized should be 42
        let uninit = engine
            .module_consts_value
            .get("main.uninitialized")
            .expect("should exist");
        let init = engine
            .module_consts_value
            .get("main.initialized")
            .expect("should exist");
        assert_eq!(*uninit, SlValue::Number(0.0));
        assert_eq!(*init, SlValue::Number(42.0));
    }

    #[test]
    pub(super) fn all_enum_members_unknown_enum_fails() {
        // 383:28, 383:32 - Test error path when enum not found
        let files = map(&[(
            "main.script.xml",
            r#"
    <module name="main" default_access="public">
      <enum name="State">
        <member name="Idle"/>
        <member name="Run"/>
      </enum>
      <script name="main">
        <temp name="members" type="string[]">all_enum_members("NonExistent")</temp>
        <text>${members}</text>
      </script>
    </module>
    "#,
        )]);
        let mut engine = engine_from_sources(files);
        engine
            .start("main.main", None)
            .expect("start should succeed");
        let error = engine
            .next_output()
            .expect_err("all_enum_members with unknown enum should fail");
        assert_eq!(error.code, "ENGINE_EVAL_ERROR");
        assert!(error.message.contains("unknown enum type"));
    }

    #[test]
    pub(super) fn enum_builtin_with_namespaced_enum_type() {
        // 325:21 - Test that namespaced enum type (with '.') is handled correctly
        // When type_name contains '.', rsplit_once returns Some and adds short alias
        let files = map(&[
            (
                "shared.xml",
                r#"
    <module name="shared" default_access="public">
      <enum name="State">
        <member name="Idle"/>
        <member name="Run"/>
      </enum>
    </module>
    "#,
            ),
            (
                "main.xml",
                r#"
    <module name="main" default_access="public">
      <!-- import shared from shared.xml -->
      <script name="main">
        <!-- Use namespaced enum type: shared.State -->
        <temp name="state" type="shared.State">shared.State.Run</temp>
        <temp name="label" type="string">enum_to_string(state)</temp>
        <temp name="members" type="string[]">all_enum_members("shared.State")</temp>
        <text>${label}:${members[0]},${members[1]}</text>
      </script>
    </module>
    "#,
            ),
        ]);
        let mut engine = engine_from_sources(files);
        engine.start("main.main", None).expect("start");
        let output = engine.next_output().expect("next");
        assert!(matches!(output, EngineOutput::Text { text, .. } if text == "Run:Idle,Run"));
    }

    #[test]
    pub(super) fn enum_builtin_with_module_var_namespaced_enum_type() {
        // 325:21 - Test that namespaced enum type (with '.') from module_var triggers the branch
        // When type_name contains '.', rsplit_once returns Some and adds short alias
        let files = map(&[
            (
                "shared.xml",
                r#"
    <module name="shared" default_access="public">
      <enum name="Status">
        <member name="Idle"/>
        <member name="Run"/>
      </enum>
    </module>"#,
            ),
            (
                "main.xml",
                r#"
    <module name="main" default_access="public">
      <!-- import shared from shared.xml -->
      <var name="current_status" type="shared.Status">shared.Status.Idle</var>
      <script name="main">
        <temp name="label" type="string">enum_to_string(current_status)</temp>
        <temp name="members" type="string[]">all_enum_members("shared.Status")</temp>
        <text>${label}:${members[0]},${members[1]}</text>
      </script>
    </module>"#,
            ),
        ]);
        let mut engine = engine_from_sources(files);
        engine.start("main.main", None).expect("start");
        let output = engine.next_output().expect("next");
        // "shared.Status" contains '.', so short alias "Status" should also be registered
        // Verify short alias works by using "Status" instead of "shared.Status"
        assert!(matches!(output, EngineOutput::Text { text, .. } if text == "Idle:Idle,Run"));
    }

    #[test]
    pub(super) fn enum_builtin_short_alias_works_for_namespaced_enum() {
        // 325:21 - Test that short alias (without namespace) works for namespaced enum type
        // When type_name contains '.', rsplit_once adds short alias entry
        let files = map(&[
            (
                "shared.xml",
                r#"
    <module name="shared" default_access="public">
      <enum name="Status">
        <member name="Idle"/>
        <member name="Run"/>
      </enum>
    </module>"#,
            ),
            (
                "main.xml",
                r#"
    <module name="main" default_access="public">
      <!-- import shared from shared.xml -->
      <var name="current_status" type="shared.Status">shared.Status.Idle</var>
      <script name="main">
        <!-- Use short alias "Status" instead of "shared.Status" -->
        <temp name="label" type="string">enum_to_string(current_status)</temp>
        <temp name="members" type="string[]">all_enum_members("Status")</temp>
        <text>${label}:${members[0]},${members[1]}</text>
      </script>
    </module>"#,
            ),
        ]);
        let mut engine = engine_from_sources(files);
        engine.start("main.main", None).expect("start");
        let output = engine.next_output().expect("next");
        // Short alias "Status" should work because 325:21 adds it
        assert!(matches!(output, EngineOutput::Text { text, .. } if text == "Idle:Idle,Run"));
    }

    #[test]
    pub(super) fn enum_builtin_with_object_type_containing_enum_field() {
        // 333:38, 334:25, 334:39, 334:46, 334:55, 335:25 - Test Object type with fields
        // When ScriptType::Object has fields, it iterates through them to collect enum members
        let files = map(&[
            (
                "shared.xml",
                r#"
    <module name="shared" default_access="public">
      <enum name="Status">
        <member name="Active"/>
        <member name="Inactive"/>
      </enum>
      <type name="Player">
        <field name="status" type="Status"/>
        <field name="score" type="int"/>
      </type>
    </module>
    "#,
            ),
            (
                "main.xml",
                r#"
    <module name="main" default_access="public">
      <!-- import shared from shared.xml -->
      <script name="main">
        <!-- Use Object type with enum field: shared.Player -->
        <temp name="player" type="shared.Player">#{status: shared.Status.Active, score: 10}</temp>
        <temp name="statusLabel" type="string">enum_to_string(player.status)</temp>
        <temp name="allStatus" type="string[]">all_enum_members("shared.Status")</temp>
        <text>${statusLabel}:${allStatus[0]},${allStatus[1]}</text>
      </script>
    </module>
    "#,
            ),
        ]);
        let mut engine = engine_from_sources(files);
        engine.start("main.main", None).expect("start");
        let output = engine.next_output().expect("next");
        assert!(
            matches!(output, EngineOutput::Text { text, .. } if text == "Active:Active,Inactive")
        );
    }
}
