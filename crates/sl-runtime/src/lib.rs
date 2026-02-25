use std::cell::RefCell;
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

    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<EngineOutput, ScriptLangError> {
        if let Some(boundary) = &self.pending_boundary {
            return Ok(self.boundary_output(boundary));
        }

        if self.ended {
            return Ok(EngineOutput::End);
        }

        let mut guard = 0usize;
        while guard < 10_000 {
            guard += 1;

            let Some(top_frame) = self.frames.last().cloned() else {
                self.ended = true;
                return Ok(EngineOutput::End);
            };

            let (script_name, group) = self.lookup_group(&top_frame.group_id)?;

            if top_frame.node_index >= group.nodes.len() {
                self.finish_frame(top_frame.frame_id)?;
                continue;
            }

            let node = group.nodes[top_frame.node_index].clone();
            match node {
                ScriptNode::Text {
                    value, once, id, ..
                } => {
                    if once && self.has_once_state(&script_name, &format!("text:{}", id)) {
                        self.bump_top_node_index(1)?;
                        continue;
                    }

                    let rendered = self.render_text(&value)?;
                    self.bump_top_node_index(1)?;

                    if once {
                        self.mark_once_state(&script_name, &format!("text:{}", id));
                    }

                    return Ok(EngineOutput::Text { text: rendered });
                }
                ScriptNode::Code { code, .. } => {
                    self.run_code(&code)?;
                    self.bump_top_node_index(1)?;
                }
                ScriptNode::Var { declaration, .. } => {
                    self.execute_var_declaration(&declaration)?;
                    self.bump_top_node_index(1)?;
                }
                ScriptNode::If {
                    when_expr,
                    then_group_id,
                    else_group_id,
                    ..
                } => {
                    let condition = self.eval_boolean(&when_expr)?;
                    self.bump_top_node_index(1)?;
                    if condition {
                        self.push_group_frame(&then_group_id, CompletionKind::ResumeAfterChild)?;
                    } else if let Some(else_group_id) = else_group_id {
                        self.push_group_frame(&else_group_id, CompletionKind::ResumeAfterChild)?;
                    }
                }
                ScriptNode::While {
                    when_expr,
                    body_group_id,
                    ..
                } => {
                    let condition = self.eval_boolean(&when_expr)?;
                    if condition {
                        self.push_group_frame(&body_group_id, CompletionKind::WhileBody)?;
                    } else {
                        self.bump_top_node_index(1)?;
                    }
                }
                ScriptNode::Choice {
                    id,
                    options,
                    prompt_text,
                    ..
                } => {
                    let visible_regular = options
                        .iter()
                        .filter(|option| !option.fall_over)
                        .filter(|option| {
                            self.is_choice_option_visible(&script_name, option)
                                .unwrap_or(false)
                        })
                        .cloned()
                        .collect::<Vec<_>>();

                    let visible_options = if visible_regular.is_empty() {
                        options
                            .iter()
                            .find(|option| option.fall_over)
                            .filter(|option| {
                                self.is_choice_option_visible(&script_name, option)
                                    .unwrap_or(false)
                            })
                            .map(|option| vec![option.clone()])
                            .unwrap_or_default()
                    } else {
                        visible_regular
                    };

                    if visible_options.is_empty() {
                        self.bump_top_node_index(1)?;
                        continue;
                    }

                    let mut items = Vec::new();
                    for (index, option) in visible_options.iter().enumerate() {
                        items.push(ChoiceItem {
                            index,
                            id: option.id.clone(),
                            text: self.render_text(&option.text)?,
                        });
                    }

                    let prompt_text = Some(self.render_text(&prompt_text)?);
                    let frame_id = self.top_frame_id()?;
                    self.pending_boundary = Some(PendingBoundary::Choice {
                        frame_id,
                        node_id: id,
                        options: items.clone(),
                        prompt_text: prompt_text.clone(),
                    });
                    self.waiting_choice = true;
                    return Ok(EngineOutput::Choices { items, prompt_text });
                }
                ScriptNode::Input {
                    id,
                    target_var,
                    prompt_text,
                    ..
                } => {
                    let current = self.read_path(&target_var)?;
                    let SlValue::String(default_text) = current else {
                        return Err(ScriptLangError::new(
                            "ENGINE_INPUT_VAR_TYPE",
                            format!("Input target var \"{}\" must be string.", target_var),
                        ));
                    };

                    let frame_id = self.top_frame_id()?;
                    self.pending_boundary = Some(PendingBoundary::Input {
                        frame_id,
                        node_id: id,
                        target_var,
                        prompt_text: prompt_text.clone(),
                        default_text: default_text.clone(),
                    });
                    self.waiting_choice = false;
                    return Ok(EngineOutput::Input {
                        prompt_text,
                        default_text,
                    });
                }
                ScriptNode::Call {
                    target_script,
                    args,
                    ..
                } => {
                    self.execute_call(&target_script, &args)?;
                }
                ScriptNode::Return {
                    target_script,
                    args,
                    ..
                } => {
                    self.execute_return(target_script, &args)?;
                }
                ScriptNode::Break { .. } => {
                    self.execute_break()?;
                }
                ScriptNode::Continue { target, .. } => match target {
                    ContinueTarget::While => self.execute_continue_while()?,
                    ContinueTarget::Choice => self.execute_continue_choice()?,
                },
            }
        }

        Err(ScriptLangError::new(
            "ENGINE_GUARD_EXCEEDED",
            "Execution guard exceeded 10000 iterations.",
        ))
    }

    pub fn choose(&mut self, index: usize) -> Result<(), ScriptLangError> {
        let Some(PendingBoundary::Choice {
            frame_id,
            node_id: _,
            options,
            ..
        }) = self.pending_boundary.clone()
        else {
            return Err(ScriptLangError::new(
                "ENGINE_NO_PENDING_CHOICE",
                "No pending choice is available.",
            ));
        };

        if index >= options.len() {
            return Err(ScriptLangError::new(
                "ENGINE_CHOICE_INDEX",
                format!("Choice index \"{}\" is out of range.", index),
            ));
        }

        let frame_index = self.find_frame_index(frame_id).ok_or_else(|| {
            ScriptLangError::new(
                "ENGINE_CHOICE_FRAME_MISSING",
                "Pending choice frame is missing.",
            )
        })?;

        let node_index = self.frames[frame_index].node_index;
        let group_id = self.frames[frame_index].group_id.clone();
        let (script_name, group) = self.lookup_group(&group_id)?;

        let Some(ScriptNode::Choice { options: node_options, .. }) = group.nodes.get(node_index) else {
            return Err(ScriptLangError::new(
                "ENGINE_CHOICE_NODE_MISSING",
                "Pending choice node is no longer valid.",
            ));
        };

        let item = &options[index];
        let option = node_options
            .iter()
            .find(|candidate| candidate.id == item.id)
            .ok_or_else(|| {
                ScriptLangError::new("ENGINE_CHOICE_NOT_FOUND", "Choice option no longer exists.")
            })?
            .clone();

        if option.once {
            self.mark_once_state(&script_name, &format!("option:{}", option.id));
        }

        self.frames[frame_index].node_index += 1;
        self.push_group_frame(&option.group_id, CompletionKind::ResumeAfterChild)?;
        self.pending_boundary = None;
        self.waiting_choice = false;
        Ok(())
    }

    pub fn submit_input(&mut self, text: &str) -> Result<(), ScriptLangError> {
        let Some(PendingBoundary::Input {
            frame_id,
            target_var,
            default_text,
            ..
        }) = self.pending_boundary.clone()
        else {
            return Err(ScriptLangError::new(
                "ENGINE_NO_PENDING_INPUT",
                "No pending input is available.",
            ));
        };

        let frame_index = self.find_frame_index(frame_id).ok_or_else(|| {
            ScriptLangError::new(
                "ENGINE_INPUT_FRAME_MISSING",
                "Pending input frame is missing.",
            )
        })?;

        let normalized = if text.trim().is_empty() {
            default_text
        } else {
            text.to_string()
        };

        self.write_path(&target_var, SlValue::String(normalized))?;
        self.frames[frame_index].node_index += 1;
        self.pending_boundary = None;
        self.waiting_choice = false;
        Ok(())
    }

    pub fn snapshot(&self) -> Result<SnapshotV3, ScriptLangError> {
        let Some(boundary) = &self.pending_boundary else {
            return Err(ScriptLangError::new(
                "SNAPSHOT_NOT_ALLOWED",
                "snapshot() is only allowed while waiting for a choice or input.",
            ));
        };

        let runtime_frames = self
            .frames
            .iter()
            .map(|frame| SnapshotFrameV3 {
                frame_id: frame.frame_id,
                group_id: frame.group_id.clone(),
                node_index: frame.node_index,
                scope: frame.scope.clone(),
                var_types: frame.var_types.clone(),
                completion: match frame.completion {
                    CompletionKind::None => SnapshotCompletion::None,
                    CompletionKind::WhileBody => SnapshotCompletion::WhileBody,
                    CompletionKind::ResumeAfterChild => SnapshotCompletion::ResumeAfterChild,
                },
                script_root: frame.script_root,
                return_continuation: frame.return_continuation.clone(),
            })
            .collect::<Vec<_>>();

        let pending_boundary = match boundary {
            PendingBoundary::Choice {
                node_id,
                options,
                prompt_text,
                ..
            } => PendingBoundaryV3::Choice {
                node_id: node_id.clone(),
                items: options.clone(),
                prompt_text: prompt_text.clone(),
            },
            PendingBoundary::Input {
                node_id,
                target_var,
                prompt_text,
                default_text,
                ..
            } => PendingBoundaryV3::Input {
                node_id: node_id.clone(),
                target_var: target_var.clone(),
                prompt_text: prompt_text.clone(),
                default_text: default_text.clone(),
            },
        };

        let once_state_by_script = self
            .once_state_by_script
            .iter()
            .map(|(script_name, set)| {
                let mut values = set.iter().cloned().collect::<Vec<_>>();
                values.sort();
                (script_name.clone(), values)
            })
            .collect();

        Ok(SnapshotV3 {
            schema_version: SNAPSHOT_SCHEMA_V3.to_string(),
            compiler_version: self.compiler_version.clone(),
            runtime_frames,
            rng_state: self.rng_state,
            pending_boundary,
            once_state_by_script,
        })
    }

    pub fn resume(&mut self, snapshot: SnapshotV3) -> Result<(), ScriptLangError> {
        if snapshot.schema_version != SNAPSHOT_SCHEMA_V3 {
            return Err(ScriptLangError::new(
                "SNAPSHOT_SCHEMA",
                format!(
                    "Unsupported snapshot schema \"{}\".",
                    snapshot.schema_version
                ),
            ));
        }

        if snapshot.compiler_version != self.compiler_version {
            return Err(ScriptLangError::new(
                "SNAPSHOT_COMPILER_VERSION",
                format!(
                    "Snapshot compiler version \"{}\" does not match engine \"{}\".",
                    snapshot.compiler_version, self.compiler_version
                ),
            ));
        }

        self.reset();
        self.rng_state = snapshot.rng_state;

        self.once_state_by_script = snapshot
            .once_state_by_script
            .into_iter()
            .map(|(script, entries)| (script, entries.into_iter().collect()))
            .collect();

        self.frames = snapshot
            .runtime_frames
            .into_iter()
            .map(|frame| RuntimeFrame {
                frame_id: frame.frame_id,
                group_id: frame.group_id,
                node_index: frame.node_index,
                scope: frame.scope,
                completion: match frame.completion {
                    SnapshotCompletion::None => CompletionKind::None,
                    SnapshotCompletion::WhileBody => CompletionKind::WhileBody,
                    SnapshotCompletion::ResumeAfterChild => CompletionKind::ResumeAfterChild,
                },
                script_root: frame.script_root,
                return_continuation: frame.return_continuation,
                var_types: frame.var_types,
            })
            .collect();

        self.frame_counter = self
            .frames
            .iter()
            .map(|frame| frame.frame_id)
            .max()
            .unwrap_or(0)
            + 1;

        let top = self
            .frames
            .last()
            .ok_or_else(|| {
                ScriptLangError::new("SNAPSHOT_EMPTY", "Snapshot contains no runtime frames.")
            })?
            .clone();

        let (script_name, group) = self.lookup_group(&top.group_id)?;
        let node = group
            .nodes
            .get(top.node_index)
            .ok_or_else(|| {
                ScriptLangError::new("SNAPSHOT_PENDING_BOUNDARY", "Pending node index invalid.")
            })?
            .clone();

        self.pending_boundary = Some(match snapshot.pending_boundary {
            PendingBoundaryV3::Choice {
                node_id,
                items,
                prompt_text,
            } => {
                let ScriptNode::Choice { id, .. } = node else {
                    return Err(ScriptLangError::new(
                        "SNAPSHOT_PENDING_BOUNDARY",
                        "Snapshot pending boundary expects choice node.",
                    ));
                };
                if id != node_id {
                    return Err(ScriptLangError::new(
                        "SNAPSHOT_PENDING_BOUNDARY",
                        "Snapshot pending choice node mismatch.",
                    ));
                }
                self.waiting_choice = true;
                PendingBoundary::Choice {
                    frame_id: top.frame_id,
                    node_id,
                    options: items,
                    prompt_text,
                }
            }
            PendingBoundaryV3::Input {
                node_id,
                target_var,
                prompt_text,
                default_text,
            } => {
                let ScriptNode::Input { id, .. } = node else {
                    return Err(ScriptLangError::new(
                        "SNAPSHOT_PENDING_BOUNDARY",
                        "Snapshot pending boundary expects input node.",
                    ));
                };
                if id != node_id {
                    return Err(ScriptLangError::new(
                        "SNAPSHOT_PENDING_BOUNDARY",
                        "Snapshot pending input node mismatch.",
                    ));
                }
                self.waiting_choice = false;
                PendingBoundary::Input {
                    frame_id: top.frame_id,
                    node_id,
                    target_var,
                    prompt_text,
                    default_text,
                }
            }
        });

        // Force visibility map access path to validate script existence.
        let _ = self.visible_json_by_script.get(&script_name);

        Ok(())
    }

    fn reset(&mut self) {
        self.frames.clear();
        self.pending_boundary = None;
        self.waiting_choice = false;
        self.ended = false;
        self.frame_counter = 1;
        self.rng_state = self.initial_random_seed;
    }

    fn boundary_output(&self, boundary: &PendingBoundary) -> EngineOutput {
        match boundary {
            PendingBoundary::Choice {
                options,
                prompt_text,
                ..
            } => EngineOutput::Choices {
                items: options.clone(),
                prompt_text: prompt_text.clone(),
            },
            PendingBoundary::Input {
                prompt_text,
                default_text,
                ..
            } => EngineOutput::Input {
                prompt_text: prompt_text.clone(),
                default_text: default_text.clone(),
            },
        }
    }

    fn top_frame_id(&self) -> Result<u64, ScriptLangError> {
        self.frames
            .last()
            .map(|frame| frame.frame_id)
            .ok_or_else(|| ScriptLangError::new("ENGINE_NO_FRAME", "No runtime frame available."))
    }

    fn bump_top_node_index(&mut self, amount: usize) -> Result<(), ScriptLangError> {
        let frame = self.frames.last_mut().ok_or_else(|| {
            ScriptLangError::new("ENGINE_NO_FRAME", "No runtime frame available.")
        })?;
        frame.node_index += amount;
        Ok(())
    }

    fn find_frame_index(&self, frame_id: u64) -> Option<usize> {
        self.frames
            .iter()
            .position(|frame| frame.frame_id == frame_id)
    }

    fn lookup_group(
        &self,
        group_id: &str,
    ) -> Result<(String, sl_core::ImplicitGroup), ScriptLangError> {
        let lookup = self.group_lookup.get(group_id).ok_or_else(|| {
            ScriptLangError::new(
                "ENGINE_GROUP_NOT_FOUND",
                format!("Group \"{}\" not found.", group_id),
            )
        })?;

        let script = self.scripts.get(&lookup.script_name).ok_or_else(|| {
            ScriptLangError::new(
                "ENGINE_SCRIPT_NOT_FOUND",
                format!("Script \"{}\" not found.", lookup.script_name),
            )
        })?;

        let group = script.groups.get(&lookup.group_id).ok_or_else(|| {
            ScriptLangError::new(
                "ENGINE_GROUP_NOT_FOUND",
                format!("Group \"{}\" missing.", group_id),
            )
        })?;

        Ok((lookup.script_name.clone(), group.clone()))
    }

    fn push_root_frame(
        &mut self,
        group_id: &str,
        scope: BTreeMap<String, SlValue>,
        return_continuation: Option<ContinuationFrame>,
        var_types: BTreeMap<String, ScriptType>,
    ) {
        self.frames.push(RuntimeFrame {
            frame_id: self.frame_counter,
            group_id: group_id.to_string(),
            node_index: 0,
            scope,
            completion: CompletionKind::None,
            script_root: true,
            return_continuation,
            var_types,
        });
        self.frame_counter += 1;
    }

    fn push_group_frame(
        &mut self,
        group_id: &str,
        completion: CompletionKind,
    ) -> Result<(), ScriptLangError> {
        if !self.group_lookup.contains_key(group_id) {
            return Err(ScriptLangError::new(
                "ENGINE_GROUP_NOT_FOUND",
                format!("Group \"{}\" not found.", group_id),
            ));
        }

        self.frames.push(RuntimeFrame {
            frame_id: self.frame_counter,
            group_id: group_id.to_string(),
            node_index: 0,
            scope: BTreeMap::new(),
            completion,
            script_root: false,
            return_continuation: None,
            var_types: BTreeMap::new(),
        });
        self.frame_counter += 1;
        Ok(())
    }

    fn finish_frame(&mut self, frame_id: u64) -> Result<(), ScriptLangError> {
        let Some(index) = self.find_frame_index(frame_id) else {
            return Ok(());
        };
        let frame = self.frames.remove(index);
        if !frame.script_root {
            return Ok(());
        }

        let Some(continuation) = frame.return_continuation else {
            self.end_execution();
            return Ok(());
        };

        let Some(resume_index) = self.find_frame_index(continuation.resume_frame_id) else {
            self.end_execution();
            return Ok(());
        };

        for (callee_var, caller_path) in continuation.ref_bindings {
            let value = frame.scope.get(&callee_var).cloned().ok_or_else(|| {
                ScriptLangError::new(
                    "ENGINE_REF_VALUE_MISSING",
                    format!("Missing ref value \"{}\" in callee scope.", callee_var),
                )
            })?;
            self.write_path(&caller_path, value)?;
        }

        self.frames[resume_index].node_index = continuation.next_node_index;
        Ok(())
    }

    fn execute_var_declaration(
        &mut self,
        decl: &sl_core::VarDeclaration,
    ) -> Result<(), ScriptLangError> {
        let duplicate = self
            .frames
            .last()
            .ok_or_else(|| {
                ScriptLangError::new(
                    "ENGINE_VAR_FRAME",
                    "No frame available for var declaration.",
                )
            })?
            .scope
            .contains_key(&decl.name);
        if duplicate {
            return Err(ScriptLangError::new(
                "ENGINE_VAR_DUPLICATE",
                format!(
                    "Variable \"{}\" is already declared in current scope.",
                    decl.name
                ),
            ));
        }

        let mut value = default_value_from_type(&decl.r#type);
        if let Some(expr) = &decl.initial_value_expr {
            value = self.eval_expression(expr)?;
        }

        if !is_type_compatible(&value, &decl.r#type) {
            return Err(ScriptLangError::new(
                "ENGINE_TYPE_MISMATCH",
                format!("Variable \"{}\" does not match declared type.", decl.name),
            ));
        }

        let frame = self.frames.last_mut().ok_or_else(|| {
            ScriptLangError::new(
                "ENGINE_VAR_FRAME",
                "No frame available for var declaration.",
            )
        })?;
        frame.scope.insert(decl.name.clone(), value);
        frame
            .var_types
            .insert(decl.name.clone(), decl.r#type.clone());
        Ok(())
    }

    fn execute_call(
        &mut self,
        target_script: &str,
        args: &[sl_core::CallArgument],
    ) -> Result<(), ScriptLangError> {
        let caller_index = self.frames.len().checked_sub(1).ok_or_else(|| {
            ScriptLangError::new("ENGINE_CALL_NO_FRAME", "No frame available for <call>.")
        })?;

        let caller_group_id = self.frames[caller_index].group_id.clone();
        let (_, caller_group) = self.lookup_group(&caller_group_id)?;

        let Some(target) = self.scripts.get(target_script).cloned() else {
            return Err(ScriptLangError::new(
                "ENGINE_CALL_TARGET",
                format!("Call target script \"{}\" not found.", target_script),
            ));
        };

        let mut arg_values = BTreeMap::new();
        let mut ref_bindings = BTreeMap::new();

        for (index, arg) in args.iter().enumerate() {
            let Some(param) = target.params.get(index) else {
                return Err(ScriptLangError::new(
                    "ENGINE_CALL_ARG_UNKNOWN",
                    format!("Call argument at position {} has no matching parameter.", index + 1),
                ));
            };

            if param.is_ref && !arg.is_ref {
                return Err(ScriptLangError::new(
                    "ENGINE_CALL_REF_MISMATCH",
                    format!("Call argument {} must use ref mode.", index + 1),
                ));
            }
            if !param.is_ref && arg.is_ref {
                return Err(ScriptLangError::new(
                    "ENGINE_CALL_REF_MISMATCH",
                    format!("Call argument {} cannot use ref mode.", index + 1),
                ));
            }

            if arg.is_ref {
                let value = self.read_path(&arg.value_expr)?;
                arg_values.insert(param.name.clone(), value);
                ref_bindings.insert(param.name.clone(), arg.value_expr.clone());
            } else {
                let value = self.eval_expression(&arg.value_expr)?;
                arg_values.insert(param.name.clone(), value);
            }
        }

        let caller = self.frames[caller_index].clone();
        let is_tail_at_root = caller.script_root
            && caller.node_index == caller_group.nodes.len().saturating_sub(1)
            && caller.return_continuation.is_some();

        if is_tail_at_root && !ref_bindings.is_empty() {
            return Err(ScriptLangError::new(
                "ENGINE_TAIL_REF_UNSUPPORTED",
                "Tail call with ref args is not supported.",
            ));
        }

        if is_tail_at_root {
            let inherited = caller.return_continuation.clone();
            self.frames.pop();
            let (scope, var_types) = self.create_script_root_scope(target_script, arg_values)?;
            self.push_root_frame(&target.root_group_id, scope, inherited, var_types);
            return Ok(());
        }

        let continuation = ContinuationFrame {
            resume_frame_id: caller.frame_id,
            next_node_index: caller.node_index + 1,
            ref_bindings,
        };

        let (scope, var_types) = self.create_script_root_scope(target_script, arg_values)?;
        self.push_root_frame(&target.root_group_id, scope, Some(continuation), var_types);
        Ok(())
    }

    fn execute_return(
        &mut self,
        target_script: Option<String>,
        args: &[sl_core::CallArgument],
    ) -> Result<(), ScriptLangError> {
        let root_index = self.find_current_root_frame_index()?;
        let root_frame = self.frames[root_index].clone();
        let inherited = root_frame.return_continuation.clone();

        let mut transfer_arg_values = BTreeMap::new();

        if let Some(target_name) = target_script.as_ref() {
            let Some(target) = self.scripts.get(target_name).cloned() else {
                return Err(ScriptLangError::new(
                    "ENGINE_RETURN_TARGET",
                    format!("Return target script \"{}\" not found.", target_name),
                ));
            };

            for (index, arg) in args.iter().enumerate() {
                let Some(param) = target.params.get(index) else {
                    return Err(ScriptLangError::new(
                        "ENGINE_RETURN_ARG_UNKNOWN",
                        format!("Return argument at position {} has no target parameter.", index + 1),
                    ));
                };
                transfer_arg_values
                    .insert(param.name.clone(), self.eval_expression(&arg.value_expr)?);
            }
        }

        self.frames.truncate(root_index);

        if let Some(target_name) = target_script {
            let Some(target) = self.scripts.get(&target_name).cloned() else {
                return Err(ScriptLangError::new(
                    "ENGINE_RETURN_TARGET",
                    format!("Return target script \"{}\" not found.", target_name),
                ));
            };

            let mut forwarded = inherited.clone();
            if let Some(continuation) = inherited {
                if self
                    .find_frame_index(continuation.resume_frame_id)
                    .is_some()
                {
                    for (callee_var, caller_path) in continuation.ref_bindings {
                        if let Some(value) = root_frame.scope.get(&callee_var).cloned() {
                            self.write_path(&caller_path, value)?;
                        }
                    }
                }

                if let Some(mut continuation) = forwarded.take() {
                    continuation.ref_bindings = BTreeMap::new();
                    forwarded = Some(continuation);
                }
            }

            let (scope, var_types) =
                self.create_script_root_scope(&target_name, transfer_arg_values)?;
            self.push_root_frame(&target.root_group_id, scope, forwarded, var_types);
            return Ok(());
        }

        let Some(continuation) = inherited else {
            self.end_execution();
            return Ok(());
        };

        let Some(resume_index) = self.find_frame_index(continuation.resume_frame_id) else {
            self.end_execution();
            return Ok(());
        };

        for (callee_var, caller_path) in continuation.ref_bindings {
            if let Some(value) = root_frame.scope.get(&callee_var).cloned() {
                self.write_path(&caller_path, value)?;
            }
        }

        self.frames[resume_index].node_index = continuation.next_node_index;
        Ok(())
    }

    fn find_current_root_frame_index(&self) -> Result<usize, ScriptLangError> {
        for (index, frame) in self.frames.iter().enumerate().rev() {
            if frame.script_root {
                return Ok(index);
            }
        }
        Err(ScriptLangError::new(
            "ENGINE_ROOT_FRAME",
            "No script root frame found.",
        ))
    }

    fn execute_break(&mut self) -> Result<(), ScriptLangError> {
        let while_body_index = self.find_nearest_while_body_frame_index().ok_or_else(|| {
            ScriptLangError::new(
                "ENGINE_WHILE_CONTROL_TARGET_MISSING",
                "No target <while> frame found for <break>.",
            )
        })?;

        if while_body_index == 0 {
            return Err(ScriptLangError::new(
                "ENGINE_WHILE_CONTROL_TARGET_MISSING",
                "No owning while frame found.",
            ));
        }

        let while_owner_index = while_body_index - 1;
        let while_owner = self.frames[while_owner_index].clone();
        let (_, group) = self.lookup_group(&while_owner.group_id)?;
        let Some(ScriptNode::While { .. }) = group.nodes.get(while_owner.node_index) else {
            return Err(ScriptLangError::new(
                "ENGINE_WHILE_CONTROL_TARGET_MISSING",
                "Owning while node is missing.",
            ));
        };

        self.frames.truncate(while_body_index);
        self.frames[while_owner_index].node_index += 1;
        Ok(())
    }

    fn execute_continue_while(&mut self) -> Result<(), ScriptLangError> {
        let while_body_index = self.find_nearest_while_body_frame_index().ok_or_else(|| {
            ScriptLangError::new(
                "ENGINE_WHILE_CONTROL_TARGET_MISSING",
                "No target <while> frame found for <continue>.",
            )
        })?;
        if while_body_index == 0 {
            return Err(ScriptLangError::new(
                "ENGINE_WHILE_CONTROL_TARGET_MISSING",
                "No owning while frame found.",
            ));
        }

        self.frames.truncate(while_body_index);
        Ok(())
    }

    fn execute_continue_choice(&mut self) -> Result<(), ScriptLangError> {
        let Some((choice_frame_index, choice_node_index)) = self.find_choice_continue_context()? else {
            return Err(ScriptLangError::new(
                "ENGINE_CHOICE_CONTINUE_TARGET_MISSING",
                "No target <choice> node found for option <continue>.",
            ));
        };

        self.frames.truncate(choice_frame_index + 1);
        self.frames[choice_frame_index].node_index = choice_node_index;
        Ok(())
    }

    fn find_choice_continue_context(&self) -> Result<Option<(usize, usize)>, ScriptLangError> {
        for frame_index in (0..self.frames.len()).rev() {
            let frame = &self.frames[frame_index];
            if frame.node_index == 0 {
                continue;
            }

            let (_, group) = self.lookup_group(&frame.group_id)?;
            let choice_node_index = frame.node_index - 1;
            let Some(ScriptNode::Choice { options, .. }) = group.nodes.get(choice_node_index) else {
                continue;
            };

            let option_group_ids = options
                .iter()
                .map(|option| option.group_id.clone())
                .collect::<BTreeSet<_>>();

            for deep_index in frame_index + 1..self.frames.len() {
                if option_group_ids.contains(&self.frames[deep_index].group_id) {
                    return Ok(Some((frame_index, choice_node_index)));
                }
            }
        }

        Ok(None)
    }

    fn find_nearest_while_body_frame_index(&self) -> Option<usize> {
        for (index, frame) in self.frames.iter().enumerate().rev() {
            if frame.completion == CompletionKind::WhileBody {
                return Some(index);
            }
        }
        None
    }

    fn end_execution(&mut self) {
        self.ended = true;
        self.frames.clear();
    }

    fn create_script_root_scope(
        &self,
        script_name: &str,
        arg_values: BTreeMap<String, SlValue>,
    ) -> Result<ScopeInit, ScriptLangError> {
        let script = self.scripts.get(script_name).ok_or_else(|| {
            ScriptLangError::new(
                "ENGINE_SCRIPT_NOT_FOUND",
                format!("Script \"{}\" not found.", script_name),
            )
        })?;

        let mut scope = BTreeMap::new();
        let mut var_types = BTreeMap::new();

        for param in &script.params {
            let value = default_value_from_type(&param.r#type);
            scope.insert(param.name.clone(), value);
            var_types.insert(param.name.clone(), param.r#type.clone());
        }

        for (name, value) in arg_values {
            if !scope.contains_key(&name) {
                return Err(ScriptLangError::new(
                    "ENGINE_CALL_ARG_UNKNOWN",
                    format!(
                        "Call argument \"{}\" is not declared in target script.",
                        name
                    ),
                ));
            }
            let Some(expected_type) = var_types.get(&name) else {
                continue;
            };
            if !is_type_compatible(&value, expected_type) {
                return Err(ScriptLangError::new(
                    "ENGINE_TYPE_MISMATCH",
                    format!("Call argument \"{}\" does not match declared type.", name),
                ));
            }
            scope.insert(name, value);
        }

        Ok((scope, var_types))
    }

    fn render_text(&mut self, template: &str) -> Result<String, ScriptLangError> {
        let regex = Regex::new(r"\$\{([^{}]+)\}").expect("template regex must compile");
        let mut output = String::new();
        let mut last_index = 0usize;
        for captures in regex.captures_iter(template) {
            let Some(full) = captures.get(0) else {
                continue;
            };
            let Some(expr) = captures.get(1) else {
                continue;
            };
            output.push_str(&template[last_index..full.start()]);
            let value = self.eval_expression(expr.as_str())?;
            output.push_str(&slvalue_to_text(&value));
            last_index = full.end();
        }
        output.push_str(&template[last_index..]);
        Ok(output)
    }

    fn eval_boolean(&mut self, expr: &str) -> Result<bool, ScriptLangError> {
        let value = self.eval_expression(expr)?;
        match value {
            SlValue::Bool(value) => Ok(value),
            _ => Err(ScriptLangError::new(
                "ENGINE_BOOLEAN_EXPECTED",
                format!("Expression \"{}\" must evaluate to boolean.", expr),
            )),
        }
    }

    fn run_code(&mut self, code: &str) -> Result<(), ScriptLangError> {
        self.execute_rhai(code, false).map(|_| ())
    }

    fn eval_expression(&mut self, expr: &str) -> Result<SlValue, ScriptLangError> {
        self.execute_rhai(expr, true)
    }

    fn execute_rhai(
        &mut self,
        script: &str,
        is_expression: bool,
    ) -> Result<SlValue, ScriptLangError> {
        let script_name = self.resolve_current_script_name().unwrap_or_default();

        if !self.host_functions.names().is_empty() {
            return Err(ScriptLangError::new(
                "ENGINE_HOST_FUNCTION_UNSUPPORTED",
                "Host function invocation is not yet supported in this runtime build.",
            ));
        }

        let (mutable_bindings, mutable_order) = self.collect_mutable_bindings();
        let visible_globals = self
            .visible_json_by_script
            .get(&script_name)
            .cloned()
            .unwrap_or_default();

        let mut scope = Scope::new();
        for name in &mutable_order {
            if let Some(binding) = mutable_bindings.get(name) {
                scope.push_dynamic(name.to_string(), slvalue_to_dynamic(&binding.value)?);
            }
        }

        let mut global_snapshot = BTreeMap::new();
        for name in visible_globals {
            if let Some(value) = self.global_json.get(&name) {
                global_snapshot.insert(name.clone(), value.clone());
                scope.push_dynamic(name, slvalue_to_dynamic(value)?);
            }
        }

        let mut engine = Engine::new();
        engine.set_strict_variables(true);

        let rng_state = Rc::new(RefCell::new(self.rng_state));
        let rng_state_clone = Rc::clone(&rng_state);
        engine.register_fn(
            "random",
            move |bound: INT| -> Result<INT, Box<EvalAltResult>> {
                if bound <= 0 {
                    return Err(Box::new(EvalAltResult::ErrorRuntime(
                        Dynamic::from("random(n) expects positive integer n."),
                        Position::NONE,
                    )));
                }

                let mut state = rng_state_clone.borrow_mut();
                let value = next_random_bounded(&mut state, bound as u32);
                Ok(value as INT)
            },
        );

        let prelude = self.build_defs_prelude(&script_name)?;
        let source = if is_expression {
            if prelude.is_empty() {
                format!("({})", script)
            } else {
                format!("{}\n({})", prelude, script)
            }
        } else if prelude.is_empty() {
            script.to_string()
        } else {
            format!("{}\n{}", prelude, script)
        };

        let run_result = if is_expression {
            engine
                .eval_with_scope::<Dynamic>(&mut scope, &source)
                .map_err(|error| {
                    ScriptLangError::new(
                        "ENGINE_EVAL_ERROR",
                        format!("Expression eval failed: {}", error),
                    )
                })
                .and_then(dynamic_to_slvalue)
        } else {
            engine
                .run_with_scope(&mut scope, &source)
                .map_err(|error| {
                    ScriptLangError::new(
                        "ENGINE_EVAL_ERROR",
                        format!("Code eval failed: {}", error),
                    )
                })
                .map(|_| SlValue::Bool(true))
        };

        self.rng_state = *rng_state.borrow();

        for (name, before) in global_snapshot {
            if let Some(after_dynamic) = scope.get_value::<Dynamic>(&name) {
                let after = dynamic_to_slvalue(after_dynamic)?;
                if after != before {
                    return Err(ScriptLangError::new(
                        "ENGINE_GLOBAL_READONLY",
                        format!(
                            "Global JSON \"{}\" is readonly and cannot be mutated.",
                            name
                        ),
                    ));
                }
            }
        }

        for name in mutable_order {
            if let Some(after_dynamic) = scope.get_value::<Dynamic>(&name) {
                let after = dynamic_to_slvalue(after_dynamic)?;
                self.write_variable(&name, after)?;
            }
        }

        run_result
    }

    fn build_defs_prelude(&self, script_name: &str) -> Result<String, ScriptLangError> {
        let Some(script) = self.scripts.get(script_name) else {
            return Ok(String::new());
        };
        let visible_json = self
            .visible_json_by_script
            .get(script_name)
            .cloned()
            .unwrap_or_default();

        let mut out = String::new();
        for (name, decl) in &script.visible_functions {
            out.push_str("fn ");
            out.push_str(name);
            out.push('(');
            out.push_str(
                &decl
                    .params
                    .iter()
                    .map(|param| param.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            out.push_str(") {\n");

            for json_symbol in &visible_json {
                if let Some(value) = self.global_json.get(json_symbol) {
                    out.push_str(&format!(
                        "let {} = {};\n",
                        json_symbol,
                        slvalue_to_rhai_literal(value)
                    ));
                }
            }

            let default_value = default_value_from_type(&decl.return_binding.r#type);
            out.push_str(&format!(
                "let {} = {};\n",
                decl.return_binding.name,
                slvalue_to_rhai_literal(&default_value)
            ));
            out.push_str(&decl.code);
            out.push('\n');
            out.push_str(&decl.return_binding.name);
            out.push_str("\n}\n");
        }

        Ok(out)
    }

    fn collect_mutable_bindings(&self) -> (BTreeMap<String, BindingOwner>, Vec<String>) {
        let mut map = BTreeMap::new();
        let mut order = Vec::new();
        for frame in self.frames.iter().rev() {
            for (name, value) in &frame.scope {
                if map.contains_key(name) {
                    continue;
                }
                map.insert(
                    name.clone(),
                    BindingOwner {
                        value: value.clone(),
                    },
                );
                order.push(name.clone());
            }
        }
        (map, order)
    }

    fn resolve_current_script_name(&self) -> Option<String> {
        let top = self.frames.last()?;
        self.group_lookup
            .get(&top.group_id)
            .map(|entry| entry.script_name.clone())
    }

    fn is_visible_json_global(&self, script_name: Option<&str>, name: &str) -> bool {
        let Some(script_name) = script_name else {
            return false;
        };
        let Some(visible) = self.visible_json_by_script.get(script_name) else {
            return false;
        };
        visible.contains(name) && self.global_json.contains_key(name)
    }

    fn read_variable(&self, name: &str) -> Result<SlValue, ScriptLangError> {
        for frame in self.frames.iter().rev() {
            if let Some(value) = frame.scope.get(name) {
                return Ok(value.clone());
            }
        }

        let script_name = self.resolve_current_script_name();
        if self.is_visible_json_global(script_name.as_deref(), name) {
            if let Some(value) = self.global_json.get(name) {
                return Ok(value.clone());
            }
        }

        Err(ScriptLangError::new(
            "ENGINE_VAR_READ",
            format!("Variable \"{}\" is not defined.", name),
        ))
    }

    fn write_variable(&mut self, name: &str, value: SlValue) -> Result<(), ScriptLangError> {
        for frame in self.frames.iter_mut().rev() {
            if frame.scope.contains_key(name) {
                if let Some(declared_type) = frame.var_types.get(name) {
                    if !is_type_compatible(&value, declared_type) {
                        return Err(ScriptLangError::new(
                            "ENGINE_TYPE_MISMATCH",
                            format!("Variable \"{}\" does not match declared type.", name),
                        ));
                    }
                }
                frame.scope.insert(name.to_string(), value);
                return Ok(());
            }
        }

        let script_name = self.resolve_current_script_name();
        if self.is_visible_json_global(script_name.as_deref(), name) {
            return Err(ScriptLangError::new(
                "ENGINE_GLOBAL_READONLY",
                format!(
                    "Global JSON \"{}\" is readonly and cannot be mutated.",
                    name
                ),
            ));
        }

        Err(ScriptLangError::new(
            "ENGINE_VAR_WRITE",
            format!("Variable \"{}\" is not defined.", name),
        ))
    }

    fn read_path(&self, path: &str) -> Result<SlValue, ScriptLangError> {
        let parts = parse_ref_path(path);
        if parts.is_empty() {
            return Err(ScriptLangError::new(
                "ENGINE_REF_PATH",
                format!("Invalid ref path \"{}\".", path),
            ));
        }

        let mut current = self.read_variable(&parts[0])?;
        for part in parts.iter().skip(1) {
            let SlValue::Map(entries) = current else {
                return Err(ScriptLangError::new(
                    "ENGINE_REF_PATH_READ",
                    format!("Cannot resolve path \"{}\".", path),
                ));
            };
            current = entries.get(part).cloned().ok_or_else(|| {
                ScriptLangError::new(
                    "ENGINE_REF_PATH_READ",
                    format!("Cannot resolve path \"{}\".", path),
                )
            })?;
        }

        Ok(current)
    }

    fn write_path(&mut self, path: &str, value: SlValue) -> Result<(), ScriptLangError> {
        let parts = parse_ref_path(path);
        if parts.is_empty() {
            return Err(ScriptLangError::new(
                "ENGINE_REF_PATH",
                format!("Invalid ref path \"{}\".", path),
            ));
        }

        if parts.len() == 1 {
            return self.write_variable(&parts[0], value);
        }

        let head = &parts[0];
        let mut root_value = self.read_variable(head)?;
        assign_nested_path(&mut root_value, &parts[1..], value).map_err(|message| {
            ScriptLangError::new(
                "ENGINE_REF_PATH_WRITE",
                format!("Cannot resolve write path \"{}\": {}", path, message),
            )
        })?;
        self.write_variable(head, root_value)
    }

    fn is_choice_option_visible(
        &mut self,
        script_name: &str,
        option: &sl_core::ChoiceOption,
    ) -> Result<bool, ScriptLangError> {
        if let Some(when_expr) = &option.when_expr {
            if !self.eval_boolean(when_expr)? {
                return Ok(false);
            }
        }

        if !option.once {
            return Ok(true);
        }

        Ok(!self.has_once_state(script_name, &format!("option:{}", option.id)))
    }

    fn has_once_state(&self, script_name: &str, key: &str) -> bool {
        self.once_state_by_script
            .get(script_name)
            .map(|set| set.contains(key))
            .unwrap_or(false)
    }

    fn mark_once_state(&mut self, script_name: &str, key: &str) {
        self.once_state_by_script
            .entry(script_name.to_string())
            .or_default()
            .insert(key.to_string());
    }
}

#[derive(Debug, Clone)]
struct BindingOwner {
    value: SlValue,
}

fn parse_ref_path(path: &str) -> Vec<String> {
    path.split('.')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn assign_nested_path(target: &mut SlValue, path: &[String], value: SlValue) -> Result<(), String> {
    if path.is_empty() {
        *target = value;
        return Ok(());
    }

    let SlValue::Map(entries) = target else {
        return Err("target is not an object/map".to_string());
    };

    let head = &path[0];
    if path.len() == 1 {
        entries.insert(head.clone(), value);
        return Ok(());
    }

    let next = entries
        .get_mut(head)
        .ok_or_else(|| format!("missing key \"{}\"", head))?;
    assign_nested_path(next, &path[1..], value)
}

fn slvalue_to_text(value: &SlValue) -> String {
    match value {
        SlValue::Bool(value) => value.to_string(),
        SlValue::Number(value) => {
            if value.fract().abs() < f64::EPSILON {
                (*value as i64).to_string()
            } else {
                value.to_string()
            }
        }
        SlValue::String(value) => value.clone(),
        SlValue::Array(_) | SlValue::Map(_) => format!("{:?}", value),
    }
}

fn slvalue_to_dynamic(value: &SlValue) -> Result<Dynamic, ScriptLangError> {
    match value {
        SlValue::Bool(value) => Ok(Dynamic::from_bool(*value)),
        SlValue::Number(value) => Ok(Dynamic::from_float(*value as FLOAT)),
        SlValue::String(value) => Ok(Dynamic::from(value.clone())),
        SlValue::Array(values) => {
            let mut array = Array::new();
            for value in values {
                array.push(slvalue_to_dynamic(value)?);
            }
            Ok(Dynamic::from_array(array))
        }
        SlValue::Map(values) => {
            let mut map = Map::new();
            for (key, value) in values {
                map.insert(key.clone().into(), slvalue_to_dynamic(value)?);
            }
            Ok(Dynamic::from_map(map))
        }
    }
}

fn dynamic_to_slvalue(value: Dynamic) -> Result<SlValue, ScriptLangError> {
    if value.is::<bool>() {
        return Ok(SlValue::Bool(value.cast::<bool>()));
    }
    if value.is::<INT>() {
        return Ok(SlValue::Number(value.cast::<INT>() as f64));
    }
    if value.is::<FLOAT>() {
        return Ok(SlValue::Number(value.cast::<FLOAT>()));
    }
    if value.is::<ImmutableString>() {
        return Ok(SlValue::String(value.cast::<ImmutableString>().to_string()));
    }
    if value.is::<Array>() {
        let array = value.cast::<Array>();
        let mut out = Vec::with_capacity(array.len());
        for item in array {
            out.push(dynamic_to_slvalue(item)?);
        }
        return Ok(SlValue::Array(out));
    }
    if value.is::<Map>() {
        let map = value.cast::<Map>();
        let mut out = BTreeMap::new();
        for (key, value) in map {
            out.insert(key.to_string(), dynamic_to_slvalue(value)?);
        }
        return Ok(SlValue::Map(out));
    }

    Err(ScriptLangError::new(
        "ENGINE_VALUE_UNSUPPORTED",
        "Unsupported Rhai value type.",
    ))
}

fn slvalue_to_rhai_literal(value: &SlValue) -> String {
    match value {
        SlValue::Bool(value) => value.to_string(),
        SlValue::Number(value) => {
            if value.fract().abs() < f64::EPSILON {
                (*value as i64).to_string()
            } else {
                value.to_string()
            }
        }
        SlValue::String(value) => format!("\"{}\"", value.replace('"', "\\\"")),
        SlValue::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(slvalue_to_rhai_literal)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        SlValue::Map(values) => {
            let entries = values
                .iter()
                .map(|(key, value)| format!("{}: {}", key, slvalue_to_rhai_literal(value)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("#{{{}}}", entries)
        }
    }
}

fn next_random_u32(state: &mut u32) -> u32 {
    let mut next = state.wrapping_add(0x6d2b79f5);
    *state = next;
    next = (next ^ (next >> 15)).wrapping_mul(next | 1);
    next ^= next.wrapping_add((next ^ (next >> 7)).wrapping_mul(next | 61));
    next ^ (next >> 14)
}

fn next_random_bounded(state: &mut u32, bound: u32) -> u32 {
    let threshold = (u64::from(u32::MAX) + 1) / u64::from(bound) * u64::from(bound);
    loop {
        let candidate = next_random_u32(state);
        if u64::from(candidate) < threshold {
            return candidate % bound;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sl_compiler::compile_project_bundle_from_xml_map;

    fn map(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    fn engine_from_sources(files: BTreeMap<String, String>) -> ScriptLangEngine {
        let compiled = compile_project_bundle_from_xml_map(&files).expect("compile should pass");
        ScriptLangEngine::new(ScriptLangEngineOptions {
            scripts: compiled.scripts,
            global_json: compiled.global_json,
            host_functions: None,
            random_seed: Some(1),
            compiler_version: Some(DEFAULT_COMPILER_VERSION.to_string()),
        })
        .expect("engine should build")
    }

    #[test]
    fn next_text_and_end() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>Hello</text></script>"#,
        )]));
        engine.start("main", None).expect("start");

        let first = engine.next().expect("next");
        assert!(matches!(first, EngineOutput::Text { .. }));

        let second = engine.next().expect("next");
        assert!(matches!(second, EngineOutput::End));
    }

    #[test]
    fn snapshot_resume_choice_roundtrip() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
<script name="main">
  <choice text="Pick">
    <option text="A"><text>Alpha</text></option>
    <option text="B"><text>Beta</text></option>
  </choice>
</script>
"#,
        )]));
        engine.start("main", None).expect("start");

        let first = engine.next().expect("next");
        assert!(matches!(first, EngineOutput::Choices { .. }));
        let snapshot = engine.snapshot().expect("snapshot");

        let mut resumed = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
<script name="main">
  <choice text="Pick">
    <option text="A"><text>Alpha</text></option>
    <option text="B"><text>Beta</text></option>
  </choice>
</script>
"#,
        )]));
        resumed.resume(snapshot).expect("resume");
        resumed.choose(0).expect("choose");
        let next = resumed.next().expect("next");
        assert!(matches!(next, EngineOutput::Text { .. }));
    }
}
