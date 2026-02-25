impl ScriptLangEngine {
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
        let Some((choice_frame_index, choice_node_index)) = self.find_choice_continue_context()?
        else {
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
            let Some(ScriptNode::Choice { options, .. }) = group.nodes.get(choice_node_index)
            else {
                continue;
            };

            let option_group_ids = options
                .iter()
                .map(|option| option.group_id.clone())
                .collect::<BTreeSet<_>>();

            let has_deep_option_frame = (frame_index + 1..self.frames.len())
                .any(|deep_index| option_group_ids.contains(&self.frames[deep_index].group_id));
            if has_deep_option_frame {
                return Ok(Some((frame_index, choice_node_index)));
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

}

#[cfg(test)]
mod control_flow_tests {
    use super::runtime_test_support::*;

    #[test]
    fn runtime_errors_cover_break_continue_and_return_args() {
        let mut source = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <choice text="Pick">
        <option text="A"><text>A</text></option>
      </choice>
    </script>
    "#,
        )]));
        source.start("main", None).expect("start");
        let _ = source.next_output().expect("choice");
        let mut snapshot = source.snapshot().expect("snapshot");
        if let Some(frame) = snapshot.runtime_frames.last_mut() {
            frame.group_id = "missing-group".to_string();
        }
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
        let error = resumed
            .resume(snapshot)
            .expect_err("missing group in snapshot should fail");
        assert_eq!(error.code, "ENGINE_GROUP_NOT_FOUND");
    
        let mut return_arg_unknown = engine_from_sources(map(&[
            (
                "main.script.xml",
                r#"<script name="main"><return script="next" args="1,2"/></script>"#,
            ),
            (
                "next.script.xml",
                r#"<script name="next" args="int:x"><text>${x}</text></script>"#,
            ),
        ]));
        return_arg_unknown.start("main", None).expect("start");
        let error = return_arg_unknown
            .next_output()
            .expect_err("extra return arg should fail");
        assert_eq!(error.code, "ENGINE_RETURN_ARG_UNKNOWN");
    }

}
