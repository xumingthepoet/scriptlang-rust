impl ScriptLangEngine {
    fn reset(&mut self) {
        self.frames.clear();
        self.pending_boundary = None;
        self.waiting_choice = false;
        self.ended = false;
        self.frame_counter = 1;
        self.rng_state = self.initial_random_seed;
        *self.shared_rng_state.borrow_mut() = self.initial_random_seed;
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
    ) -> Result<(&str, &sl_core::ImplicitGroup), ScriptLangError> {
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

        Ok((&lookup.script_name, group))
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

}

#[cfg(test)]
mod frame_stack_tests {
    use super::*;
    use super::runtime_test_support::*;

    #[test]
    fn internal_state_error_paths_are_covered() {
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
        let first = engine.next_output().expect("choice");
        assert!(matches!(first, EngineOutput::Choices { .. }));
        let mut items = Vec::new();
        let mut prompt_text = None;
        if let Some(PendingBoundary::Choice {
            options,
            prompt_text: choice_prompt,
            ..
        }) = engine.pending_boundary.clone()
        {
            items = options;
            prompt_text = choice_prompt;
        }
        assert!(!items.is_empty());
        let frame_id = engine.frames.last().expect("frame").frame_id;
        engine.pending_boundary = Some(PendingBoundary::Choice {
            frame_id,
            node_id: "x".to_string(),
            options: items.clone(),
            prompt_text: prompt_text.clone(),
        });
        let again = engine.next_output().expect("pending boundary should echo");
        assert!(matches!(again, EngineOutput::Choices { .. }));
    
        engine.pending_boundary = None;
        engine.ended = true;
        let end = engine.next_output().expect("ended should return end");
        assert!(matches!(end, EngineOutput::End));
    
        engine.pending_boundary = Some(PendingBoundary::Choice {
            frame_id: 999_999,
            node_id: "x".to_string(),
            options: items.clone(),
            prompt_text,
        });
        let error = engine.choose(0).expect_err("missing frame should fail");
        assert_eq!(error.code, "ENGINE_CHOICE_FRAME_MISSING");
    
        engine.pending_boundary = Some(PendingBoundary::Input {
            frame_id: 999_999,
            node_id: "x".to_string(),
            target_var: "name".to_string(),
            prompt_text: "p".to_string(),
            default_text: "d".to_string(),
        });
        let error = engine
            .submit_input("abc")
            .expect_err("missing input frame should fail");
        assert_eq!(error.code, "ENGINE_INPUT_FRAME_MISSING");
    
        let mut helper_engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>Hello</text></script>"#,
        )]));
        let err = helper_engine.top_frame_id().expect_err("no frame");
        assert_eq!(err.code, "ENGINE_NO_FRAME");
        let err = helper_engine
            .bump_top_node_index(1)
            .expect_err("no frame for bump");
        assert_eq!(err.code, "ENGINE_NO_FRAME");
    
        let err = helper_engine
            .push_group_frame("missing-group", CompletionKind::ResumeAfterChild)
            .expect_err("missing group");
        assert_eq!(err.code, "ENGINE_GROUP_NOT_FOUND");
    
        assert!(helper_engine.finish_frame(123).is_ok());
        let err = helper_engine
            .find_current_root_frame_index()
            .expect_err("no root frame");
        assert_eq!(err.code, "ENGINE_ROOT_FRAME");
    }

}
