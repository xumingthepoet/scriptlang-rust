use super::lifecycle::{
    CompletionKind, PendingBoundary as RuntimePendingBoundary, PendingChoiceOption, RuntimeFrame,
    RuntimeRandomState,
};
use super::*;
use sl_core::PendingBoundary as SnapshotPendingBoundary;

impl ScriptLangEngine {
    pub fn snapshot(&self) -> Result<Snapshot, ScriptLangError> {
        let Some(boundary) = &self.pending_boundary else {
            return Err(ScriptLangError::new(
                "SNAPSHOT_NOT_ALLOWED",
                "snapshot() is only allowed while waiting for a choice or input.",
            ));
        };

        let runtime_frames = self
            .frames
            .iter()
            .map(|frame| SnapshotFrame {
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
            RuntimePendingBoundary::Choice {
                node_id,
                options,
                prompt_text,
                ..
            } => SnapshotPendingBoundary::Choice {
                node_id: node_id.clone(),
                items: options.iter().map(|option| option.item.clone()).collect(),
                prompt_text: prompt_text.clone(),
                dynamic_bindings: options
                    .iter()
                    .filter_map(|option| {
                        option
                            .dynamic_binding
                            .clone()
                            .map(|binding| (option.item.id.clone(), binding))
                    })
                    .collect(),
            },
            RuntimePendingBoundary::Input {
                node_id,
                target_var,
                prompt_text,
                default_text,
                max_length,
                ..
            } => SnapshotPendingBoundary::Input {
                node_id: node_id.clone(),
                target_var: target_var.clone(),
                prompt_text: prompt_text.clone(),
                default_text: default_text.clone(),
                max_length: *max_length,
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

        Ok(Snapshot {
            schema_version: SNAPSHOT_SCHEMA.to_string(),
            compiler_version: self.compiler_version.clone(),
            runtime_frames,
            rng_state: self.current_seeded_rng_state(),
            pending_boundary,
            module_vars: self.module_vars_value.clone(),
            once_state_by_script,
        })
    }

    pub fn resume(&mut self, snapshot: Snapshot) -> Result<(), ScriptLangError> {
        if snapshot.schema_version != SNAPSHOT_SCHEMA {
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
        self.initialize_module_consts()?;
        self.seeded_rng_state = snapshot.rng_state;
        if self.initial_random_sequence.is_none() {
            *self.shared_rng_state.borrow_mut() = RuntimeRandomState::Seeded(snapshot.rng_state);
        }

        for qualified_name in snapshot.module_vars.keys() {
            if !self.module_var_declarations.contains_key(qualified_name) {
                return Err(ScriptLangError::new(
                    "SNAPSHOT_MODULE_GLOBAL_UNKNOWN",
                    format!(
                        "Snapshot contains unknown module global \"{}\".",
                        qualified_name
                    ),
                ));
            }
        }

        let mut restored_module_vars = BTreeMap::new();
        for (qualified_name, decl) in &self.module_var_declarations {
            let value = snapshot
                .module_vars
                .get(qualified_name)
                .cloned()
                .unwrap_or_else(|| default_value_from_type(&decl.r#type));
            if !is_type_compatible(&value, &decl.r#type) {
                return Err(ScriptLangError::new(
                    "SNAPSHOT_MODULE_GLOBAL_TYPE",
                    format!(
                        "Module global \"{}\" from snapshot does not match declared type.",
                        qualified_name
                    ),
                ));
            }
            restored_module_vars.insert(qualified_name.clone(), value);
        }
        self.module_vars_value = restored_module_vars;

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

        let node = {
            let (_, group) = self.lookup_group(&top.group_id)?;
            let node = group
                .nodes
                .get(top.node_index)
                .ok_or_else(|| {
                    ScriptLangError::new("SNAPSHOT_PENDING_BOUNDARY", "Pending node index invalid.")
                })?
                .clone();
            node
        };

        self.pending_boundary = Some(match snapshot.pending_boundary {
            SnapshotPendingBoundary::Choice {
                node_id,
                items,
                prompt_text,
                dynamic_bindings,
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
                RuntimePendingBoundary::Choice {
                    frame_id: top.frame_id,
                    node_id,
                    options: items
                        .into_iter()
                        .map(|item| PendingChoiceOption {
                            dynamic_binding: dynamic_bindings.get(&item.id).cloned(),
                            item,
                        })
                        .collect(),
                    prompt_text,
                }
            }
            SnapshotPendingBoundary::Input {
                node_id,
                target_var,
                prompt_text,
                default_text,
                max_length,
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
                RuntimePendingBoundary::Input {
                    frame_id: top.frame_id,
                    node_id,
                    target_var,
                    prompt_text,
                    default_text,
                    max_length,
                }
            }
        });

        Ok(())
    }
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    use crate::engine::runtime_test_support::*;
    use sl_core::PendingBoundary;

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

    fn pending_kind(pending: &PendingBoundary) -> &'static str {
        match pending {
            PendingBoundary::Choice { .. } => "choice",
            PendingBoundary::Input { .. } => "input",
        }
    }

    fn pending_node_id(pending: &PendingBoundary) -> String {
        match pending {
            PendingBoundary::Choice { node_id, .. } => node_id.clone(),
            PendingBoundary::Input { node_id, .. } => node_id.clone(),
        }
    }

    #[test]
    pub(super) fn snapshot_resume_choice_roundtrip() {
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

        let first = engine.next_output().expect("next");
        assert_eq!(output_kind(&first), "choices");
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
        let next = resumed.next_output().expect("next");
        assert_eq!(output_kind(&next), "text");
    }

    #[test]
    pub(super) fn snapshot_and_resume_cover_while_completion_and_once_state() {
        let files = map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <temp name="n" type="int">1</temp>
      <text once="true">Intro</text>
      <while when="n > 0">
        <choice text="Pick">
          <option text="Stop"><code>n = 0;</code></option>
        </choice>
      </while>
      <end/>
    </script>
    "#,
        )]);

        let mut engine = engine_from_sources(files.clone());
        engine.start("main", None).expect("start");
        assert_eq!(output_kind(&engine.next_output().expect("text")), "text");
        assert_eq!(
            output_kind(&engine.next_output().expect("choice")),
            "choices"
        );
        let snapshot = engine.snapshot().expect("snapshot");
        assert!(!snapshot.once_state_by_script.is_empty());

        let mut resumed = engine_from_sources(files);
        resumed.resume(snapshot).expect("resume");
        resumed.choose(0).expect("choose should pass");
        assert_eq!(output_kind(&resumed.next_output().expect("end")), "end");
    }

    #[test]
    pub(super) fn resume_restores_pending_input_boundary() {
        let files = map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <temp name="heroName" type="string">"Traveler"</temp>
      <input var="heroName" text="Name your hero" max_length="5"/>
      <text>Hello ${heroName}</text>
    </script>
    "#,
        )]);

