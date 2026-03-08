use super::lifecycle::{CompletionKind, PendingBoundary, PendingChoiceOption, RuntimeFrame};
use super::*;

impl ScriptLangEngine {
    fn bump_top_node_index_infallible(&mut self, amount: usize) {
        self.bump_top_node_index(amount)
            .expect("top frame should exist while stepping");
    }

    pub fn next_output(&mut self) -> Result<EngineOutput, ScriptLangError> {
        if let Some(boundary) = &self.pending_boundary {
            return Ok(self.boundary_output(boundary));
        }

        if self.ended {
            return Ok(EngineOutput::End);
        }

        let mut guard = 0usize;
        while guard < 10_000 {
            guard += 1;

            let Some((top_frame_id, top_group_id, top_node_index)) = self
                .frames
                .last()
                .map(|frame| (frame.frame_id, frame.group_id.clone(), frame.node_index))
            else {
                self.ended = true;
                return Ok(EngineOutput::End);
            };

            enum PlannedNode {
                FinishFrame {
                    frame_id: u64,
                },
                Text {
                    script_name: String,
                    value: String,
                    tag: Option<String>,
                    once: bool,
                    id: String,
                },
                Debug {
                    value: String,
                },
                Code {
                    code: String,
                },
                Var {
                    declaration: sl_core::VarDeclaration,
                },
                If {
                    when_expr: String,
                    then_group_id: String,
                    else_group_id: Option<String>,
                },
                While {
                    when_expr: String,
                    body_group_id: String,
                },
                Choice {
                    script_name: String,
                    id: String,
                    entries: Vec<ChoiceEntry>,
                    prompt_text: String,
                },
                Input {
                    id: String,
                    target_var: String,
                    prompt_text: String,
                },
                Call {
                    target_script: String,
                    args: Vec<sl_core::CallArgument>,
                },
                Return {
                    target_script: Option<String>,
                    args: Vec<sl_core::CallArgument>,
                },
                Break,
                Continue {
                    target: ContinueTarget,
                },
            }

            let planned_node = {
                let (script_name, group) = self.lookup_group(&top_group_id)?;
                if top_node_index >= group.nodes.len() {
                    PlannedNode::FinishFrame {
                        frame_id: top_frame_id,
                    }
                } else {
                    match &group.nodes[top_node_index] {
                        ScriptNode::Text {
                            value,
                            tag,
                            once,
                            id,
                            ..
                        } => PlannedNode::Text {
                            script_name: script_name.to_string(),
                            value: value.clone(),
                            tag: tag.clone(),
                            once: *once,
                            id: id.clone(),
                        },
                        ScriptNode::Debug { value, .. } => PlannedNode::Debug {
                            value: value.clone(),
                        },
                        ScriptNode::Code { code, .. } => PlannedNode::Code { code: code.clone() },
                        ScriptNode::Var { declaration, .. } => PlannedNode::Var {
                            declaration: declaration.clone(),
                        },
                        ScriptNode::If {
                            when_expr,
                            then_group_id,
                            else_group_id,
                            ..
                        } => PlannedNode::If {
                            when_expr: when_expr.clone(),
                            then_group_id: then_group_id.clone(),
                            else_group_id: else_group_id.clone(),
                        },
                        ScriptNode::While {
                            when_expr,
                            body_group_id,
                            ..
                        } => PlannedNode::While {
                            when_expr: when_expr.clone(),
                            body_group_id: body_group_id.clone(),
                        },
                        ScriptNode::Choice {
                            id,
                            entries,
                            prompt_text,
                            ..
                        } => PlannedNode::Choice {
                            script_name: script_name.to_string(),
                            id: id.clone(),
                            entries: entries.clone(),
                            prompt_text: prompt_text.clone(),
                        },
                        ScriptNode::Input {
                            id,
                            target_var,
                            prompt_text,
                            ..
                        } => PlannedNode::Input {
                            id: id.clone(),
                            target_var: target_var.clone(),
                            prompt_text: prompt_text.clone(),
                        },
                        ScriptNode::Call {
                            target_script,
                            args,
                            ..
                        } => PlannedNode::Call {
                            target_script: target_script.clone(),
                            args: args.clone(),
                        },
                        ScriptNode::Return {
                            target_script,
                            args,
                            ..
                        } => PlannedNode::Return {
                            target_script: target_script.clone(),
                            args: args.clone(),
                        },
                        ScriptNode::Break { .. } => PlannedNode::Break,
                        ScriptNode::Continue { target, .. } => PlannedNode::Continue {
                            target: target.clone(),
                        },
                    }
                }
            };

            match planned_node {
                PlannedNode::FinishFrame { frame_id } => {
                    self.finish_frame(frame_id)?;
                    continue;
                }
                PlannedNode::Text {
                    script_name,
                    value,
                    tag,
                    once,
                    id,
                } => {
                    if once && self.has_once_state(&script_name, &format!("text:{}", id)) {
                        self.bump_top_node_index_infallible(1);
                        continue;
                    }

                    let rendered = self.render_text(&value)?;
                    self.bump_top_node_index_infallible(1);

                    if once {
                        self.mark_once_state(&script_name, &format!("text:{}", id));
                    }

                    return Ok(EngineOutput::Text {
                        text: rendered,
                        tag,
                    });
                }
                PlannedNode::Debug { value } => {
                    let rendered = self.render_text(&value)?;
                    self.bump_top_node_index_infallible(1);
                    return Ok(EngineOutput::Debug { text: rendered });
                }
                PlannedNode::Code { code } => {
                    self.run_code(&code)?;
                    self.bump_top_node_index_infallible(1);
                }
                PlannedNode::Var { declaration } => {
                    self.execute_var_declaration(&declaration)?;
                    self.bump_top_node_index_infallible(1);
                }
                PlannedNode::If {
                    when_expr,
                    then_group_id,
                    else_group_id,
                } => {
                    let condition = self.eval_boolean(&when_expr)?;
                    self.bump_top_node_index_infallible(1);
                    if condition {
                        self.push_group_frame(&then_group_id, CompletionKind::ResumeAfterChild)
                            .expect("compiler should emit existing then group");
                    } else {
                        let else_group_id = else_group_id
                            .expect("compiler should always synthesize an else group id");
                        self.push_group_frame(&else_group_id, CompletionKind::ResumeAfterChild)
                            .expect("compiler should emit existing else group");
                    }
                }
                PlannedNode::While {
                    when_expr,
                    body_group_id,
                } => {
                    let condition = self.eval_boolean(&when_expr)?;
                    if condition {
                        self.push_group_frame(&body_group_id, CompletionKind::WhileBody)
                            .expect("compiler should emit existing while body group");
                    } else {
                        self.bump_top_node_index_infallible(1);
                    }
                }
                PlannedNode::Choice {
                    script_name,
                    id,
                    entries,
                    prompt_text,
                } => {
                    let mut visible_regular = Vec::<PendingChoiceOption>::new();
                    let mut visible_fall_over = None;
                    let mut dynamic_block_ordinal = 0usize;

                    for entry in &entries {
                        match entry {
                            ChoiceEntry::Static { option } => {
                                if option.fall_over {
                                    let visible = !option.once
                                        || !self.has_once_state(
                                            &script_name,
                                            &format!("option:{}", option.id),
                                        );
                                    if visible {
                                        visible_fall_over = Some(PendingChoiceOption {
                                            item: ChoiceItem {
                                                index: 0,
                                                id: option.id.clone(),
                                                text: self.render_text(&option.text)?,
                                            },
                                            dynamic_binding: None,
                                        });
                                    }
                                    continue;
                                }

                                if self.is_choice_option_visible(&script_name, option)? {
                                    visible_regular.push(PendingChoiceOption {
                                        item: ChoiceItem {
                                            index: 0,
                                            id: option.id.clone(),
                                            text: self.render_text(&option.text)?,
                                        },
                                        dynamic_binding: None,
                                    });
                                }
                            }
                            ChoiceEntry::Dynamic { block } => {
                                let array_value = self.eval_expression(&block.array_expr)?;
                                let SlValue::Array(items) = array_value else {
                                    return Err(ScriptLangError::new(
                                        "ENGINE_CHOICE_ARRAY_NOT_ARRAY",
                                        format!(
                                            "dynamic-options array expression \"{}\" must evaluate to array.",
                                            block.array_expr
                                        ),
                                    ));
                                };

                                for (element_index, element_value) in items.into_iter().enumerate()
                                {
                                    let binding = (&element_value, element_index);
                                    let visible = self.dynamic_choice_when(block, binding)?;

                                    if !visible {
                                        continue;
                                    }

                                    let rendered_text = self.dynamic_choice_text(block, binding)?;

                                    visible_regular.push(PendingChoiceOption {
                                        item: ChoiceItem {
                                            index: 0,
                                            id: format!(
                                                "dyn:{}:{}:{}",
                                                id, dynamic_block_ordinal, element_index
                                            ),
                                            text: rendered_text,
                                        },
                                        dynamic_binding: Some(PendingDynamicChoiceBinding {
                                            group_id: block.template.group_id.clone(),
                                            item_name: block.item_name.clone(),
                                            item_value: element_value,
                                            index_name: block.index_name.clone(),
                                            index_value: block
                                                .index_name
                                                .as_ref()
                                                .map(|_| element_index),
                                        }),
                                    });
                                }

                                dynamic_block_ordinal += 1;
                            }
                        }
                    }

                    let visible_options = if visible_regular.is_empty() {
                        visible_fall_over.into_iter().collect::<Vec<_>>()
                    } else {
                        visible_regular
                    };

                    if visible_options.is_empty() {
                        self.bump_top_node_index_infallible(1);
                        continue;
                    }

                    let mut pending_options = visible_options;
                    for (index, option) in pending_options.iter_mut().enumerate() {
                        option.item.index = index;
                    }
                    let items = pending_options
                        .iter()
                        .map(|option| option.item.clone())
                        .collect::<Vec<_>>();

                    let prompt_text = Some(self.render_text(&prompt_text)?);
                    let frame_id = top_frame_id;
                    self.pending_boundary = Some(PendingBoundary::Choice {
                        frame_id,
                        node_id: id,
                        options: pending_options,
                        prompt_text: prompt_text.clone(),
                    });
                    self.waiting_choice = true;
                    return Ok(EngineOutput::Choices { items, prompt_text });
                }
                PlannedNode::Input {
                    id,
                    target_var,
                    prompt_text,
                } => {
                    let current = self.read_path(&target_var)?;
                    let SlValue::String(default_text) = current else {
                        return Err(ScriptLangError::new(
                            "ENGINE_INPUT_VAR_TYPE",
                            format!("Input target var \"{}\" must be string.", target_var),
                        ));
                    };

                    let frame_id = top_frame_id;
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
                PlannedNode::Call {
                    target_script,
                    args,
                } => {
                    self.execute_call(&target_script, &args)?;
                }
                PlannedNode::Return {
                    target_script,
                    args,
                } => {
                    self.execute_return(target_script, &args)?;
                }
                PlannedNode::Break => {
                    self.execute_break()?;
                }
                PlannedNode::Continue { target } => match target {
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

    fn dynamic_choice_when(
        &mut self,
        block: &sl_core::DynamicChoiceBlock,
        binding: (&SlValue, usize),
    ) -> Result<bool, ScriptLangError> {
        let (element_value, element_index) = binding;
        match block.template.when_expr.as_ref() {
            Some(when_expr) => self.with_dynamic_choice_bindings(
                &block.item_name,
                element_value,
                block.index_name.as_deref(),
                element_index,
                |engine| engine.eval_boolean(when_expr),
            ),
            None => Ok(true),
        }
    }

    fn dynamic_choice_text(
        &mut self,
        block: &sl_core::DynamicChoiceBlock,
        binding: (&SlValue, usize),
    ) -> Result<String, ScriptLangError> {
        let (element_value, element_index) = binding;
        self.with_dynamic_choice_bindings(
            &block.item_name,
            element_value,
            block.index_name.as_deref(),
            element_index,
            |engine| engine.render_text(&block.template.text),
        )
    }

    fn with_dynamic_choice_bindings<T, F>(
        &mut self,
        item_name: &str,
        item_value: &SlValue,
        index_name: Option<&str>,
        index_value: usize,
        evaluator: F,
    ) -> Result<T, ScriptLangError>
    where
        F: FnOnce(&mut ScriptLangEngine) -> Result<T, ScriptLangError>,
    {
        let frame = self
            .frames
            .last_mut()
            .expect("dynamic choice binding requires active frame");

        let item_previous = frame
            .scope
            .insert(item_name.to_string(), item_value.clone());
        let index_previous = if let Some(index_name) = index_name {
            frame
                .scope
                .insert(index_name.to_string(), SlValue::Number(index_value as f64))
        } else {
            None
        };

        let result = evaluator(self);

        let frame = self
            .frames
            .last_mut()
            .expect("dynamic choice binding restore requires active frame");

        if let Some(previous) = item_previous {
            frame.scope.insert(item_name.to_string(), previous);
        } else {
            frame.scope.remove(item_name);
        }

        if let Some(index_name) = index_name {
            if let Some(previous) = index_previous {
                frame.scope.insert(index_name.to_string(), previous);
            } else {
                frame.scope.remove(index_name);
            }
        }

        result
    }
}

#[cfg(test)]
mod step_tests {
    use super::runtime_test_support::*;
    use super::*;

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

    fn pending_choice_options_mut(
        pending: &mut PendingBoundary,
    ) -> Option<&mut Vec<PendingChoiceOption>> {
        match pending {
            PendingBoundary::Choice { options, .. } => Some(options),
            PendingBoundary::Input { .. } => None,
        }
    }

    fn take_choice_items(output: EngineOutput) -> Option<Vec<ChoiceItem>> {
        match output {
            EngineOutput::Choices { items, .. } => Some(items),
            _ => None,
        }
    }

    #[test]
    pub(super) fn next_text_and_end() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>Hello</text></script>"#,
        )]));
        engine.start("main", None).expect("start");

        let first = engine.next_output().expect("next");
        assert_eq!(output_kind(&first), "text");

        let second = engine.next_output().expect("next");
        assert_eq!(output_kind(&second), "end");
    }

