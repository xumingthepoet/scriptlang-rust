impl ScriptLangEngine {
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
                    once: bool,
                    id: String,
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
                    options: Vec<sl_core::ChoiceOption>,
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
                            value, once, id, ..
                        } => PlannedNode::Text {
                            script_name: script_name.to_string(),
                            value: value.clone(),
                            once: *once,
                            id: id.clone(),
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
                            options,
                            prompt_text,
                            ..
                        } => PlannedNode::Choice {
                            script_name: script_name.to_string(),
                            id: id.clone(),
                            options: options.clone(),
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
                    once,
                    id,
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
                PlannedNode::Code { code } => {
                    self.run_code(&code)?;
                    self.bump_top_node_index(1)?;
                }
                PlannedNode::Var { declaration } => {
                    self.execute_var_declaration(&declaration)?;
                    self.bump_top_node_index(1)?;
                }
                PlannedNode::If {
                    when_expr,
                    then_group_id,
                    else_group_id,
                } => {
                    let condition = self.eval_boolean(&when_expr)?;
                    self.bump_top_node_index(1)?;
                    if condition {
                        self.push_group_frame(&then_group_id, CompletionKind::ResumeAfterChild)?;
                    } else {
                        let else_group_id = else_group_id
                            .expect("compiler should always synthesize an else group id");
                        self.push_group_frame(&else_group_id, CompletionKind::ResumeAfterChild)?;
                    }
                }
                PlannedNode::While {
                    when_expr,
                    body_group_id,
                } => {
                    let condition = self.eval_boolean(&when_expr)?;
                    if condition {
                        self.push_group_frame(&body_group_id, CompletionKind::WhileBody)?;
                    } else {
                        self.bump_top_node_index(1)?;
                    }
                }
                PlannedNode::Choice {
                    script_name,
                    id,
                    options,
                    prompt_text,
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

}

#[cfg(test)]
mod step_tests {
    use super::*;
    use super::runtime_test_support::*;
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn next_text_and_end() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>Hello</text></script>"#,
        )]));
        engine.start("main", None).expect("start");
    
        let first = engine.next_output().expect("next");
        assert!(matches!(first, EngineOutput::Text { .. }));
    
        let second = engine.next_output().expect("next");
        assert!(matches!(second, EngineOutput::End));
    }

    #[test]
    fn drives_all_examples_to_end() {
        let examples_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("examples")
            .join("scripts-rhai");
        let mut dirs = fs::read_dir(&examples_root)
            .expect("examples should exist")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect::<Vec<_>>();
        dirs.sort();
    
        for dir in dirs {
            let mut files = BTreeMap::new();
            read_sources_recursive(&dir, &dir, &mut files).expect("load sources");
            let mut engine = engine_from_sources(files);
            engine.start("main", None).expect("start main");
            drive_engine_to_end(&mut engine);
        }
    }
    
    #[test]
    fn if_else_branch_covered_when_condition_false() {
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
    fn while_loop_condition_false_covered() {
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
    fn choice_with_no_visible_options_covered() {
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
        assert!(matches!(first, EngineOutput::Choices { .. }));
    
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
    fn guard_and_choice_error_paths_are_covered() {
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
        if let Some(frame) = choice_node_missing.frames.last_mut() {
            frame.node_index += 1;
        }
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
        if let PendingBoundary::Choice { options, .. } = pending {
            options[0].id = "missing-option".to_string();
        }
        let error = option_missing
            .choose(0)
            .expect_err("missing option should fail");
        assert_eq!(error.code, "ENGINE_CHOICE_NOT_FOUND");
    }

    #[test]
    fn runtime_remaining_branch_paths_are_covered() {
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
            options: vec![ChoiceItem {
                index: 0,
                id: "id0".to_string(),
                text: "A".to_string(),
            }],
            prompt_text: None,
        });
        if let Some(frame) = with_choice.frames.last_mut() {
            frame.scope.insert("id0".to_string(), SlValue::Number(9.0));
        }
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
            .execute_return(Some("next".to_string()), &[])
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
    
        let mut expr_engine = engine_from_sources(map(&[
            ("game.json", r#"{ "score": 5 }"#),
            (
                "main.script.xml",
                r#"
    <!-- include: game.json -->
    <script name="main">
      <var name="x" type="int">1</var>
      <text>${x + game.score}</text>
    </script>
    "#,
            ),
        ]));
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
    fn runtime_last_missing_lines_are_covered() {
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
    
        let mut globals = engine_from_sources(map(&[
            ("game.json", r#"{ "score": 5 }"#),
            (
                "main.script.xml",
                r##"
    <!-- include: game.json -->
    <script name="main">
      <var name="obj" type="#{int}"/>
      <code>obj.n = game.score + 1;</code>
      <text>${obj.n}</text>
    </script>
    "##,
            ),
        ]));
        globals.start("main", None).expect("start");
        let global = globals
            .read_variable("game")
            .expect("global should be readable");
        assert!(matches!(global, SlValue::Map(_)));
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
            .execute_return(Some("next".to_string()), &[])
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
            .execute_return(Some("next".to_string()), &[])
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
        if let Some(frame) = find_ctx.frames.first_mut() {
            frame.node_index = 1;
        }
        find_ctx.frames.truncate(1);
        let missing = find_ctx
            .find_choice_continue_context()
            .expect("choice context lookup should still pass");
        assert!(missing.is_none());
    }

}
