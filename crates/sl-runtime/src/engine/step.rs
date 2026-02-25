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
                    } else {
                        let else_group_id = else_group_id
                            .expect("compiler should always synthesize an else group id");
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

}