        let mut engine = engine_from_sources(files.clone());
        engine.start("main", None).expect("start");
        assert_eq!(output_kind(&engine.next_output().expect("input")), "input");
        let snapshot = engine.snapshot().expect("snapshot");

        let mut resumed = engine_from_sources(files);
        resumed.resume(snapshot).expect("resume");
        assert_eq!(output_kind(&resumed.next_output().expect("input")), "input");
        let over_limit = resumed
            .submit_input("TooLong")
            .expect_err("overlong should fail after resume");
        assert_eq!(over_limit.code, "ENGINE_INPUT_TOO_LONG");
        resumed.submit_input("Guild").expect("submit input");
        assert_eq!(output_kind(&resumed.next_output().expect("text")), "text");
    }

    #[test]
    pub(super) fn snapshot_requires_pending_boundary() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>Hello</text></script>"#,
        )]));
        engine.start("main", None).expect("start");
        let error = engine.snapshot().expect_err("snapshot should fail");
        assert_eq!(error.code, "SNAPSHOT_NOT_ALLOWED");
    }

    #[test]
    pub(super) fn resume_validates_schema_and_compiler_version() {
        let sources = map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <choice text="Pick">
        <option text="A"><text>A</text></option>
      </choice>
    </script>
    "#,
        )]);

        let mut base = engine_from_sources(sources.clone());
        base.start("main", None).expect("start");
        let first = base.next_output().expect("next");
        assert_eq!(output_kind(&first), "choices");
        let snapshot = base.snapshot().expect("snapshot");

        let mut schema_mismatch = engine_from_sources(sources.clone());
        let mut bad_schema = snapshot.clone();
        bad_schema.schema_version = "snapshot.bad".to_string();
        let error = schema_mismatch
            .resume(bad_schema)
            .expect_err("schema mismatch should fail");
        assert_eq!(error.code, "SNAPSHOT_SCHEMA");

        let mut compiler_mismatch = engine_from_sources(sources);
        let mut bad_compiler = snapshot;
        bad_compiler.compiler_version = "player.bad".to_string();
        let error = compiler_mismatch
            .resume(bad_compiler)
            .expect_err("compiler mismatch should fail");
        assert_eq!(error.code, "SNAPSHOT_COMPILER_VERSION");
    }

    #[test]
    pub(super) fn resume_rejects_pending_boundary_node_mismatch() {
        let sources = map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <choice text="Pick">
        <option text="A"><text>A</text></option>
      </choice>
    </script>
    "#,
        )]);
        let mut engine = engine_from_sources(sources.clone());
        engine.start("main", None).expect("start");
        let first = engine.next_output().expect("next");
        assert_eq!(output_kind(&first), "choices");
        let mut snapshot = engine.snapshot().expect("snapshot");
        assert_eq!(pending_kind(&snapshot.pending_boundary), "choice");
        snapshot.pending_boundary = PendingBoundary::Choice {
            node_id: "invalid-node-id".to_string(),
            items: Vec::new(),
            prompt_text: None,
            dynamic_bindings: BTreeMap::new(),
        };

        let mut resumed = engine_from_sources(sources);
        let error = resumed
            .resume(snapshot)
            .expect_err("pending choice node mismatch");
        assert_eq!(error.code, "SNAPSHOT_PENDING_BOUNDARY");
    }

    #[test]
    pub(super) fn runtime_errors_cover_snapshot_shape_mismatches() {
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
        engine.start("main", None).expect("start");
        let _ = engine.next_output().expect("choice");
        let mut snapshot = engine.snapshot().expect("snapshot");

        snapshot.runtime_frames.clear();
        let error = engine
            .resume(snapshot.clone())
            .expect_err("empty runtime frames should fail");
        assert_eq!(error.code, "SNAPSHOT_EMPTY");

        let mut fresh = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <choice text="Pick">
        <option text="A"><text>A</text></option>
      </choice>
    </script>
    "#,
        )]));
        fresh.start("main", None).expect("start fresh");
        let _ = fresh.next_output().expect("choice fresh");
        let mut bad_index = fresh.snapshot().expect("snapshot again");
        let frame = bad_index
            .runtime_frames
            .last_mut()
            .expect("snapshot should contain frame");
        frame.node_index = 9999;
        let error = fresh
            .resume(bad_index)
            .expect_err("invalid pending node index should fail");
        assert_eq!(error.code, "SNAPSHOT_PENDING_BOUNDARY");
    }

    #[test]
    pub(super) fn resume_and_boundary_shape_paths_are_covered() {
        let mut input_engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <temp name="name" type="string">"X"</temp>
      <input var="name" text="name?"/>
    </script>
    "#,
        )]));
        input_engine.start("main", None).expect("start");
        let input = input_engine.next_output().expect("input boundary");
        assert_eq!(output_kind(&input), "input");
        let input_snapshot = input_engine.snapshot().expect("snapshot");

        let mut choice_on_input = input_snapshot.clone();
        assert_eq!(pending_kind(&choice_on_input.pending_boundary), "input");
        let input_node_id = pending_node_id(&choice_on_input.pending_boundary);
        choice_on_input.pending_boundary = PendingBoundary::Choice {
            node_id: input_node_id,
            items: Vec::new(),
            prompt_text: None,
            dynamic_bindings: BTreeMap::new(),
        };
        let mut resume_choice = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <temp name="name" type="string">"X"</temp>
      <input var="name" text="name?"/>
    </script>
    "#,
        )]));
        let error = resume_choice
            .resume(choice_on_input)
            .expect_err("choice on input node should fail");
        assert_eq!(error.code, "SNAPSHOT_PENDING_BOUNDARY");

        let mut choice_engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <choice text="Pick">
        <option text="A"><text>A</text></option>
      </choice>
    </script>
    "#,
        )]));
        choice_engine.start("main", None).expect("start");
        let _ = choice_engine.next_output().expect("choice");
        let choice_snapshot = choice_engine.snapshot().expect("snapshot");

        let mut input_on_choice = choice_snapshot.clone();
        assert_eq!(pending_kind(&input_on_choice.pending_boundary), "choice");
        let choice_node_id = pending_node_id(&input_on_choice.pending_boundary);
        input_on_choice.pending_boundary = PendingBoundary::Input {
            node_id: choice_node_id,
            target_var: "name".to_string(),
            prompt_text: "p".to_string(),
            default_text: "d".to_string(),
            max_length: None,
        };
        let mut resume_input = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <choice text="Pick">
        <option text="A"><text>A</text></option>
      </choice>
    </script>
    "#,
        )]));
        let error = resume_input
            .resume(input_on_choice)
            .expect_err("input on choice node should fail");
        assert_eq!(error.code, "SNAPSHOT_PENDING_BOUNDARY");

        let mut input_mismatch = input_snapshot.clone();
        assert_eq!(pending_kind(&input_mismatch.pending_boundary), "input");
        input_mismatch.pending_boundary = PendingBoundary::Input {
            node_id: "missing-input-node".to_string(),
            target_var: "name".to_string(),
            prompt_text: "name?".to_string(),
            default_text: String::new(),
            max_length: None,
        };
        let mut resume_mismatch = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <temp name="name" type="string">"X"</temp>
      <input var="name" text="name?"/>
    </script>
    "#,
        )]));
        let error = resume_mismatch
            .resume(input_mismatch)
            .expect_err("input node mismatch should fail");
        assert_eq!(error.code, "SNAPSHOT_PENDING_BOUNDARY");

        let pending = RuntimePendingBoundary::Input {
            frame_id: 1,
            node_id: "n".to_string(),
            target_var: "name".to_string(),
            prompt_text: "p".to_string(),
            default_text: "d".to_string(),
            max_length: None,
        };
        let output = resume_mismatch.boundary_output(&pending);
        assert_eq!(output_kind(&output), "input");

        let mut with_resume = choice_snapshot.clone();
        let frame = with_resume
            .runtime_frames
            .last_mut()
            .expect("snapshot should contain frame");
        frame.completion = SnapshotCompletion::ResumeAfterChild;
        let mut resumed = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <choice text="Pick">
        <option text="A"><text>A</text></option>
      </choice>
    </script>
    "#,
        )]));
        resumed
            .resume(with_resume)
            .expect("resume after child completion should work");
    }

    #[test]
    pub(super) fn snapshot_resume_persists_module_vars() {
        let files = map(&[
            (
                "shared.xml",
                r#"
    <module name="shared" export="var:hp">
      <var name="hp" type="int">10</var>
    </module>
    "#,
            ),
            (
                "main.script.xml",
                r#"
    <!-- import shared from shared.xml -->
    <script name="main">
      <code>shared.hp = shared.hp + 5;</code>
      <choice text="Pick">
    <option text="A"><text>${shared.hp}</text></option>
      </choice>
    </script>
    "#,
            ),
        ]);

        let mut engine = engine_from_sources(files.clone());
        engine.start("main", None).expect("start");
        let first = engine.next_output().expect("choice");
        assert_eq!(output_kind(&first), "choices");
        let snapshot = engine.snapshot().expect("snapshot");
        assert_eq!(
            snapshot.module_vars.get("shared.hp"),
            Some(&SlValue::Number(15.0))
        );

        let mut resumed = engine_from_sources(files);
        resumed.resume(snapshot).expect("resume");
        resumed.choose(0).expect("choose");
        let text = resumed.next_output().expect("text");
        assert!(matches!(text, EngineOutput::Text { text, .. } if text == "15"));
    }

    #[test]
    pub(super) fn snapshot_does_not_store_module_consts_and_resume_rebuilds_them() {
        let files = map(&[(
            "main.xml",
            r#"<module name="main" export="script:main;const:base">
  <const name="base" type="int">7</const>
  <script name="main">
    <choice text="Pick">
      <option text="A"><text>${base}</text></option>
    </choice>
  </script>
</module>"#,
        )]);

        let mut engine = engine_from_sources(files.clone());
        engine.start("main.main", None).expect("start");
        let first = engine.next_output().expect("choice");
        assert_eq!(output_kind(&first), "choices");
        let snapshot = engine.snapshot().expect("snapshot");
        assert!(!snapshot.module_vars.contains_key("main.base"));

        let mut resumed = engine_from_sources(files);
        resumed.resume(snapshot).expect("resume");
        resumed.choose(0).expect("choose");
        let text = resumed.next_output().expect("text");
        assert!(matches!(text, EngineOutput::Text { text, .. } if text == "7"));
    }

    #[test]
    pub(super) fn resume_fails_when_module_const_declaration_missing() {
        let files = map(&[(
            "main.xml",
            r#"<module name="main" export="script:main;const:base">
  <const name="base" type="int">7</const>
  <script name="main">
    <choice text="Pick"><option text="A"><text>x</text></option></choice>
  </script>
</module>"#,
        )]);

        let mut engine = engine_from_sources(files.clone());
        engine.start("main.main", None).expect("start");
        let _ = engine.next_output().expect("choice");
        let snapshot = engine.snapshot().expect("snapshot");

        let mut resumed = engine_from_sources(files);
        resumed.module_const_declarations.clear();
        resumed.module_const_init_order = vec!["main.base".to_string()];
        let error = resumed
            .resume(snapshot)
            .expect_err("missing const declaration should fail");
        assert_eq!(error.code, "ENGINE_MODULE_CONST_DECL_MISSING");
    }

    #[test]
    pub(super) fn resume_validates_module_vars_shape_and_types() {
        let files = map(&[
            (
                "shared.xml",
                r#"
    <module name="shared" export="var:hp">
      <var name="hp" type="int">10</var>
    </module>
    "#,
            ),
            (
                "main.script.xml",
                r#"
    <!-- import shared from shared.xml -->
    <script name="main">
      <choice text="Pick">
    <option text="A"><text>${shared.hp}</text></option>
      </choice>
    </script>
    "#,
            ),
        ]);

        let mut engine = engine_from_sources(files.clone());
        engine.start("main", None).expect("start");
        let first = engine.next_output().expect("choice");
        assert_eq!(output_kind(&first), "choices");
        let snapshot = engine.snapshot().expect("snapshot");

        let mut unknown = snapshot.clone();
        unknown
            .module_vars
            .insert("missing.hp".to_string(), SlValue::Number(1.0));
        let mut unknown_engine = engine_from_sources(files.clone());
        let error = unknown_engine
            .resume(unknown)
            .expect_err("unknown module global should fail");
        assert_eq!(error.code, "SNAPSHOT_MODULE_GLOBAL_UNKNOWN");

        let mut bad_type = snapshot;
        bad_type
            .module_vars
            .insert("shared.hp".to_string(), SlValue::String("bad".to_string()));
        let mut bad_type_engine = engine_from_sources(files);
        let error = bad_type_engine
            .resume(bad_type)
            .expect_err("module global type mismatch should fail");
        assert_eq!(error.code, "SNAPSHOT_MODULE_GLOBAL_TYPE");
    }

    #[test]
    pub(super) fn resume_allows_missing_module_global_and_uses_type_default() {
        let files = map(&[
            (
                "shared.xml",
                r#"
    <module name="shared" export="var:hp">
      <var name="hp" type="int">10</var>
    </module>
    "#,
            ),
            (
                "main.script.xml",
                r#"
    <!-- import shared from shared.xml -->
    <script name="main">
      <choice text="Pick">
        <option text="A"><text>${shared.hp}</text></option>
      </choice>
    </script>
    "#,
            ),
        ]);

        let mut engine = engine_from_sources(files.clone());
        engine.start("main", None).expect("start");
        let _ = engine.next_output().expect("choice");
        let mut snapshot = engine.snapshot().expect("snapshot");
        snapshot.module_vars.remove("shared.hp");

        let mut resumed = engine_from_sources(files);
        resumed
            .resume(snapshot)
            .expect("resume should fill missing module global with default");
        resumed.choose(0).expect("choose");
        let text = resumed.next_output().expect("text");
        assert!(matches!(text, EngineOutput::Text { text, .. } if text == "0"));
    }

    #[test]
    pub(super) fn snapshot_resume_preserves_dynamic_choice_bindings() {
        let files = map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <temp name="arr" type="int[]">[5]</temp>
      <choice text="Pick">
        <dynamic-options array="arr" item="it" index="i">
          <option text="${it}-${i}">
            <text>${it}:${i}</text>
          </option>
        </dynamic-options>
      </choice>
    </script>
    "#,
        )]);

        let mut engine = engine_from_sources(files.clone());
        engine.start("main", None).expect("start");
        let first = engine.next_output().expect("choice");
        assert!(matches!(
            &first,
            EngineOutput::Choices { items, .. } if items.len() == 1 && items[0].text == "5-0"
        ));
        let snapshot = engine.snapshot().expect("snapshot");

        let mut resumed = engine_from_sources(files);
        resumed.resume(snapshot).expect("resume");
        resumed.choose(0).expect("choose");
        let out = resumed.next_output().expect("text");
        assert!(matches!(out, EngineOutput::Text { text, .. } if text == "5:0"));
    }

    #[test]
    pub(super) fn snapshot_resume_keeps_debug_event_flow() {
        let files = map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <choice text="Pick">
        <option text="A">
          <debug>dbg=${1+1}</debug>
          <text>ok</text>
        </option>
      </choice>
    </script>
    "#,
        )]);

        let mut engine = engine_from_sources(files.clone());
        engine.start("main", None).expect("start");
        let _ = engine.next_output().expect("choice");
        let snapshot = engine.snapshot().expect("snapshot");

        let mut resumed = engine_from_sources(files);
        resumed.resume(snapshot).expect("resume");
        resumed.choose(0).expect("choose");
        let debug = resumed.next_output().expect("debug");
        assert!(matches!(debug, EngineOutput::Debug { text } if text == "dbg=2"));
        let text = resumed.next_output().expect("text");
        assert!(matches!(text, EngineOutput::Text { text, .. } if text == "ok"));
    }

    #[test]
    pub(super) fn resume_preserves_sequence_rng_mode() {
        let sources = map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <choice text="Pick">
        <option text="A"><text>A</text></option>
      </choice>
    </script>
    "#,
        )]);
        let compiled = compile_project_from_sources(sources);
        let mut source = ScriptLangEngine::new(ScriptLangEngineOptions {
            scripts: compiled.scripts.clone(),
            global_data: compiled.global_data.clone(),
            module_var_declarations: compiled.module_var_declarations.clone(),
            module_var_init_order: compiled.module_var_init_order.clone(),
            module_const_declarations: compiled.module_const_declarations.clone(),
            module_const_init_order: compiled.module_const_init_order.clone(),
            host_functions: None,
            random_seed: Some(1),
            random_sequence: Some(vec![7, 9]),
            random_sequence_index: Some(0),
            compiler_version: Some(DEFAULT_COMPILER_VERSION.to_string()),
        })
        .expect("source engine");
        source.start("main", None).expect("start");
        assert_eq!(
            output_kind(&source.next_output().expect("choice")),
            "choices"
        );
        let snapshot = source.snapshot().expect("snapshot");

        let mut target = ScriptLangEngine::new(ScriptLangEngineOptions {
            scripts: compiled.scripts,
            global_data: compiled.global_data,
            module_var_declarations: compiled.module_var_declarations,
            module_var_init_order: compiled.module_var_init_order,
            module_const_declarations: compiled.module_const_declarations,
            module_const_init_order: compiled.module_const_init_order,
            host_functions: None,
            random_seed: Some(1),
            random_sequence: Some(vec![7, 9]),
            random_sequence_index: Some(0),
            compiler_version: Some(DEFAULT_COMPILER_VERSION.to_string()),
        })
        .expect("target engine");
        target.resume(snapshot).expect("resume");
        let random_state_pair = |view: RandomStateView| match view {
            RandomStateView::Sequence { values, index } => (values, index),
            RandomStateView::Seeded { .. } => (Vec::new(), usize::MAX),
        };
        let (values, index) = random_state_pair(target.random_state_snapshot());
        assert_eq!(values, vec![7, 9]);
        assert_eq!(index, 0);
        let (fallback_values, fallback_index) = random_state_pair(
            engine_from_sources(map(&[(
                "main.script.xml",
                r#"<script name="main"><text>x</text></script>"#,
            )]))
            .random_state_snapshot(),
        );
        assert!(fallback_values.is_empty());
        assert_eq!(fallback_index, usize::MAX);
    }
}
