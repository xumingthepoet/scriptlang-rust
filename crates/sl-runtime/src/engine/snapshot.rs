use super::lifecycle::{CompletionKind, PendingBoundary, RuntimeFrame};
use super::*;

impl ScriptLangEngine {
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
            defs_globals: self.defs_globals_value.clone(),
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
        *self.shared_rng_state.borrow_mut() = snapshot.rng_state;

        for qualified_name in snapshot.defs_globals.keys() {
            if !self.defs_global_declarations.contains_key(qualified_name) {
                return Err(ScriptLangError::new(
                    "SNAPSHOT_DEFS_GLOBAL_UNKNOWN",
                    format!(
                        "Snapshot contains unknown defs global \"{}\".",
                        qualified_name
                    ),
                ));
            }
        }

        let mut restored_defs_globals = BTreeMap::new();
        for (qualified_name, decl) in &self.defs_global_declarations {
            let value = snapshot
                .defs_globals
                .get(qualified_name)
                .cloned()
                .unwrap_or_else(|| default_value_from_type(&decl.r#type));
            if !is_type_compatible(&value, &decl.r#type) {
                return Err(ScriptLangError::new(
                    "SNAPSHOT_DEFS_GLOBAL_TYPE",
                    format!(
                        "Defs global \"{}\" from snapshot does not match declared type.",
                        qualified_name
                    ),
                ));
            }
            restored_defs_globals.insert(qualified_name.clone(), value);
        }
        self.defs_globals_value = restored_defs_globals;

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

        let (script_name, node) = {
            let (script_name, group) = self.lookup_group(&top.group_id)?;
            let node = group
                .nodes
                .get(top.node_index)
                .ok_or_else(|| {
                    ScriptLangError::new("SNAPSHOT_PENDING_BOUNDARY", "Pending node index invalid.")
                })?
                .clone();
            (script_name.to_string(), node)
        };

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
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    use crate::engine::runtime_test_support::*;

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
        let next = resumed.next_output().expect("next");
        assert!(matches!(next, EngineOutput::Text { .. }));
    }

    #[test]
    pub(super) fn snapshot_and_resume_cover_while_completion_and_once_state() {
        let files = map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <var name="n" type="int">1</var>
      <text once="true">Intro</text>
      <while when="n > 0">
        <choice text="Pick">
          <option text="Stop"><code>n = 0;</code></option>
        </choice>
      </while>
    </script>
    "#,
        )]);

        let mut engine = engine_from_sources(files.clone());
        engine.start("main", None).expect("start");
        assert!(matches!(
            engine.next_output().expect("text"),
            EngineOutput::Text { .. }
        ));
        assert!(matches!(
            engine.next_output().expect("choice"),
            EngineOutput::Choices { .. }
        ));
        let snapshot = engine.snapshot().expect("snapshot");
        assert!(!snapshot.once_state_by_script.is_empty());

        let mut resumed = engine_from_sources(files);
        resumed.resume(snapshot).expect("resume");
        resumed.choose(0).expect("choose should pass");
        assert!(matches!(
            resumed.next_output().expect("end"),
            EngineOutput::End
        ));
    }

    #[test]
    pub(super) fn resume_restores_pending_input_boundary() {
        let files = map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <var name="heroName" type="string">&quot;Traveler&quot;</var>
      <input var="heroName" text="Name your hero"/>
      <text>Hello ${heroName}</text>
    </script>
    "#,
        )]);

        let mut engine = engine_from_sources(files.clone());
        engine.start("main", None).expect("start");
        assert!(matches!(
            engine.next_output().expect("input"),
            EngineOutput::Input { .. }
        ));
        let snapshot = engine.snapshot().expect("snapshot");

        let mut resumed = engine_from_sources(files);
        resumed.resume(snapshot).expect("resume");
        assert!(matches!(
            resumed.next_output().expect("input"),
            EngineOutput::Input { .. }
        ));
        resumed.submit_input("Guild").expect("submit input");
        assert!(matches!(
            resumed.next_output().expect("text"),
            EngineOutput::Text { .. }
        ));
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
        assert!(matches!(first, EngineOutput::Choices { .. }));
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
        assert!(matches!(first, EngineOutput::Choices { .. }));
        let mut snapshot = engine.snapshot().expect("snapshot");
        assert!(matches!(
            snapshot.pending_boundary,
            PendingBoundaryV3::Choice { .. }
        ));
        snapshot.pending_boundary = PendingBoundaryV3::Choice {
            node_id: "invalid-node-id".to_string(),
            items: Vec::new(),
            prompt_text: None,
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
      <var name="name" type="string">&quot;X&quot;</var>
      <input var="name" text="name?"/>
    </script>
    "#,
        )]));
        input_engine.start("main", None).expect("start");
        let input = input_engine.next_output().expect("input boundary");
        assert!(matches!(input, EngineOutput::Input { .. }));
        let input_snapshot = input_engine.snapshot().expect("snapshot");

        let mut choice_on_input = input_snapshot.clone();
        assert!(matches!(
            choice_on_input.pending_boundary,
            PendingBoundaryV3::Input { .. }
        ));
        let mut input_node_id = None;
        if let PendingBoundaryV3::Input { node_id, .. } = &choice_on_input.pending_boundary {
            input_node_id = Some(node_id.clone());
        }
        let input_node_id = input_node_id.expect("snapshot should contain input boundary");
        choice_on_input.pending_boundary = PendingBoundaryV3::Choice {
            node_id: input_node_id,
            items: Vec::new(),
            prompt_text: None,
        };
        let mut resume_choice = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <var name="name" type="string">&quot;X&quot;</var>
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
        assert!(matches!(
            input_on_choice.pending_boundary,
            PendingBoundaryV3::Choice { .. }
        ));
        let mut choice_node_id = None;
        if let PendingBoundaryV3::Choice { node_id, .. } = &input_on_choice.pending_boundary {
            choice_node_id = Some(node_id.clone());
        }
        let choice_node_id = choice_node_id.expect("snapshot should contain choice boundary");
        input_on_choice.pending_boundary = PendingBoundaryV3::Input {
            node_id: choice_node_id,
            target_var: "name".to_string(),
            prompt_text: "p".to_string(),
            default_text: "d".to_string(),
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
        assert!(matches!(
            input_mismatch.pending_boundary,
            PendingBoundaryV3::Input { .. }
        ));
        input_mismatch.pending_boundary = PendingBoundaryV3::Input {
            node_id: "missing-input-node".to_string(),
            target_var: "name".to_string(),
            prompt_text: "name?".to_string(),
            default_text: String::new(),
        };
        let mut resume_mismatch = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <var name="name" type="string">&quot;X&quot;</var>
      <input var="name" text="name?"/>
    </script>
    "#,
        )]));
        let error = resume_mismatch
            .resume(input_mismatch)
            .expect_err("input node mismatch should fail");
        assert_eq!(error.code, "SNAPSHOT_PENDING_BOUNDARY");

        let pending = PendingBoundary::Input {
            frame_id: 1,
            node_id: "n".to_string(),
            target_var: "name".to_string(),
            prompt_text: "p".to_string(),
            default_text: "d".to_string(),
        };
        let output = resume_mismatch.boundary_output(&pending);
        assert!(matches!(output, EngineOutput::Input { .. }));

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
    pub(super) fn snapshot_resume_persists_defs_globals() {
        let files = map(&[
            (
                "shared.defs.xml",
                r#"
    <defs name="shared">
      <var name="hp" type="int">10</var>
    </defs>
    "#,
            ),
            (
                "main.script.xml",
                r#"
    <!-- include: shared.defs.xml -->
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
        assert!(matches!(first, EngineOutput::Choices { .. }));
        let snapshot = engine.snapshot().expect("snapshot");
        assert_eq!(
            snapshot.defs_globals.get("shared.hp"),
            Some(&SlValue::Number(15.0))
        );

        let mut resumed = engine_from_sources(files);
        resumed.resume(snapshot).expect("resume");
        resumed.choose(0).expect("choose");
        let text = resumed.next_output().expect("text");
        assert!(matches!(text, EngineOutput::Text { text, .. } if text == "15"));
    }

    #[test]
    pub(super) fn resume_validates_defs_globals_shape_and_types() {
        let files = map(&[
            (
                "shared.defs.xml",
                r#"
    <defs name="shared">
      <var name="hp" type="int">10</var>
    </defs>
    "#,
            ),
            (
                "main.script.xml",
                r#"
    <!-- include: shared.defs.xml -->
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
        assert!(matches!(first, EngineOutput::Choices { .. }));
        let snapshot = engine.snapshot().expect("snapshot");

        let mut unknown = snapshot.clone();
        unknown
            .defs_globals
            .insert("missing.hp".to_string(), SlValue::Number(1.0));
        let mut unknown_engine = engine_from_sources(files.clone());
        let error = unknown_engine
            .resume(unknown)
            .expect_err("unknown defs global should fail");
        assert_eq!(error.code, "SNAPSHOT_DEFS_GLOBAL_UNKNOWN");

        let mut bad_type = snapshot;
        bad_type
            .defs_globals
            .insert("shared.hp".to_string(), SlValue::String("bad".to_string()));
        let mut bad_type_engine = engine_from_sources(files);
        let error = bad_type_engine
            .resume(bad_type)
            .expect_err("defs global type mismatch should fail");
        assert_eq!(error.code, "SNAPSHOT_DEFS_GLOBAL_TYPE");
    }
}
