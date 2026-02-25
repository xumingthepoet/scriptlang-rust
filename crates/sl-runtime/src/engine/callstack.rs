impl ScriptLangEngine {
    fn execute_var_declaration(
        &mut self,
        decl: &sl_core::VarDeclaration,
    ) -> Result<(), ScriptLangError> {
        let frame_index = self.frames.len().checked_sub(1).ok_or_else(|| {
            ScriptLangError::new(
                "ENGINE_VAR_FRAME",
                "No frame available for var declaration.",
            )
        })?;

        let duplicate = self.frames[frame_index].scope.contains_key(&decl.name);
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

        let frame = &mut self.frames[frame_index];
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
                    format!(
                        "Call argument at position {} has no matching parameter.",
                        index + 1
                    ),
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
        let mut resolved_return_target: Option<(String, ScriptIr)> = None;

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
                        format!(
                            "Return argument at position {} has no target parameter.",
                            index + 1
                        ),
                    ));
                };
                transfer_arg_values
                    .insert(param.name.clone(), self.eval_expression(&arg.value_expr)?);
            }

            resolved_return_target = Some((target_name.clone(), target));
        }

        self.frames.truncate(root_index);

        if let Some((target_name, target)) = resolved_return_target {
            let mut forwarded = inherited.clone();
            if let Some(continuation) = inherited {
                if self
                    .find_frame_index(continuation.resume_frame_id)
                    .is_some()
                {
                    for (caller_path, value) in continuation.ref_bindings.into_iter().filter_map(
                        |(callee_var, caller_path)| {
                            root_frame
                                .scope
                                .get(&callee_var)
                                .cloned()
                                .map(|value| (caller_path, value))
                        },
                    ) {
                        self.write_path(&caller_path, value)?;
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

}
