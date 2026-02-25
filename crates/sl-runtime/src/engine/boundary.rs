impl ScriptLangEngine {
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

        let Some(ScriptNode::Choice {
            options: node_options,
            ..
        }) = group.nodes.get(node_index)
        else {
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

}
