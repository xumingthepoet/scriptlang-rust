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

#[cfg(test)]
mod boundary_tests {
    use super::*;
    use super::runtime_test_support::*;

    #[test]
    fn choose_and_input_validate_pending_boundary_state() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>Hello</text></script>"#,
        )]));
        engine.start("main", None).expect("start");
    
        let choose_error = engine.choose(0).expect_err("no pending choice");
        assert_eq!(choose_error.code, "ENGINE_NO_PENDING_CHOICE");
        let input_error = engine.submit_input("x").expect_err("no pending input");
        assert_eq!(input_error.code, "ENGINE_NO_PENDING_INPUT");
    }

    #[test]
    fn choose_rejects_out_of_range_index() {
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
        let first = engine.next_output().expect("next");
        assert!(matches!(first, EngineOutput::Choices { .. }));
        let error = engine.choose(9).expect_err("index out of range");
        assert_eq!(error.code, "ENGINE_CHOICE_INDEX");
    }

    #[test]
    fn submit_input_uses_default_value_for_blank_input() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <var name="heroName" type="string">&quot;Traveler&quot;</var>
      <input var="heroName" text="Name your hero"/>
      <text>Hello ${heroName}</text>
    </script>
    "#,
        )]));
        engine.start("main", None).expect("start");
        let first = engine.next_output().expect("next");
        assert!(matches!(first, EngineOutput::Input { .. }));
        engine.submit_input("   ").expect("submit input");
        let second = engine.next_output().expect("next");
        let mut text = String::new();
        if let EngineOutput::Text { text: output } = second {
            text = output;
        }
        assert_eq!(text, "Hello Traveler");
    }

    #[test]
    fn submit_input_uses_provided_non_empty_value() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <var name="heroName" type="string">&quot;Traveler&quot;</var>
      <input var="heroName" text="Name your hero"/>
      <text>Hello ${heroName}</text>
    </script>
    "#,
        )]));
        engine.start("main", None).expect("start");
        let first = engine.next_output().expect("next");
        assert!(matches!(first, EngineOutput::Input { .. }));
        engine.submit_input("Guild").expect("submit input");
        let second = engine.next_output().expect("next");
        let mut text = String::new();
        if let EngineOutput::Text { text: output } = second {
            text = output;
        }
        assert_eq!(text, "Hello Guild");
    }

}