    #[test]
    pub(super) fn next_debug_interpolates_and_keeps_order_with_text() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
<script name="main">
  <var name="hp" type="int">2</var>
  <debug>dbg hp=${hp}</debug>
  <text>text hp=${hp}</text>
</script>
"#,
        )]));
        engine.start("main", None).expect("start");

        let first = engine.next_output().expect("next");
        assert!(matches!(
            first,
            EngineOutput::Debug { text } if text == "dbg hp=2"
        ));

        let second = engine.next_output().expect("next");
        assert!(matches!(
            second,
            EngineOutput::Text { text, .. } if text == "text hp=2"
        ));

        let third = engine.next_output().expect("next");
        assert_eq!(output_kind(&third), "end");
    }

    #[test]
    pub(super) fn next_debug_propagates_interpolation_errors() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><debug>${missing}</debug></script>"#,
        )]));
        engine.start("main", None).expect("start");
        let error = engine
            .next_output()
            .expect_err("missing variable should fail");
        assert_eq!(error.code, "ENGINE_EVAL_ERROR");
    }

    #[test]
    pub(super) fn next_text_preserves_optional_tag() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text tag="sound">sfx/open.ogg</text></script>"#,
        )]));
        engine.start("main", None).expect("start");

        let first = engine.next_output().expect("next");
        assert!(matches!(
            first,
            EngineOutput::Text { text, tag } if text == "sfx/open.ogg" && tag == Some("sound".to_string())
        ));
    }

    #[test]
    pub(super) fn next_text_treats_blank_tag_as_none() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text tag="   ">plain</text></script>"#,
        )]));
        engine.start("main", None).expect("start");

        let first = engine.next_output().expect("next");
        assert!(matches!(
            first,
            EngineOutput::Text { text, tag } if text == "plain" && tag.is_none()
        ));
    }

    #[test]
    pub(super) fn drives_complex_flow_to_end() {
        let files = map(&[
            (
                "main.script.xml",
                r#"
<script name="main">
  <var name="name" type="string">"Traveler"</var>
  <debug>dbg=${name}</debug>
  <choice text="Pick">
    <option text="A"><text>A</text></option>
  </choice>
  <input var="name" text="Name"/>
  <call script="next.next"/>
  <text>done ${name}</text>
</script>
"#,
            ),
            (
                "next.script.xml",
                r#"
<script name="next">
  <text>Next</text>
</script>
"#,
            ),
        ]);

        let mut engine = engine_from_sources(files);
        engine.start("main.main", None).expect("start main");
        drive_engine_to_end(&mut engine);
    }

    #[test]
    pub(super) fn defs_global_shadowing_example_behaves_as_expected() {
        let files = map(&[
            (
                "shared.defs.xml",
                r#"
<defs name="shared">
  <var name="hp" type="int">100</var>
</defs>
"#,
            ),
            (
                "battle.script.xml",
                r#"
<!-- include: shared.defs.xml -->
<script name="battle">
  <var name="hp" type="int">30</var>
  <code>hp = hp + 5; shared.hp = shared.hp - 40;</code>
  <text>battle.local=${hp}</text>
  <text>battle.global=${shared.hp}</text>
  <return/>
</script>
"#,
            ),
            (
                "main.script.xml",
                r#"
<!-- include: shared.defs.xml -->
<!-- include: battle.script.xml -->
<script name="main">
  <var name="hp" type="int">10</var>
  <text>main.local.before=${hp}</text>
  <text>main.global.before=${shared.hp}</text>
  <call script="battle.battle"/>
  <text>main.local.after=${hp}</text>
  <text>main.global.after=${shared.hp}</text>
</script>
"#,
            ),
        ]);
        let mut engine = engine_from_sources(files);
        engine.start("main.main", None).expect("start main");

        let mut texts = Vec::new();
        for _ in 0..64usize {
            let output = engine.next_output().expect("next should pass");
            if let EngineOutput::Text { text, .. } = &output {
                texts.push(text.clone());
            }
            if matches!(output, EngineOutput::End) {
                break;
            }
        }

        assert_eq!(
            texts,
            vec![
                "main.local.before=10".to_string(),
                "main.global.before=100".to_string(),
                "battle.local=35".to_string(),
                "battle.global=60".to_string(),
                "main.local.after=10".to_string(),
                "main.global.after=60".to_string(),
            ]
        );
    }

    #[test]
    pub(super) fn all_output_types_in_test_helper_are_covered() {
        // Test to cover all branches in test helper: Choices and Input (lines 426-428, 430-431)
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
<script name="main">
  <debug>dbg</debug>
  <choice text="Pick">
    <option text="A"><text>A</text></option>
  </choice>
  <var name="name" type="string">""</var>
  <input var="name" text="Name"/>
  <text>done</text>
</script>"#,
        )]));
        engine.start("main", None).expect("start");

        let mut hit_choices = false;
        let mut hit_input = false;

        for _ in 0..10 {
            let output = engine.next_output().expect("next should succeed");
            match output {
                EngineOutput::Text { text, .. } => {
                    println!("Text: {}", text);
                }
                EngineOutput::Debug { text } => {
                    println!("Debug: {}", text);
                }
                EngineOutput::Choices { items, .. } => {
                    println!("Choices: {} items", items.len());
                    hit_choices = true;
                    engine.choose(0).expect("choose");
                }
                EngineOutput::Input { .. } => {
                    println!("Input");
                    hit_input = true;
                    engine.submit_input("test").expect("input");
                }
                EngineOutput::End => break,
            }
        }

        // Verify we hit both Choices and Input branches
        assert!(hit_choices, "should have hit Choices branch");
        assert!(hit_input, "should have hit Input branch");
    }

    #[test]
    pub(super) fn if_else_branch_covered_when_condition_false() {
        // Test else branch when condition evaluates to false
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <var name="hp" type="int">1</var>
      <if when="hp > 2">
        <text>strong</text>
        <else>
          <text>weak</text>
        </else>
      </if>
    </script>
    "#,
        )]));
        engine.start("main", None).expect("start");

        let output = engine.next_output().expect("next should pass");
        assert!(matches!(output, EngineOutput::Text { text, .. } if text == "weak"));
    }

    #[test]
    pub(super) fn while_loop_condition_false_covered() {
        // Test while loop when condition is initially false
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <var name="hp" type="int">0</var>
      <while when="hp > 0">
        <code>hp = hp - 1;</code>
      </while>
      <text>done</text>
    </script>
    "#,
        )]));
        engine.start("main", None).expect("start");

        // Should skip while loop and go directly to "done"
        let output = engine.next_output().expect("next should pass");
        assert!(matches!(output, EngineOutput::Text { text, .. } if text == "done"));
    }

    #[test]
    pub(super) fn choice_with_no_visible_options_covered() {
        // Test choice when all options have once=True and have been used
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <choice text="Pick">
        <option text="A" once="true"><text>A</text></option>
        <option text="B" once="true"><text>B</text></option>
      </choice>
      <text>end</text>
    </script>
    "#,
        )]));
        engine.start("main", None).expect("start");

        // First time: show choice
        let first = engine.next_output().expect("next should pass");
        assert_eq!(output_kind(&first), "choices");

        // Choose option A
        engine.choose(0).expect("choose should pass");

        // After choice, should output A then move to end
        let output = engine.next_output().expect("next should pass");
        assert!(matches!(output, EngineOutput::Text { text, .. } if text == "A"));

        // Now go back to choice - both options have once=True and were used, should skip
        let output = engine.next_output().expect("next should pass");
        assert!(matches!(output, EngineOutput::Text { text, .. } if text == "end"));
    }

    #[test]
    pub(super) fn choice_visibility_when_expr_error_is_not_swallowed() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <choice text="Pick">
        <option text="A" when="missing > 0"><text>A</text></option>
      </choice>
    </script>
    "#,
        )]));
        engine.start("main", None).expect("start");

        let error = engine
            .next_output()
            .expect_err("invalid when expression should fail");
        assert_eq!(error.code, "ENGINE_EVAL_ERROR");
    }

    #[test]
    pub(super) fn choice_fall_over_invisible_path_is_covered() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <code>let x = 1;</code>
      <choice text="Pick">
        <option text="A" when="false"><text>A</text></option>
        <option text="F" fall_over="true" once="true"><text>F</text></option>
      </choice>
      <text>after</text>
    </script>
    "#,
        )]));
        engine.start("main", None).expect("start");

        let script = engine.scripts.get("main.main").expect("main script");
        let root = script
            .groups
            .get(&script.root_group_id)
            .expect("root group");
        assert!(root
            .nodes
            .iter()
            .any(|node| matches!(node, ScriptNode::Choice { .. })));
        let option_id = root
            .nodes
            .iter()
            .find_map(|node| match node {
                ScriptNode::Choice { entries, .. } => {
                    entries.iter().find_map(|entry| match entry {
                        ChoiceEntry::Static { option } if option.fall_over => {
                            Some(option.id.clone())
                        }
                        _ => None,
                    })
                }
                _ => None,
            })
            .expect("fall_over option");
        engine.mark_once_state("main.main", &format!("option:{}", option_id));

        let output = engine.next_output().expect("choice should be skipped");
        assert!(matches!(output, EngineOutput::Text { text, .. } if text == "after"));
    }

    #[test]
    pub(super) fn guard_and_choice_error_paths_are_covered() {
        let mut infinite = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <while when="true">
        <continue/>
      </while>
    </script>
    "#,
        )]));
        infinite.start("main", None).expect("start");
        let error = infinite.next_output().expect_err("guard should exceed");
        assert_eq!(error.code, "ENGINE_GUARD_EXCEEDED");

        let mut skip_choice = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <choice text="Pick">
        <option text="A" when="false"><text>A</text></option>
      </choice>
      <text>after</text>
    </script>
    "#,
        )]));
        skip_choice.start("main", None).expect("start");
        let output = skip_choice.next_output().expect("next");
        assert!(matches!(output, EngineOutput::Text { text, .. } if text == "after"));

        let mut choice_node_missing = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <choice text="Pick">
        <option text="A"><text>A</text></option>
      </choice>
      <text>tail</text>
    </script>
    "#,
        )]));
        choice_node_missing.start("main", None).expect("start");
        let _ = choice_node_missing.next_output().expect("choice boundary");
        let frame = choice_node_missing
            .frames
            .last_mut()
            .expect("engine should have frame");
        frame.node_index += 1;
        let error = choice_node_missing
            .choose(0)
            .expect_err("pending choice node mismatch should fail");
        assert_eq!(error.code, "ENGINE_CHOICE_NODE_MISSING");

        let mut option_missing = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <choice text="Pick">
        <option text="A"><text>A</text></option>
      </choice>
    </script>
    "#,
        )]));
        option_missing.start("main", None).expect("start");
        let _ = option_missing.next_output().expect("choice boundary");
        let pending = option_missing
            .pending_boundary
            .as_mut()
            .expect("pending choice should exist");
        let options =
            pending_choice_options_mut(pending).expect("pending boundary should be choice");
        options[0].item.id = "missing-option".to_string();
        let error = option_missing
            .choose(0)
            .expect_err("missing option should fail");
        assert_eq!(error.code, "ENGINE_CHOICE_NOT_FOUND");
    }

    #[test]
    pub(super) fn runtime_remaining_branch_paths_are_covered() {
        let mut if_without_else = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><if when="false"><text>x</text></if><text>done</text></script>"#,
        )]));
        if_without_else.start("main", None).expect("start");
        let output = if_without_else.next_output().expect("next");
        assert!(matches!(output, EngineOutput::Text { text, .. } if text == "done"));

        let mut with_choice = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><choice text="Pick"><option text="A"><text>A</text></option></choice></script>"#,
        )]));
        with_choice.start("main", None).expect("start");
        let _ = with_choice.next_output().expect("choice");
        let frame_id = with_choice.frames.last().expect("frame").frame_id;
        with_choice.frames.insert(
            0,
            RuntimeFrame {
                frame_id: 999,
                group_id: with_choice.frames[0].group_id.clone(),
                node_index: 0,
                scope: BTreeMap::from([("target".to_string(), SlValue::Number(0.0))]),
                completion: CompletionKind::None,
                script_root: true,
                return_continuation: None,
                var_types: BTreeMap::from([(
                    "target".to_string(),
                    ScriptType::Primitive {
                        name: "int".to_string(),
                    },
                )]),
            },
        );
        with_choice.pending_boundary = Some(PendingBoundary::Choice {
            frame_id,
            node_id: "node".to_string(),
            options: vec![PendingChoiceOption {
                item: ChoiceItem {
                    index: 0,
                    id: "id0".to_string(),
                    text: "A".to_string(),
                },
                dynamic_binding: None,
            }],
            prompt_text: None,
        });
        let frame = with_choice
            .frames
            .last_mut()
            .expect("engine should have frame");
        frame.scope.insert("id0".to_string(), SlValue::Number(9.0));
        with_choice
            .finish_frame(frame_id)
            .expect("finish should write ref and update continuation");

        let mut no_frame = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>x</text></script>"#,
        )]));
        let decl = sl_core::VarDeclaration {
            name: "x".to_string(),
            r#type: ScriptType::Primitive {
                name: "int".to_string(),
            },
            initial_value_expr: None,
            location: sl_core::SourceSpan::synthetic(),
        };
        no_frame.frames.clear();
        let error = no_frame
            .execute_var_declaration(&decl)
            .expect_err("execute var without frame should fail");
        assert_eq!(error.code, "ENGINE_VAR_FRAME");

        let mut return_engine = engine_from_sources(map(&[
            (
                "main.script.xml",
                r#"<script name="main"><text>main</text></script>"#,
            ),
            (
                "next.script.xml",
                r#"<script name="next"><text>next</text></script>"#,
            ),
        ]));
        let main_root = return_engine
            .scripts
            .get("main")
            .expect("main")
            .root_group_id
            .clone();
        return_engine.frames = vec![
            RuntimeFrame {
                frame_id: 1,
                group_id: main_root.clone(),
                node_index: 0,
                scope: BTreeMap::from([("caller".to_string(), SlValue::Number(1.0))]),
                completion: CompletionKind::None,
                script_root: true,
                return_continuation: None,
                var_types: BTreeMap::new(),
            },
            RuntimeFrame {
                frame_id: 2,
                group_id: main_root.clone(),
                node_index: 0,
                scope: BTreeMap::new(),
                completion: CompletionKind::None,
                script_root: true,
                return_continuation: Some(ContinuationFrame {
                    resume_frame_id: 1,
                    next_node_index: 2,
                    ref_bindings: BTreeMap::from([("x".to_string(), "caller".to_string())]),
                }),
                var_types: BTreeMap::new(),
            },
        ];
        return_engine
            .execute_return(Some("next.next".to_string()), &[])
            .expect("return should pass even when value missing");
        return_engine.frames = vec![
            RuntimeFrame {
                frame_id: 1,
                group_id: main_root.clone(),
                node_index: 0,
                scope: BTreeMap::from([("caller".to_string(), SlValue::Number(1.0))]),
                completion: CompletionKind::None,
                script_root: true,
                return_continuation: None,
                var_types: BTreeMap::new(),
            },
            RuntimeFrame {
                frame_id: 2,
                group_id: main_root,
                node_index: 0,
                scope: BTreeMap::new(),
                completion: CompletionKind::None,
                script_root: true,
                return_continuation: Some(ContinuationFrame {
                    resume_frame_id: 1,
                    next_node_index: 2,
                    ref_bindings: BTreeMap::from([("x".to_string(), "caller".to_string())]),
                }),
                var_types: BTreeMap::new(),
            },
        ];
        return_engine
            .execute_return(None, &[])
            .expect("return should pass even when value missing");

        let mut while_control = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>x</text></script>"#,
        )]));
        while_control.start("main", None).expect("start");
        let error = while_control
            .execute_break()
            .expect_err("break without while should fail");
        assert_eq!(error.code, "ENGINE_WHILE_CONTROL_TARGET_MISSING");
        while_control.frames = vec![RuntimeFrame {
            frame_id: 1,
            group_id: while_control
                .group_lookup
                .keys()
                .next()
                .expect("group")
                .to_string(),
            node_index: 0,
            scope: BTreeMap::new(),
            completion: CompletionKind::WhileBody,
            script_root: false,
            return_continuation: None,
            var_types: BTreeMap::new(),
        }];
        let error = while_control
            .execute_break()
            .expect_err("break without owner should fail");
        assert_eq!(error.code, "ENGINE_WHILE_CONTROL_TARGET_MISSING");
        while_control.frames = vec![RuntimeFrame {
            frame_id: 1,
            group_id: while_control
                .group_lookup
                .keys()
                .next()
                .expect("group")
                .to_string(),
            node_index: 0,
            scope: BTreeMap::new(),
            completion: CompletionKind::None,
            script_root: false,
            return_continuation: None,
            var_types: BTreeMap::new(),
        }];
        let error = while_control
            .execute_continue_while()
            .expect_err("continue without while should fail");
        assert_eq!(error.code, "ENGINE_WHILE_CONTROL_TARGET_MISSING");
        assert!(while_control
            .find_nearest_while_body_frame_index()
            .is_none());

        let mut choice_ctx = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <choice text="Pick">
        <option text="A"><continue/></option>
      </choice>
      <text>done</text>
    </script>
    "#,
        )]));
        choice_ctx.start("main", None).expect("start");
        let _ = choice_ctx.next_output().expect("choice");
        choice_ctx.choose(0).expect("choose");
        let found = choice_ctx
            .find_choice_continue_context()
            .expect("context lookup");
        assert!(found.is_some());
        assert_eq!(choice_ctx.find_frame_index(9999), None);

        let mut expr_engine = engine_from_sources_with_global_json(
            map(&[(
                "main.script.xml",
                r#"
    <script name="main">
      <var name="x" type="int">1</var>
      <text>${x + game.score}</text>
    </script>
    "#,
            )]),
            BTreeMap::from([(
                "game".to_string(),
                SlValue::Map(BTreeMap::from([(
                    "score".to_string(),
                    SlValue::Number(5.0),
                )])),
            )]),
            &["game"],
        );
        expr_engine.start("main", None).expect("start");
        let output = expr_engine.next_output().expect("next");
        assert!(matches!(output, EngineOutput::Text { text, .. } if text == "6"));
        let global = expr_engine
            .global_json
            .get("game")
            .expect("global present")
            .clone();
        assert!(expr_engine.global_json.contains_key("game"));
        expr_engine
            .write_variable("x", SlValue::Number(2.0))
            .expect("write variable should pass");
        let read_back = expr_engine.read_path("x").expect("read path");
        assert_eq!(read_back, SlValue::Number(2.0));
        expr_engine
            .write_path("x", SlValue::Number(3.0))
            .expect("write path should pass");
        assert!(slvalue_to_text(&global).contains("score"));

        let mut snapshot_engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <choice text="Pick">
        <option text="A"><text>A</text></option>
      </choice>
    </script>
    "#,
        )]));
        snapshot_engine.start("main", None).expect("start");
        let _ = snapshot_engine.next_output().expect("choice");
        snapshot_engine.frames.push(RuntimeFrame {
            frame_id: 99,
            group_id: snapshot_engine.frames[0].group_id.clone(),
            node_index: 0,
            scope: BTreeMap::new(),
            completion: CompletionKind::ResumeAfterChild,
            script_root: false,
            return_continuation: None,
            var_types: BTreeMap::new(),
        });
        let _ = snapshot_engine.snapshot().expect("snapshot should pass");
    }

    #[test]
    pub(super) fn runtime_last_missing_lines_are_covered() {
        let mut finisher = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>x</text></script>"#,
        )]));
        let group_id = finisher
            .group_lookup
            .keys()
            .next()
            .expect("group")
            .to_string();
        finisher.frames = vec![
            RuntimeFrame {
                frame_id: 1,
                group_id: group_id.clone(),
                node_index: 0,
                scope: BTreeMap::from([("dst".to_string(), SlValue::Number(0.0))]),
                completion: CompletionKind::None,
                script_root: true,
                return_continuation: None,
                var_types: BTreeMap::new(),
            },
            RuntimeFrame {
                frame_id: 2,
                group_id,
                node_index: 0,
                scope: BTreeMap::from([("src".to_string(), SlValue::Number(9.0))]),
                completion: CompletionKind::None,
                script_root: true,
                return_continuation: Some(ContinuationFrame {
                    resume_frame_id: 1,
                    next_node_index: 5,
                    ref_bindings: BTreeMap::from([("src".to_string(), "dst".to_string())]),
                }),
                var_types: BTreeMap::new(),
            },
        ];
        finisher
            .finish_frame(2)
            .expect("finish should update caller");
        assert_eq!(
            finisher.frames[0].scope.get("dst"),
            Some(&SlValue::Number(9.0))
        );
        assert_eq!(finisher.frames[0].node_index, 5);

        let mut globals = engine_from_sources_with_global_json(
            map(&[(
                "main.script.xml",
                r##"
    <script name="main">
      <var name="obj" type="#{int}"/>
      <code>obj.n = game.score + 1;</code>
      <text>${obj.n}</text>
    </script>
    "##,
            )]),
            BTreeMap::from([(
                "game".to_string(),
                SlValue::Map(BTreeMap::from([(
                    "score".to_string(),
                    SlValue::Number(5.0),
                )])),
            )]),
            &["game"],
        );
        globals.start("main", None).expect("start");
        let global = globals
            .read_variable("game")
            .expect("global should be readable");
        assert_eq!(global.type_name(), "map");
        let text = globals.next_output().expect("next");
        assert!(matches!(text, EngineOutput::Text { text, .. } if text == "6"));
        globals
            .write_variable(
                "obj",
                SlValue::Map(BTreeMap::from([("n".to_string(), SlValue::Number(7.0))])),
            )
            .expect("typed write should pass");
        globals
            .write_path("obj.n", SlValue::Number(8.0))
            .expect("nested write should pass");
        assert_eq!(
            globals.read_path("obj.n").expect("nested read"),
            SlValue::Number(8.0)
        );

        let mut return_skip = engine_from_sources(map(&[
            (
                "main.script.xml",
                r#"<script name="main"><text>x</text></script>"#,
            ),
            (
                "next.script.xml",
                r#"<script name="next"><text>n</text></script>"#,
            ),
        ]));
        let main_group = return_skip
            .scripts
            .get("main")
            .expect("main")
            .root_group_id
            .clone();
        return_skip.frames = vec![
            RuntimeFrame {
                frame_id: 10,
                group_id: main_group.clone(),
                node_index: 0,
                scope: BTreeMap::from([("dst".to_string(), SlValue::Number(1.0))]),
                completion: CompletionKind::None,
                script_root: true,
                return_continuation: None,
                var_types: BTreeMap::new(),
            },
            RuntimeFrame {
                frame_id: 11,
                group_id: main_group,
                node_index: 0,
                scope: BTreeMap::new(),
                completion: CompletionKind::None,
                script_root: true,
                return_continuation: Some(ContinuationFrame {
                    resume_frame_id: 10,
                    next_node_index: 2,
                    ref_bindings: BTreeMap::from([("missing".to_string(), "dst".to_string())]),
                }),
                var_types: BTreeMap::new(),
            },
        ];
        return_skip
            .execute_return(Some("next.next".to_string()), &[])
            .expect("return should pass when source value is missing");
        return_skip.frames = vec![RuntimeFrame {
            frame_id: 12,
            group_id: return_skip
                .scripts
                .get("main")
                .expect("main script")
                .root_group_id
                .clone(),
            node_index: 0,
            scope: BTreeMap::new(),
            completion: CompletionKind::None,
            script_root: true,
            return_continuation: Some(ContinuationFrame {
                resume_frame_id: 999_999,
                next_node_index: 1,
                ref_bindings: BTreeMap::from([("missing".to_string(), "dst".to_string())]),
            }),
            var_types: BTreeMap::new(),
        }];
        return_skip
            .execute_return(Some("next.next".to_string()), &[])
            .expect("return should pass when resume frame is missing");

        let mut find_ctx = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <choice text="Pick">
        <option text="A"><continue/></option>
      </choice>
    </script>
    "#,
        )]));
        find_ctx.start("main", None).expect("start");
        let _ = find_ctx.next_output().expect("choice");
        find_ctx.choose(0).expect("choose");
        let found = find_ctx
            .find_choice_continue_context()
            .expect("choice context");
        assert!(found.is_some());
        let frame = find_ctx
            .frames
            .first_mut()
            .expect("engine should have frame");
        frame.node_index = 1;
        find_ctx.frames.truncate(1);
        let missing = find_ctx
            .find_choice_continue_context()
            .expect("choice context lookup should still pass");
        assert!(missing.is_none());
    }

    #[test]
    pub(super) fn choice_option_when_expr_false_hides_option() {
        // Test that when_expr returning false makes option invisible (covers once_state.rs line 10)
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <choice text="Pick">
        <option text="A" when="false"><text>A</text></option>
        <option text="B"><text>B</text></option>
      </choice>
    </script>
    "#,
        )]));
        engine.start("main", None).expect("start");
        let output = engine.next_output().expect("next should pass");
        // Option A with when="false" should be hidden, only B should be visible
        assert!(
            matches!(output, EngineOutput::Choices { ref items, .. } if items.len() == 1 && items[0].text == "B")
        );
    }

    #[test]
    pub(super) fn while_break_continue_execution_path_covered() {
        // Test normal execution of break and continue in while loop
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <var name="i" type="int">0</var>
      <while when="i  LT  5">
        <code>i = i + 1;</code>
        <if when="i == 2">
          <continue/>
        </if>
        <if when="i == 4">
          <break/>
        </if>
        <text>tick-${i}</text>
      </while>
      <text>done-${i}</text>
    </script>
    "#,
        )]));
        engine.start("main", None).expect("start");

        let mut outputs = Vec::new();
        for _ in 0..10 {
            let output = engine.next_output().expect("next");
            if let EngineOutput::Text { text, .. } = &output {
                outputs.push(text.clone());
            }
            if matches!(output, EngineOutput::End) {
                break;
            }
        }

        // Should see tick-1, tick-3, then done-4
        // i=1: output tick-1
        // i=2: continue (skip)
        // i=3: output tick-3
        // i=4: break
        assert!(outputs.iter().any(|s| s.contains("tick-1")));
        assert!(outputs.iter().any(|s| s.contains("tick-3")));
        assert!(outputs.iter().any(|s| s.contains("done-4")));
    }

    #[test]
    pub(super) fn once_text_skipped_on_revisit() {
        // Test that once text is skipped when revisited (covers step.rs lines 169-170)
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <var name="i" type="int">0</var>
      <while when="i  LT  2">
        <code>i = i + 1;</code>
        <text once="true">only once</text>
        <text>every time</text>
      </while>
    </script>
    "#,
        )]));
        engine.start("main", None).expect("start");

        // First iteration: "only once" and "every time"
        let out1 = engine.next_output().expect("first");
        assert!(matches!(out1, EngineOutput::Text { text, .. } if text == "only once"));

        let out2 = engine.next_output().expect("second");
        assert!(matches!(out2, EngineOutput::Text { text, .. } if text == "every time"));

        // Second iteration: should skip "only once", only "every time"
        let out3 = engine.next_output().expect("third");
        assert!(matches!(out3, EngineOutput::Text { text, .. } if text == "every time"));

        let out4 = engine.next_output().expect("fourth");
        assert_eq!(output_kind(&out4), "end");
    }

    #[test]
    pub(super) fn choice_continue_executes_successfully() {
        // Test choice continue execution path (covers control_flow.rs lines 59-61)
        // Note: continue in choice re-shows the choice, not advance to next node
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <choice text="Pick">
        <option text="A"><continue/></option>
        <option text="B"><text>B</text></option>
      </choice>
      <text>after</text>
    </script>
    "#,
        )]));
        engine.start("main", None).expect("start");

        // Get choice
        let out = engine.next_output().expect("choice");
        assert_eq!(output_kind(&out), "choices");

        // Choose option A with continue - this should re-show the choice
        engine.choose(0).expect("choose");

        // Continue in choice re-shows the choice (not advancing to "after")
        let out2 = engine.next_output().expect("after choice");
        // It should show choice again (since continue loops back)
        assert_eq!(output_kind(&out2), "choices");

        // Now choose B to exit the choice
        engine.choose(1).expect("choose B");
        let out3 = engine.next_output().expect("B text");
        assert!(matches!(out3, EngineOutput::Text { text, .. } if text == "B"));
    }

    #[test]
    pub(super) fn choice_with_fallover_visible_when_regular_hidden() {
        // Test choice where all regular options are hidden but fallover IS visible (covers line 234)
        // Use while loop to revisit the choice multiple times
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <var name="count" type="int">0</var>
      <while when="count  LT  3">
        <code>count = count + 1;</code>
        <choice text="Pick">
          <option text="A" once="true"><text>A</text></option>
          <option text="B" once="true"><text>B</text></option>
          <option text="C" fall_over="true"><text>C</text></option>
        </choice>
      </while>
    </script>
    "#,
        )]));
        engine.start("main", None).expect("start");

        // Iteration 1: A and B visible
        let out1 = engine.next_output().expect("1");
        assert_eq!(output_kind(&out1), "choices");
        engine.choose(0).expect("choose A");
        let _ = engine.next_output();

        // Iteration 2: A hidden (once), B visible, still no fallover
        let out2 = engine.next_output().expect("2");
        assert_eq!(output_kind(&out2), "choices");
        engine.choose(0).expect("choose B");
        let _ = engine.next_output();

        // Iteration 3: A and B hidden (both once), C as fallover visible -> line 234 covered
        let out3 = engine.next_output().expect("3");
        assert!(
            matches!(&out3, EngineOutput::Choices { items, .. } if items.len() == 1 && items[0].text == "C")
        );
    }

    #[test]
    pub(super) fn choice_option_when_expr_true_continues() {
        // Test that when_expr returning true continues to check once state (covers once_state.rs line 10)
        // when_expr = true, once = false -> option should be visible
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <choice text="Pick">
        <option text="A" when="true" once="false"><text>A</text></option>
      </choice>
    </script>
    "#,
        )]));
        engine.start("main", None).expect("start");
        let output = engine.next_output().expect("next should pass");
        // Option A with when="true" and once="false" should be visible
        assert!(
            matches!(output, EngineOutput::Choices { ref items, .. } if items.len() == 1 && items[0].text == "A")
        );
    }

    #[test]
    pub(super) fn dynamic_options_mix_with_static_options_in_source_order() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <var name="arr" type="int[]">[2, 3]</var>
      <choice text="Pick">
        <option text="Static"><text>S</text></option>
        <dynamic-options array="arr" item="it" index="i">
          <option text="D-${it}-${i}" when="it > 2"><text>D ${it}/${i}</text></option>
        </dynamic-options>
        <option text="Tail"><text>T</text></option>
      </choice>
    </script>
    "#,
        )]));
        engine.start("main", None).expect("start");

        let output = engine.next_output().expect("choice output");
        assert!(matches!(
            &output,
            EngineOutput::Choices { items, .. }
                if items.len() == 3
                    && items[0].text == "Static"
                    && items[1].text == "D-3-1"
                    && items[2].text == "Tail"
        ));
        engine.choose(1).expect("choose dynamic option");
        let text = engine.next_output().expect("dynamic text");
        assert!(matches!(text, EngineOutput::Text { text, .. } if text == "D 3/1"));
    }

    #[test]
    pub(super) fn dynamic_options_array_expression_must_evaluate_to_array() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <var name="arr" type="int">1</var>
      <choice text="Pick">
        <dynamic-options array="arr" item="it">
          <option text="${it}"><text>X</text></option>
        </dynamic-options>
      </choice>
    </script>
    "#,
        )]));
        engine.start("main", None).expect("start");
        let error = engine
            .next_output()
            .expect_err("non-array dynamic source should fail");
        assert_eq!(error.code, "ENGINE_CHOICE_ARRAY_NOT_ARRAY");
    }

    #[test]
    pub(super) fn dynamic_options_without_index_still_render_and_choose() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <var name="arr" type="int[]">[7]</var>
      <choice text="Pick">
        <dynamic-options array="arr" item="it">
          <option text="${it}" when="it > 0">
            <text>picked ${it}</text>
          </option>
        </dynamic-options>
      </choice>
    </script>
    "#,
        )]));
        engine.start("main", None).expect("start");
        let out = engine.next_output().expect("choice");
        assert!(
            matches!(&out, EngineOutput::Choices { items, .. } if items.len() == 1 && items[0].text == "7")
        );
        engine.choose(0).expect("choose");
        let text = engine.next_output().expect("text");
        assert!(matches!(text, EngineOutput::Text { text, .. } if text == "picked 7"));
    }

    #[test]
    pub(super) fn nested_dynamic_options_allow_shadowing_and_restore_outer_bindings() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <var name="arr1" type="int[]">[1]</var>
      <var name="arr2" type="int[]">[10]</var>
      <choice text="Outer">
        <dynamic-options array="arr1" item="it" index="i">
          <option text="${it}-${i}">
            <choice text="Inner">
              <dynamic-options array="arr2" item="it" index="i">
                <option text="${it}-${i}">
                  <text>inner ${it}-${i}</text>
                </option>
              </dynamic-options>
            </choice>
            <text>outer ${it}-${i}</text>
          </option>
        </dynamic-options>
      </choice>
    </script>
    "#,
        )]));
        engine.start("main", None).expect("start");

        let outer = engine.next_output().expect("outer choice");
        assert!(
            matches!(&outer, EngineOutput::Choices { items, .. } if items.len() == 1 && items[0].text == "1-0")
        );
        engine.choose(0).expect("choose outer");

        let inner = engine.next_output().expect("inner choice");
        assert!(
            matches!(&inner, EngineOutput::Choices { items, .. } if items.len() == 1 && items[0].text == "10-0")
        );
        engine.choose(0).expect("choose inner");

        let inner_text = engine.next_output().expect("inner text");
        assert!(matches!(inner_text, EngineOutput::Text { text, .. } if text == "inner 10-0"));
        let outer_text = engine.next_output().expect("outer text");
        assert!(matches!(outer_text, EngineOutput::Text { text, .. } if text == "outer 1-0"));
    }

    #[test]
    pub(super) fn next_output_covers_if_while_break_and_continue_paths() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <var name="n" type="int">3</var>
      <while when="n > 0">
        <if when="n == 3">
          <code>n = n - 1;</code>
          <continue/>
        </if>
        <if when="n == 2">
          <code>n = n - 1;</code>
          <break/>
        </if>
      </while>
      <text>done</text>
    </script>
    "#,
        )]));
        engine.start("main", None).expect("start");
        let out = engine.next_output().expect("next");
        assert!(matches!(out, EngineOutput::Text { ref text, .. } if text == "done"));
    }

    #[test]
    pub(super) fn next_output_covers_choice_dynamic_fall_over_and_input_type_error() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <var name="arr" type="int[]">[1, 2]</var>
      <choice text="Pick ${arr[0]}">
        <option text="Hidden" when="false"><text>hidden</text></option>
        <dynamic-options array="arr" item="it" index="i">
          <option text="D${it}" when="i == 1"><text>dynamic ${it}</text></option>
        </dynamic-options>
      </choice>
      <choice text="Fallback">
        <option text="Nope" when="false"><text>nope</text></option>
        <option text="F" fall_over="true"><text>fall</text></option>
      </choice>
      <var name="x" type="int">1</var>
      <input var="x" text="input"/>
    </script>
    "#,
        )]));
        engine.start("main", None).expect("start");

        let first = engine.next_output().expect("choice 1");
        assert_eq!(output_kind(&first), "choices");
        let items = take_choice_items(first).expect("expected first choice");
        assert_eq!(items.len(), 1);
        assert!(items[0].id.starts_with("dyn:"));
        engine.choose(0).expect("choose dynamic");
        let dynamic_text = engine.next_output().expect("dynamic text");
        assert!(matches!(dynamic_text, EngineOutput::Text { ref text, .. } if text == "dynamic 2"));

        let second = engine.next_output().expect("choice 2");
        assert_eq!(output_kind(&second), "choices");
        let items = take_choice_items(second).expect("expected second choice");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].text, "F");
        engine.choose(0).expect("choose fall_over");
        let fall = engine.next_output().expect("fall text");
        assert!(matches!(fall, EngineOutput::Text { ref text, .. } if text == "fall"));

        let error = engine
            .next_output()
            .expect_err("input target must be string");
        assert_eq!(error.code, "ENGINE_INPUT_VAR_TYPE");
        assert!(take_choice_items(EngineOutput::End).is_none());
    }

    #[test]
    pub(super) fn output_kind_includes_input_variant() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <var name="heroName" type="string">"Traveler"</var>
      <input var="heroName" text="Name"/>
    </script>
    "#,
        )]));
        engine.start("main", None).expect("start");
        let output = engine.next_output().expect("input");
        assert_eq!(output_kind(&output), "input");
        let mut pending = PendingBoundary::Input {
            frame_id: 1,
            node_id: "n".to_string(),
            target_var: "name".to_string(),
            prompt_text: "p".to_string(),
            default_text: "d".to_string(),
        };
        assert!(pending_choice_options_mut(&mut pending).is_none());
    }

    #[test]
    pub(super) fn next_output_error_branches_are_covered() {
        let mut missing_group = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>x</text></script>"#,
        )]));
        missing_group.frames = vec![RuntimeFrame {
            frame_id: 1,
            group_id: "missing-group".to_string(),
            node_index: 0,
            scope: BTreeMap::new(),
            completion: CompletionKind::None,
            script_root: true,
            return_continuation: None,
            var_types: BTreeMap::new(),
        }];
        let error = missing_group
            .next_output()
            .expect_err("missing group should fail during planning");
        assert_eq!(error.code, "ENGINE_GROUP_NOT_FOUND");

        let mut finish_error = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>x</text></script>"#,
        )]));
        let group_id = finish_error
            .group_lookup
            .keys()
            .next()
            .expect("group key")
            .to_string();
        finish_error.frames = vec![
            RuntimeFrame {
                frame_id: 10,
                group_id: group_id.clone(),
                node_index: 0,
                scope: BTreeMap::new(),
                completion: CompletionKind::None,
                script_root: true,
                return_continuation: None,
                var_types: BTreeMap::new(),
            },
            RuntimeFrame {
                frame_id: 11,
                group_id,
                node_index: usize::MAX,
                scope: BTreeMap::new(),
                completion: CompletionKind::None,
                script_root: true,
                return_continuation: Some(ContinuationFrame {
                    resume_frame_id: 10,
                    next_node_index: 1,
                    ref_bindings: BTreeMap::from([("src".to_string(), "dst".to_string())]),
                }),
                var_types: BTreeMap::new(),
            },
        ];
        let error = finish_error
            .next_output()
            .expect_err("finish frame branch should surface ref value missing");
        assert_eq!(error.code, "ENGINE_REF_VALUE_MISSING");

        let mut while_non_bool = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><while when="1"><text>x</text></while></script>"#,
        )]));
        while_non_bool.start("main", None).expect("start");
        let error = while_non_bool
            .next_output()
            .expect_err("while condition must be bool");
        assert_eq!(error.code, "ENGINE_BOOLEAN_EXPECTED");

        let mut fallover_text_error = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><choice text="Pick"><option text="${bad +}" fall_over="true"><text>F</text></option></choice></script>"#,
        )]));
        fallover_text_error.start("main", None).expect("start");
        let error = fallover_text_error
            .next_output()
            .expect_err("fallover text render error should bubble");
        assert_eq!(error.code, "ENGINE_EVAL_ERROR");

        let mut regular_text_error = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><choice text="Pick"><option text="${bad +}"><text>A</text></option></choice></script>"#,
        )]));
        regular_text_error.start("main", None).expect("start");
        let error = regular_text_error
            .next_output()
            .expect_err("regular option text render error should bubble");
        assert_eq!(error.code, "ENGINE_EVAL_ERROR");

        let mut dynamic_array_eval_error = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><choice text="Pick"><dynamic-options array="bad +" item="it"><option text="${it}"><text>x</text></option></dynamic-options></choice></script>"#,
        )]));
        dynamic_array_eval_error.start("main", None).expect("start");
        let error = dynamic_array_eval_error
            .next_output()
            .expect_err("dynamic array expression eval error should bubble");
        assert_eq!(error.code, "ENGINE_EVAL_ERROR");

        let mut dynamic_when_error = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><var name="arr" type="int[]">[1]</var><choice text="Pick"><dynamic-options array="arr" item="it"><option text="${it}" when="bad +"><text>x</text></option></dynamic-options></choice></script>"#,
        )]));
        dynamic_when_error.start("main", None).expect("start");
        let error = dynamic_when_error
            .next_output()
            .expect_err("dynamic when eval error should bubble");
        assert_eq!(error.code, "ENGINE_EVAL_ERROR");

        let mut dynamic_text_error = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><var name="arr" type="int[]">[1]</var><choice text="Pick"><dynamic-options array="arr" item="it"><option text="${bad +}"><text>x</text></option></dynamic-options></choice></script>"#,
        )]));
        dynamic_text_error.start("main", None).expect("start");
        let error = dynamic_text_error
            .next_output()
            .expect_err("dynamic text render error should bubble");
        assert_eq!(error.code, "ENGINE_EVAL_ERROR");

        let mut prompt_error = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><choice text="${bad +}"><option text="A"><text>A</text></option></choice></script>"#,
        )]));
        prompt_error.start("main", None).expect("start");
        let error = prompt_error
            .next_output()
            .expect_err("choice prompt render error should bubble");
        assert_eq!(error.code, "ENGINE_EVAL_ERROR");

        let mut input_read_error = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><input var="missing" text="input"/></script>"#,
        )]));
        input_read_error.start("main", None).expect("start");
        let error = input_read_error
            .next_output()
            .expect_err("input target read should fail");
        assert_eq!(error.code, "ENGINE_VAR_READ");

        let mut break_error = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><while when="true"><break/></while></script>"#,
        )]));
        let break_group_id = break_error
            .scripts
            .values()
            .flat_map(|script| script.groups.values())
            .find_map(|group| {
                group
                    .nodes
                    .iter()
                    .any(|node| matches!(node, ScriptNode::Break { .. }))
                    .then(|| group.group_id.clone())
            })
            .expect("break group");
        break_error.frames = vec![RuntimeFrame {
            frame_id: 1,
            group_id: break_group_id,
            node_index: 0,
            scope: BTreeMap::new(),
            completion: CompletionKind::None,
            script_root: true,
            return_continuation: None,
            var_types: BTreeMap::new(),
        }];
        let error = break_error
            .next_output()
            .expect_err("break without while body context should fail");
        assert_eq!(error.code, "ENGINE_WHILE_CONTROL_TARGET_MISSING");

        let mut continue_while_error = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><while when="true"><continue/></while></script>"#,
        )]));
        let continue_while_group_id = continue_while_error
            .scripts
            .values()
            .flat_map(|script| script.groups.values())
            .find_map(|group| {
                group
                    .nodes
                    .iter()
                    .any(|node| {
                        matches!(
                            node,
                            ScriptNode::Continue {
                                target: ContinueTarget::While,
                                ..
                            }
                        )
                    })
                    .then(|| group.group_id.clone())
            })
            .expect("continue while group");
        continue_while_error.frames = vec![RuntimeFrame {
            frame_id: 1,
            group_id: continue_while_group_id,
            node_index: 0,
            scope: BTreeMap::new(),
            completion: CompletionKind::None,
            script_root: true,
            return_continuation: None,
            var_types: BTreeMap::new(),
        }];
        let error = continue_while_error
            .next_output()
            .expect_err("continue while without context should fail");
        assert_eq!(error.code, "ENGINE_WHILE_CONTROL_TARGET_MISSING");

        let mut continue_choice_error = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><choice text="Pick"><option text="A"><continue/></option></choice></script>"#,
        )]));
        let continue_choice_group_id = continue_choice_error
            .scripts
            .values()
            .flat_map(|script| script.groups.values())
            .find_map(|group| {
                group
                    .nodes
                    .iter()
                    .any(|node| {
                        matches!(
                            node,
                            ScriptNode::Continue {
                                target: ContinueTarget::Choice,
                                ..
                            }
                        )
                    })
                    .then(|| group.group_id.clone())
            })
            .expect("continue choice group");
        continue_choice_error.frames = vec![RuntimeFrame {
            frame_id: 1,
            group_id: continue_choice_group_id,
            node_index: 0,
            scope: BTreeMap::new(),
            completion: CompletionKind::None,
            script_root: true,
            return_continuation: None,
            var_types: BTreeMap::new(),
        }];
        let error = continue_choice_error
            .next_output()
            .expect_err("continue choice without context should fail");
        assert_eq!(error.code, "ENGINE_CHOICE_CONTINUE_TARGET_MISSING");
    }
}
