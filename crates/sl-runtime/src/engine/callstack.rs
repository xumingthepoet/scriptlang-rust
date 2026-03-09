use super::lifecycle::{CompletionKind, RuntimeFrame};
use super::*;

impl ScriptLangEngine {
    fn resolve_current_module_name(&self) -> Option<String> {
        self.resolve_current_script_name()
            .and_then(|current_script_name| self.scripts.get(&current_script_name).cloned())
            .and_then(|script| script.module_name)
    }

    fn validate_script_access_from_current(
        &self,
        target_script_name: &str,
        target: &ScriptIr,
    ) -> Result<(), ScriptLangError> {
        if target.access != AccessLevel::Private {
            return Ok(());
        }
        let Some(target_module_name) = target.module_name.as_deref() else {
            return Err(ScriptLangError::new(
                "ENGINE_SCRIPT_ACCESS_DENIED",
                format!(
                    "Script \"{}\" is private and cannot be called from current context.",
                    target_script_name
                ),
            ));
        };
        let Some(current_module_name) = self.resolve_current_module_name() else {
            return Err(ScriptLangError::new(
                "ENGINE_SCRIPT_ACCESS_DENIED",
                format!(
                    "Script \"{}\" is private and cannot be called from current context.",
                    target_script_name
                ),
            ));
        };
        if current_module_name != target_module_name {
            return Err(ScriptLangError::new(
                "ENGINE_SCRIPT_ACCESS_DENIED",
                format!(
                    "Script \"{}\" is private and cannot be called from current context.",
                    target_script_name
                ),
            ));
        }
        Ok(())
    }

    fn qualify_target_script(
        &self,
        resolved: &str,
        missing_code: &str,
        missing_message: &str,
    ) -> Result<String, ScriptLangError> {
        let mut target_script = resolved.trim().to_string();
        if target_script.is_empty() {
            return Err(ScriptLangError::new(missing_code, missing_message));
        }
        if !target_script.contains('.') {
            if let Some(module_name) = self.resolve_current_module_name() {
                target_script = format!("{}.{}", module_name, target_script);
            }
        }
        Ok(target_script)
    }

    fn resolve_target_script(
        &mut self,
        target: &ScriptTarget,
        missing_code: &str,
        missing_message: &str,
    ) -> Result<String, ScriptLangError> {
        match target {
            ScriptTarget::Literal { script_name } => {
                self.qualify_target_script(script_name, missing_code, missing_message)
            }
            ScriptTarget::Variable { var_name } => {
                let value = self.read_variable(var_name)?;
                let SlValue::String(raw) = value else {
                    return Err(ScriptLangError::new(
                        "ENGINE_TARGET_VAR_TYPE",
                        format!("Target variable \"{}\" must be script.", var_name),
                    ));
                };
                let stripped = raw.trim().strip_prefix('@').ok_or_else(|| {
                    ScriptLangError::new(
                        "ENGINE_TARGET_VAR_TYPE",
                        format!("Target variable \"{}\" must be script.", var_name),
                    )
                })?;
                self.qualify_target_script(stripped, missing_code, missing_message)
            }
        }
    }

    pub(super) fn execute_var_declaration(
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
            value = self.eval_initializer_expression(expr, "initializer")?;
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

    pub(super) fn execute_call(
        &mut self,
        target_script: &ScriptTarget,
        args: &[sl_core::CallArgument],
    ) -> Result<(), ScriptLangError> {
        let target_script = self.resolve_target_script(
            target_script,
            "ENGINE_CALL_TARGET_EMPTY",
            "Call target script cannot resolve to empty.",
        )?;
        let caller_index = self.frames.len().checked_sub(1).ok_or_else(|| {
            ScriptLangError::new("ENGINE_CALL_NO_FRAME", "No frame available for <call>.")
        })?;

        let caller_group_id = self.frames[caller_index].group_id.clone();
        let caller_group_len = {
            let (_, caller_group) = self.lookup_group(&caller_group_id)?;
            caller_group.nodes.len()
        };

        let Some(target) = self.scripts.get(&target_script).cloned() else {
            return Err(ScriptLangError::new(
                "ENGINE_CALL_TARGET",
                format!("Call target script \"{}\" not found.", target_script),
            ));
        };
        self.validate_script_access_from_current(&target_script, &target)?;

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
            && caller.node_index == caller_group_len.saturating_sub(1)
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
            let (scope, var_types) = self.create_script_root_scope(&target_script, arg_values)?;
            self.push_root_frame(&target.root_group_id, scope, inherited, var_types);
            return Ok(());
        }

        let continuation = ContinuationFrame {
            resume_frame_id: caller.frame_id,
            next_node_index: caller.node_index + 1,
            ref_bindings,
        };

        let (scope, var_types) = self.create_script_root_scope(&target_script, arg_values)?;
        self.push_root_frame(&target.root_group_id, scope, Some(continuation), var_types);
        Ok(())
    }

    pub(super) fn execute_return(
        &mut self,
        target_script: Option<ScriptTarget>,
        args: &[sl_core::CallArgument],
    ) -> Result<(), ScriptLangError> {
        let root_index = self.find_current_root_frame_index()?;
        let root_frame = self.frames[root_index].clone();
        let inherited = root_frame.return_continuation.clone();

        let mut transfer_arg_values = BTreeMap::new();
        let mut resolved_return_target: Option<(String, ScriptIr)> = None;

        if let Some(target_name) = target_script.as_ref() {
            let target_name = self.resolve_target_script(
                target_name,
                "ENGINE_RETURN_TARGET_EMPTY",
                "Return target script cannot resolve to empty.",
            )?;
            let Some(target) = self.scripts.get(&target_name).cloned() else {
                return Err(ScriptLangError::new(
                    "ENGINE_RETURN_TARGET",
                    format!("Return target script \"{}\" not found.", target_name),
                ));
            };
            self.validate_script_access_from_current(&target_name, &target)?;

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

            resolved_return_target = Some((target_name, target));
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

                let mut continuation = forwarded
                    .take()
                    .expect("forwarded continuation should exist when inherited is present");
                continuation.ref_bindings = BTreeMap::new();
                forwarded = Some(continuation);
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

    pub(super) fn find_current_root_frame_index(&self) -> Result<usize, ScriptLangError> {
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

#[cfg(test)]
mod callstack_tests {
    use super::runtime_test_support::*;
    use super::*;

    fn lit(name: &str) -> ScriptTarget {
        ScriptTarget::Literal {
            script_name: name.to_string(),
        }
    }

    fn var(name: &str) -> ScriptTarget {
        ScriptTarget::Variable {
            var_name: name.to_string(),
        }
    }

    #[test]
    pub(super) fn nested_script_calls_covered() {
        // Test nested script calls
        let mut engine = engine_from_sources(map(&[
            (
                "main.script.xml",
                r#"
    <script name="main">
      <call script="@greeting.greeting"/>
    </script>
    "#,
            ),
            (
                "greeting.script.xml",
                r#"<script name="greeting"><text>Hi</text></script>"#,
            ),
        ]));
        engine.start("main.main", None).expect("start");

        let output = engine.next_output().expect("next should pass");
        assert!(matches!(output, EngineOutput::Text { text, .. } if text == "Hi"));

        let mut dynamic_engine = engine_from_sources(map(&[
            (
                "main.script.xml",
                r#"
    <!-- import greeting from greeting.xml -->
    <script name="main">
      <temp name="nextScene" type="script">@greeting.greeting</temp>
      <call script="nextScene"/>
    </script>
    "#,
            ),
            (
                "greeting.script.xml",
                r#"<script name="greeting"><text>Dynamic hi</text></script>"#,
            ),
        ]));
        dynamic_engine.start("main.main", None).expect("start");

        let output = dynamic_engine
            .next_output()
            .expect("dynamic next should pass");
        assert!(matches!(output, EngineOutput::Text { text, .. } if text == "Dynamic hi"));
    }

    #[test]
    pub(super) fn runtime_errors_cover_call_argument_and_return_target_paths() {
        let mut call_missing_target = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>x</text></script>"#,
        )]));
        call_missing_target.start("main", None).expect("start");
        let group_id = call_missing_target
            .scripts
            .get("main")
            .expect("main script")
            .root_group_id
            .clone();
        call_missing_target.frames = vec![RuntimeFrame {
            frame_id: 1,
            group_id,
            node_index: 0,
            scope: BTreeMap::new(),
            completion: CompletionKind::None,
            script_root: true,
            return_continuation: None,
            var_types: BTreeMap::new(),
        }];
        let error = call_missing_target
            .execute_call(&lit("missing"), &[])
            .expect_err("missing call target should fail");
        assert_eq!(error.code, "ENGINE_CALL_TARGET");

        let mut call_empty_target = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>x</text></script>"#,
        )]));
        call_empty_target.start("main", None).expect("start");
        let group_id = call_empty_target
            .scripts
            .get("main")
            .expect("main script")
            .root_group_id
            .clone();
        call_empty_target.frames = vec![RuntimeFrame {
            frame_id: 1,
            group_id,
            node_index: 0,
            scope: BTreeMap::from([("dst".to_string(), SlValue::String("@".to_string()))]),
            completion: CompletionKind::None,
            script_root: true,
            return_continuation: None,
            var_types: BTreeMap::from([("dst".to_string(), ScriptType::Script)]),
        }];
        let error = call_empty_target
            .execute_call(&var("dst"), &[])
            .expect_err("empty call target should fail");
        assert_eq!(error.code, "ENGINE_CALL_TARGET_EMPTY");

        let mut call_bad_type = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>x</text></script>"#,
        )]));
        call_bad_type.start("main", None).expect("start");
        let group_id = call_bad_type
            .scripts
            .get("main")
            .expect("main script")
            .root_group_id
            .clone();
        call_bad_type.frames = vec![RuntimeFrame {
            frame_id: 1,
            group_id,
            node_index: 0,
            scope: BTreeMap::from([("dst".to_string(), SlValue::String("bad".to_string()))]),
            completion: CompletionKind::None,
            script_root: true,
            return_continuation: None,
            var_types: BTreeMap::from([("dst".to_string(), ScriptType::Script)]),
        }];
        let error = call_bad_type
            .execute_call(&var("dst"), &[])
            .expect_err("non-script target value should fail");
        assert_eq!(error.code, "ENGINE_TARGET_VAR_TYPE");

        let mut call_non_string_target = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>x</text></script>"#,
        )]));
        call_non_string_target.start("main", None).expect("start");
        let group_id = call_non_string_target
            .scripts
            .get("main")
            .expect("main script")
            .root_group_id
            .clone();
        call_non_string_target.frames = vec![RuntimeFrame {
            frame_id: 1,
            group_id,
            node_index: 0,
            scope: BTreeMap::from([("dst".to_string(), SlValue::Number(1.0))]),
            completion: CompletionKind::None,
            script_root: true,
            return_continuation: None,
            var_types: BTreeMap::from([("dst".to_string(), ScriptType::Script)]),
        }];
        let error = call_non_string_target
            .execute_call(&var("dst"), &[])
            .expect_err("non-string target value should fail");
        assert_eq!(error.code, "ENGINE_TARGET_VAR_TYPE");

        let mut call_missing_var_target = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>x</text></script>"#,
        )]));
        call_missing_var_target.start("main", None).expect("start");
        let group_id = call_missing_var_target
            .scripts
            .get("main")
            .expect("main script")
            .root_group_id
            .clone();
        call_missing_var_target.frames = vec![RuntimeFrame {
            frame_id: 1,
            group_id,
            node_index: 0,
            scope: BTreeMap::new(),
            completion: CompletionKind::None,
            script_root: true,
            return_continuation: None,
            var_types: BTreeMap::new(),
        }];
        let error = call_missing_var_target
            .execute_call(&var("dst"), &[])
            .expect_err("missing target variable should fail");
        assert_eq!(error.code, "ENGINE_VAR_READ");

        let mut call_arg_mismatch = engine_from_sources(map(&[
            (
                "main.script.xml",
                r#"
    <!-- import callee from callee.xml -->
    <script name="main">
      <temp name="hp" type="int">1</temp>
      <call script="@callee.callee" args="hp"/>
    </script>
    "#,
            ),
            (
                "callee.script.xml",
                r#"<script name="callee" args="ref:int:x"><return/></script>"#,
            ),
        ]));
        call_arg_mismatch.start("main.main", None).expect("start");
        let error = call_arg_mismatch
            .next_output()
            .expect_err("ref mismatch should fail");
        assert_eq!(error.code, "ENGINE_CALL_REF_MISMATCH");

        let mut return_target_missing = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>x</text></script>"#,
        )]));
        return_target_missing.start("main", None).expect("start");
        let group_id = return_target_missing
            .scripts
            .get("main")
            .expect("main script")
            .root_group_id
            .clone();
        return_target_missing.frames = vec![RuntimeFrame {
            frame_id: 1,
            group_id,
            node_index: 0,
            scope: BTreeMap::new(),
            completion: CompletionKind::None,
            script_root: true,
            return_continuation: None,
            var_types: BTreeMap::new(),
        }];
        let error = return_target_missing
            .execute_return(Some(lit("missing")), &[])
            .expect_err("missing return target should fail");
        assert_eq!(error.code, "ENGINE_RETURN_TARGET");

        let mut return_empty_target = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>x</text></script>"#,
        )]));
        return_empty_target.start("main", None).expect("start");
        let group_id = return_empty_target
            .scripts
            .get("main")
            .expect("main script")
            .root_group_id
            .clone();
        return_empty_target.frames = vec![RuntimeFrame {
            frame_id: 1,
            group_id,
            node_index: 0,
            scope: BTreeMap::from([("dst".to_string(), SlValue::String("@".to_string()))]),
            completion: CompletionKind::None,
            script_root: true,
            return_continuation: None,
            var_types: BTreeMap::from([("dst".to_string(), ScriptType::Script)]),
        }];
        let error = return_empty_target
            .execute_return(Some(var("dst")), &[])
            .expect_err("empty return target should fail");
        assert_eq!(error.code, "ENGINE_RETURN_TARGET_EMPTY");

        let mut return_bad_type = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>x</text></script>"#,
        )]));
        return_bad_type.start("main", None).expect("start");
        let group_id = return_bad_type
            .scripts
            .get("main")
            .expect("main script")
            .root_group_id
            .clone();
        return_bad_type.frames = vec![RuntimeFrame {
            frame_id: 1,
            group_id,
            node_index: 0,
            scope: BTreeMap::from([("dst".to_string(), SlValue::String("bad".to_string()))]),
            completion: CompletionKind::None,
            script_root: true,
            return_continuation: None,
            var_types: BTreeMap::from([("dst".to_string(), ScriptType::Script)]),
        }];
        let error = return_bad_type
            .execute_return(Some(var("dst")), &[])
            .expect_err("non-script return target value should fail");
        assert_eq!(error.code, "ENGINE_TARGET_VAR_TYPE");
    }

    #[test]
    pub(super) fn finish_frame_and_return_paths_are_covered() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>Hello</text></script>"#,
        )]));
        let group_id = engine
            .group_lookup
            .keys()
            .next()
            .expect("group key")
            .to_string();
        let number_ty = ScriptType::Primitive {
            name: "int".to_string(),
        };

        engine.frames = vec![RuntimeFrame {
            frame_id: 1,
            group_id: group_id.clone(),
            node_index: 0,
            scope: BTreeMap::new(),
            completion: CompletionKind::None,
            script_root: true,
            return_continuation: Some(ContinuationFrame {
                resume_frame_id: 99,
                next_node_index: 1,
                ref_bindings: BTreeMap::new(),
            }),
            var_types: BTreeMap::new(),
        }];
        engine.finish_frame(1).expect("finish should pass");
        assert!(engine.ended);

        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>Hello</text></script>"#,
        )]));
        let group_id = engine
            .group_lookup
            .keys()
            .next()
            .expect("group key")
            .to_string();
        engine.frames = vec![
            RuntimeFrame {
                frame_id: 2,
                group_id: group_id.clone(),
                node_index: 0,
                scope: BTreeMap::from([("target".to_string(), SlValue::Number(0.0))]),
                completion: CompletionKind::None,
                script_root: true,
                return_continuation: None,
                var_types: BTreeMap::from([("target".to_string(), number_ty.clone())]),
            },
            RuntimeFrame {
                frame_id: 1,
                group_id: group_id.clone(),
                node_index: 0,
                scope: BTreeMap::new(),
                completion: CompletionKind::None,
                script_root: true,
                return_continuation: Some(ContinuationFrame {
                    resume_frame_id: 2,
                    next_node_index: 3,
                    ref_bindings: BTreeMap::from([("missing".to_string(), "target".to_string())]),
                }),
                var_types: BTreeMap::new(),
            },
        ];
        let error = engine
            .finish_frame(1)
            .expect_err("missing ref value should fail");
        assert_eq!(error.code, "ENGINE_REF_VALUE_MISSING");

        let mut engine = engine_from_sources(map(&[
            (
                "main.script.xml",
                r#"<script name="main"><text>main</text></script>"#,
            ),
            (
                "next.script.xml",
                r#"<script name="next"><text>next</text></script>"#,
            ),
        ]));
        let main_root = engine
            .scripts
            .get("main")
            .expect("main script")
            .root_group_id
            .clone();
        let next_root = engine
            .scripts
            .get("next")
            .expect("next script")
            .root_group_id
            .clone();
        engine.frames = vec![
            RuntimeFrame {
                frame_id: 10,
                group_id: main_root.clone(),
                node_index: 0,
                scope: BTreeMap::from([("caller".to_string(), SlValue::Number(0.0))]),
                completion: CompletionKind::None,
                script_root: true,
                return_continuation: None,
                var_types: BTreeMap::from([("caller".to_string(), number_ty.clone())]),
            },
            RuntimeFrame {
                frame_id: 11,
                group_id: main_root.clone(),
                node_index: 0,
                scope: BTreeMap::from([("x".to_string(), SlValue::Number(7.0))]),
                completion: CompletionKind::None,
                script_root: true,
                return_continuation: Some(ContinuationFrame {
                    resume_frame_id: 10,
                    next_node_index: 4,
                    ref_bindings: BTreeMap::from([("x".to_string(), "caller".to_string())]),
                }),
                var_types: BTreeMap::from([("x".to_string(), number_ty.clone())]),
            },
        ];
        engine
            .execute_return(Some(lit("next.next")), &[])
            .expect("return to next should pass");
        assert_eq!(engine.frames.len(), 2);
        assert_eq!(
            engine.frames[0].scope.get("caller"),
            Some(&SlValue::Number(7.0))
        );
        assert_eq!(engine.frames[1].group_id, next_root);

        engine.frames = vec![RuntimeFrame {
            frame_id: 1,
            group_id: main_root.clone(),
            node_index: 0,
            scope: BTreeMap::new(),
            completion: CompletionKind::None,
            script_root: true,
            return_continuation: None,
            var_types: BTreeMap::new(),
        }];
        engine
            .execute_return(None, &[])
            .expect("return without continuation should pass");
        assert!(engine.ended);

        engine.ended = false;
        engine.frames = vec![RuntimeFrame {
            frame_id: 1,
            group_id: main_root.clone(),
            node_index: 0,
            scope: BTreeMap::new(),
            completion: CompletionKind::None,
            script_root: true,
            return_continuation: Some(ContinuationFrame {
                resume_frame_id: 999,
                next_node_index: 1,
                ref_bindings: BTreeMap::new(),
            }),
            var_types: BTreeMap::new(),
        }];
        engine
            .execute_return(None, &[])
            .expect("missing resume frame should end execution");
        assert!(engine.ended);

        engine.ended = false;
        engine.frames = vec![
            RuntimeFrame {
                frame_id: 20,
                group_id: main_root.clone(),
                node_index: 0,
                scope: BTreeMap::from([("caller".to_string(), SlValue::Number(1.0))]),
                completion: CompletionKind::None,
                script_root: true,
                return_continuation: None,
                var_types: BTreeMap::from([("caller".to_string(), number_ty.clone())]),
            },
            RuntimeFrame {
                frame_id: 21,
                group_id: main_root,
                node_index: 0,
                scope: BTreeMap::from([("x".to_string(), SlValue::Number(3.0))]),
                completion: CompletionKind::None,
                script_root: true,
                return_continuation: Some(ContinuationFrame {
                    resume_frame_id: 20,
                    next_node_index: 6,
                    ref_bindings: BTreeMap::from([("x".to_string(), "caller".to_string())]),
                }),
                var_types: BTreeMap::from([("x".to_string(), number_ty)]),
            },
        ];
        engine
            .execute_return(None, &[])
            .expect("return with continuation should pass");
        assert_eq!(engine.frames.len(), 1);
        assert_eq!(engine.frames[0].node_index, 6);
        assert_eq!(
            engine.frames[0].scope.get("caller"),
            Some(&SlValue::Number(3.0))
        );
    }

    #[test]
    pub(super) fn return_forwarding_and_root_index_success_paths_are_covered() {
        let mut engine = engine_from_sources(map(&[
            (
                "main.script.xml",
                r#"<script name="main"><text>main</text></script>"#,
            ),
            (
                "next.script.xml",
                r#"<script name="next"><text>next</text></script>"#,
            ),
        ]));
        let main_root = engine
            .scripts
            .get("main")
            .expect("main script")
            .root_group_id
            .clone();

        engine.frames = vec![
            RuntimeFrame {
                frame_id: 30,
                group_id: main_root.clone(),
                node_index: 0,
                scope: BTreeMap::from([("caller".to_string(), SlValue::Number(1.0))]),
                completion: CompletionKind::None,
                script_root: true,
                return_continuation: None,
                var_types: BTreeMap::from([(
                    "caller".to_string(),
                    ScriptType::Primitive {
                        name: "int".to_string(),
                    },
                )]),
            },
            RuntimeFrame {
                frame_id: 31,
                group_id: main_root,
                node_index: 0,
                scope: BTreeMap::new(),
                completion: CompletionKind::None,
                script_root: true,
                return_continuation: Some(ContinuationFrame {
                    resume_frame_id: 30,
                    next_node_index: 2,
                    ref_bindings: BTreeMap::from([("x".to_string(), "caller".to_string())]),
                }),
                var_types: BTreeMap::new(),
            },
        ];

        engine
            .execute_return(Some(lit("next.next")), &[])
            .expect("return to target script should pass");

        let continuation = engine
            .frames
            .last()
            .expect("target root frame")
            .return_continuation
            .as_ref()
            .expect("forwarded continuation should exist");
        assert!(continuation.ref_bindings.is_empty());

        let root_index = engine
            .find_current_root_frame_index()
            .expect("root frame index should resolve");
        assert_eq!(root_index, engine.frames.len() - 1);

        let mut no_inherited = engine_from_sources(map(&[
            (
                "main.script.xml",
                r#"<script name="main"><text>main</text></script>"#,
            ),
            (
                "next.script.xml",
                r#"<script name="next"><text>next</text></script>"#,
            ),
        ]));
        let main_root = no_inherited
            .scripts
            .get("main")
            .expect("main script")
            .root_group_id
            .clone();
        let next_root = no_inherited
            .scripts
            .get("next")
            .expect("next script")
            .root_group_id
            .clone();
        no_inherited.frames = vec![RuntimeFrame {
            frame_id: 40,
            group_id: main_root,
            node_index: 0,
            scope: BTreeMap::new(),
            completion: CompletionKind::None,
            script_root: true,
            return_continuation: None,
            var_types: BTreeMap::new(),
        }];
        no_inherited
            .execute_return(Some(lit("next.next")), &[])
            .expect("return target should work without inherited continuation");
        assert_eq!(no_inherited.frames.len(), 1);
        assert_eq!(no_inherited.frames[0].group_id, next_root);
        assert!(no_inherited.frames[0].return_continuation.is_none());

        let mut root_lookup = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>x</text></script>"#,
        )]));
        let root_group = root_lookup
            .scripts
            .get("main")
            .expect("main script")
            .root_group_id
            .clone();
        root_lookup.frames = vec![
            RuntimeFrame {
                frame_id: 50,
                group_id: root_group.clone(),
                node_index: 0,
                scope: BTreeMap::new(),
                completion: CompletionKind::None,
                script_root: true,
                return_continuation: None,
                var_types: BTreeMap::new(),
            },
            RuntimeFrame {
                frame_id: 51,
                group_id: root_group,
                node_index: 0,
                scope: BTreeMap::new(),
                completion: CompletionKind::None,
                script_root: false,
                return_continuation: None,
                var_types: BTreeMap::new(),
            },
        ];
        let root_index = root_lookup
            .find_current_root_frame_index()
            .expect("root should be found after skipping non-root frame");
        assert_eq!(root_index, 0);

        let mut dynamic_return = engine_from_sources(map(&[
            (
                "main.script.xml",
                r#"
    <!-- import next from next.xml -->
    <script name="main">
      <temp name="nextScene" type="script">@next.next</temp>
      <return script="nextScene"/>
    </script>
    "#,
            ),
            (
                "next.script.xml",
                r#"<script name="next"><text>moved</text></script>"#,
            ),
        ]));
        dynamic_return.start("main.main", None).expect("start");
        let output = dynamic_return
            .next_output()
            .expect("dynamic return should pass");
        assert!(matches!(output, EngineOutput::Text { text, .. } if text == "moved"));
    }

    #[test]
    pub(super) fn call_helpers_and_value_path_branches_are_covered() {
        let mut no_frame = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>x</text></script>"#,
        )]));
        no_frame.frames.clear();
        let error = no_frame
            .execute_call(&lit("main"), &[])
            .expect_err("execute_call without frame should fail");
        assert_eq!(error.code, "ENGINE_CALL_NO_FRAME");

        let mut ref_mismatch = engine_from_sources(map(&[
            (
                "main.script.xml",
                r#"
    <!-- import callee from callee.xml -->
    <script name="main">
      <temp name="x" type="int">1</temp>
      <call script="@callee.callee" args="ref:x"/>
    </script>
    "#,
            ),
            (
                "callee.script.xml",
                r#"<script name="callee" args="int:x"><return/></script>"#,
            ),
        ]));
        ref_mismatch.start("main.main", None).expect("start");
        let error = ref_mismatch
            .next_output()
            .expect_err("non-ref param with ref arg should fail");
        assert_eq!(error.code, "ENGINE_CALL_REF_MISMATCH");

        let mut tail = engine_from_sources(map(&[
            (
                "main.script.xml",
                r#"<script name="main"><text>main</text></script>"#,
            ),
            (
                "callee.script.xml",
                r#"<script name="callee" args="ref:int:x"><text>${x}</text></script>"#,
            ),
        ]));
        let main_root = tail
            .scripts
            .get("main")
            .expect("main script")
            .root_group_id
            .clone();
        tail.frames = vec![RuntimeFrame {
            frame_id: 1,
            group_id: main_root.clone(),
            node_index: 0,
            scope: BTreeMap::from([("x".to_string(), SlValue::Number(1.0))]),
            completion: CompletionKind::None,
            script_root: true,
            return_continuation: Some(ContinuationFrame {
                resume_frame_id: 99,
                next_node_index: 1,
                ref_bindings: BTreeMap::new(),
            }),
            var_types: BTreeMap::from([(
                "x".to_string(),
                ScriptType::Primitive {
                    name: "int".to_string(),
                },
            )]),
        }];
        let error = tail
            .execute_call(
                &lit("callee.callee"),
                &[sl_core::CallArgument {
                    value_expr: "x".to_string(),
                    is_ref: true,
                }],
            )
            .expect_err("tail call with ref args should fail");
        assert_eq!(error.code, "ENGINE_TAIL_REF_UNSUPPORTED");

        let mut tail_ok = engine_from_sources(map(&[
            (
                "main.script.xml",
                r#"<script name="main"><text>main</text></script>"#,
            ),
            (
                "callee.script.xml",
                r#"<script name="callee" args="int:x"><text>${x}</text></script>"#,
            ),
        ]));
        tail_ok.frames = vec![RuntimeFrame {
            frame_id: 1,
            group_id: main_root,
            node_index: 0,
            scope: BTreeMap::from([("x".to_string(), SlValue::Number(2.0))]),
            completion: CompletionKind::None,
            script_root: true,
            return_continuation: Some(ContinuationFrame {
                resume_frame_id: 42,
                next_node_index: 1,
                ref_bindings: BTreeMap::new(),
            }),
            var_types: BTreeMap::from([(
                "x".to_string(),
                ScriptType::Primitive {
                    name: "int".to_string(),
                },
            )]),
        }];
        tail_ok
            .execute_call(
                &lit("callee.callee"),
                &[sl_core::CallArgument {
                    value_expr: "x".to_string(),
                    is_ref: false,
                }],
            )
            .expect("tail call optimization path should pass");
        assert_eq!(tail_ok.frames.len(), 1);

        let mut globals = engine_from_sources_with_global_data(
            map(&[(
                "main.script.xml",
                r#"
    <script name="main">
      <temp name="x" type="int">1</temp>
      <code>x = x + game.score;</code>
      <text>${x}</text>
    </script>
    "#,
            )]),
            BTreeMap::from([(
                "game".to_string(),
                SlValue::Map(BTreeMap::from([(
                    "score".to_string(),
                    SlValue::Number(10.0),
                )])),
            )]),
            &["game"],
        );
        globals.start("main", None).expect("start");
        let output = globals.next_output().expect("next");
        assert!(matches!(output, EngineOutput::Text { text, .. } if text == "11"));
        assert!(!globals.is_visible_global_data(None, "game"));
        assert!(!globals.is_visible_global_data(Some("missing"), "game"));
        assert!(globals.is_visible_global_data(Some("main"), "game"));

        let value = globals
            .read_variable("game")
            .expect("visible global data global should be readable");
        assert_eq!(
            value,
            SlValue::Map(BTreeMap::from([(
                "score".to_string(),
                SlValue::Number(10.0)
            )]))
        );
        let error = globals
            .read_variable("missing")
            .expect_err("missing variable should fail");
        assert_eq!(error.code, "ENGINE_VAR_READ");

        let error = globals
            .write_variable("x", SlValue::String("bad".to_string()))
            .expect_err("type mismatch should fail");
        assert_eq!(error.code, "ENGINE_TYPE_MISMATCH");
        let error = globals
            .write_variable("game", SlValue::Number(1.0))
            .expect_err("global should be readonly");
        assert_eq!(error.code, "ENGINE_GLOBAL_READONLY");
        let error = globals
            .write_variable("unknown", SlValue::Number(1.0))
            .expect_err("unknown variable should fail");
        assert_eq!(error.code, "ENGINE_VAR_WRITE");

        let error = globals.read_path(" . ").expect_err("invalid path");
        assert_eq!(error.code, "ENGINE_REF_PATH");
        let error = globals
            .read_path("x.y")
            .expect_err("path read on non-map should fail");
        assert_eq!(error.code, "ENGINE_REF_PATH_READ");
        let error = globals
            .read_path("game.missing")
            .expect_err("missing nested key should fail");
        assert_eq!(error.code, "ENGINE_REF_PATH_READ");

        let error = globals
            .write_path(" . ", SlValue::Number(1.0))
            .expect_err("invalid write path should fail");
        assert_eq!(error.code, "ENGINE_REF_PATH");
        globals
            .write_path("x", SlValue::Number(12.0))
            .expect("single segment write should pass");
        let error = globals
            .write_path("x.y", SlValue::Number(1.0))
            .expect_err("nested write on non-map should fail");
        assert_eq!(error.code, "ENGINE_REF_PATH_WRITE");

        assert!(slvalue_to_text(&SlValue::Array(vec![SlValue::Number(1.0)])).contains("Array"));
        assert_eq!(slvalue_to_rhai_literal(&SlValue::Bool(false)), "false");
        assert_eq!(slvalue_to_rhai_literal(&SlValue::Number(2.5)), "2.5");
        assert_eq!(
            slvalue_to_rhai_literal(&SlValue::Array(vec![SlValue::Number(1.0)])),
            "[1]"
        );

        let mut state = 1u32;
        let bounded = next_random_bounded_with(&mut state, 3, |state| {
            let candidate = if *state == 1 { u32::MAX } else { 7 };
            *state = state.wrapping_add(1);
            candidate
        });
        assert_eq!(bounded, 1);

        let error = globals
            .create_script_root_scope("missing-script", BTreeMap::new())
            .expect_err("missing script should fail");
        assert_eq!(error.code, "ENGINE_SCRIPT_NOT_FOUND");
        assert_eq!(
            globals
                .build_module_prelude("missing-script", &BTreeMap::new())
                .expect("missing script prelude should be empty"),
            ""
        );
        let mut module_engine = engine_from_sources(map(&[
            (
                "main.script.xml",
                r#"
    <!-- import shared from shared.xml -->
    <script name="main"><text>x</text></script>
    "#,
            ),
            (
                "shared.xml",
                r#"<module name="shared" default_access="public"><function name="make" return="int:out">out = 1;</function></module>"#,
            ),
        ]));
        module_engine.invoke_function_symbols.clear();
        let error = module_engine
            .build_module_prelude("main", &BTreeMap::new())
            .expect_err("missing symbol mapping should fail");
        assert_eq!(error.code, "ENGINE_MODULE_FUNCTION_SYMBOL_MISSING");

        let registry = TestRegistry {
            names: vec!["f".to_string()],
        };
        let call_value = registry.call("f", &[]).expect("test registry call");
        assert_eq!(call_value, SlValue::Bool(true));
    }

    #[test]
    pub(super) fn module_function_call_execution_is_covered() {
        // Test actual module function call to cover rhai_bridge.rs rewrite code
        let mut engine = engine_from_sources(map(&[
            (
                "shared.xml",
                r#"<module name="shared" default_access="public">
  <function name="add" args="int:a,int:b" return="int:result">
    result = a + b;
  </function>
</module>"#,
            ),
            (
                "main.script.xml",
                r#"<!-- import shared from shared.xml -->
<script name="main">
  <text>${shared.add(1, 2)}</text>
</script>"#,
            ),
        ]));
        engine.start("main", None).expect("start");
        let output = engine.next_output().expect("next");
        assert!(matches!(output, EngineOutput::Text { text, .. } if text == "3"));
    }

    #[test]
    pub(super) fn callstack_error_branches_on_lookup_and_ref_paths_are_covered() {
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
            .execute_call(&lit("main"), &[])
            .expect_err("caller group lookup should fail");
        assert_eq!(error.code, "ENGINE_GROUP_NOT_FOUND");

        let mut ref_read_error = engine_from_sources(map(&[
            (
                "main.script.xml",
                r#"
    <!-- import callee from callee.xml -->
    <script name="main">
      <call script="@callee.callee" args="ref:missing.path"/>
    </script>
    "#,
            ),
            (
                "callee.script.xml",
                r#"<script name="callee" args="ref:int:x"><return/></script>"#,
            ),
        ]));
        ref_read_error.start("main.main", None).expect("start");
        let error = ref_read_error
            .next_output()
            .expect_err("ref read should fail");
        assert_eq!(error.code, "ENGINE_VAR_READ");

        let mut eval_arg_error = engine_from_sources(map(&[
            (
                "main.script.xml",
                r#"
    <!-- import callee from callee.xml -->
    <script name="main">
      <call script="@callee.callee" args="unknown +"/>
    </script>
    "#,
            ),
            (
                "callee.script.xml",
                r#"<script name="callee" args="int:x"><return/></script>"#,
            ),
        ]));
        eval_arg_error.start("main", None).expect("start");
        let error = eval_arg_error
            .next_output()
            .expect_err("arg eval should fail");
        assert_eq!(error.code, "ENGINE_EVAL_ERROR");

        let mut tail_scope_error = engine_from_sources(map(&[
            (
                "main.script.xml",
                r#"<script name="main"><text>main</text></script>"#,
            ),
            (
                "callee.script.xml",
                r#"<script name="callee" args="int:x"><text>${x}</text></script>"#,
            ),
        ]));
        let main_root = tail_scope_error
            .scripts
            .get("main")
            .expect("main script")
            .root_group_id
            .clone();
        tail_scope_error.frames = vec![RuntimeFrame {
            frame_id: 1,
            group_id: main_root,
            node_index: 0,
            scope: BTreeMap::from([("x".to_string(), SlValue::String("bad".to_string()))]),
            completion: CompletionKind::None,
            script_root: true,
            return_continuation: Some(ContinuationFrame {
                resume_frame_id: 42,
                next_node_index: 1,
                ref_bindings: BTreeMap::new(),
            }),
            var_types: BTreeMap::from([(
                "x".to_string(),
                ScriptType::Primitive {
                    name: "string".to_string(),
                },
            )]),
        }];
        let error = tail_scope_error
            .execute_call(
                &lit("callee.callee"),
                &[sl_core::CallArgument {
                    value_expr: "x".to_string(),
                    is_ref: false,
                }],
            )
            .expect_err("tail call scope creation should fail");
        assert_eq!(error.code, "ENGINE_TYPE_MISMATCH");

        let mut no_root = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><return/></script>"#,
        )]));
        no_root.frames.clear();
        let error = no_root
            .execute_return(None, &[])
            .expect_err("missing root frame should fail");
        assert_eq!(error.code, "ENGINE_ROOT_FRAME");

        let mut return_arg_eval_error = engine_from_sources(map(&[
            (
                "main.script.xml",
                r#"<script name="main"><return script="@next.next" args="bad +"/></script>"#,
            ),
            (
                "next.script.xml",
                r#"<script name="next" args="int:x"><text>${x}</text></script>"#,
            ),
        ]));
        return_arg_eval_error
            .start("main.main", None)
            .expect("start");
        let error = return_arg_eval_error
            .next_output()
            .expect_err("return arg eval should fail");
        assert_eq!(error.code, "ENGINE_EVAL_ERROR");

        let mut return_write_error = engine_from_sources(map(&[
            (
                "main.script.xml",
                r#"<script name="main"><text>main</text></script>"#,
            ),
            (
                "next.script.xml",
                r#"<script name="next"><text>next</text></script>"#,
            ),
        ]));
        let main_root = return_write_error
            .scripts
            .get("main")
            .expect("main script")
            .root_group_id
            .clone();
        return_write_error.frames = vec![
            RuntimeFrame {
                frame_id: 10,
                group_id: main_root.clone(),
                node_index: 0,
                scope: BTreeMap::from([("dst".to_string(), SlValue::Number(0.0))]),
                completion: CompletionKind::None,
                script_root: true,
                return_continuation: None,
                var_types: BTreeMap::from([(
                    "dst".to_string(),
                    ScriptType::Primitive {
                        name: "int".to_string(),
                    },
                )]),
            },
            RuntimeFrame {
                frame_id: 11,
                group_id: main_root,
                node_index: 0,
                scope: BTreeMap::from([("x".to_string(), SlValue::Number(7.0))]),
                completion: CompletionKind::None,
                script_root: true,
                return_continuation: Some(ContinuationFrame {
                    resume_frame_id: 10,
                    next_node_index: 1,
                    ref_bindings: BTreeMap::from([("x".to_string(), "dst.bad".to_string())]),
                }),
                var_types: BTreeMap::from([(
                    "x".to_string(),
                    ScriptType::Primitive {
                        name: "int".to_string(),
                    },
                )]),
            },
        ];
        let error = return_write_error
            .execute_return(None, &[])
            .expect_err("return ref write path should fail");
        assert_eq!(error.code, "ENGINE_REF_PATH_WRITE");

        let mut target_return_write_error = engine_from_sources(map(&[
            (
                "main.script.xml",
                r#"<script name="main"><text>main</text></script>"#,
            ),
            (
                "next.script.xml",
                r#"<script name="next"><text>next</text></script>"#,
            ),
        ]));
        let main_root = target_return_write_error
            .scripts
            .get("main")
            .expect("main script")
            .root_group_id
            .clone();
        target_return_write_error.frames = vec![
            RuntimeFrame {
                frame_id: 30,
                group_id: main_root.clone(),
                node_index: 0,
                scope: BTreeMap::from([("dst".to_string(), SlValue::Number(0.0))]),
                completion: CompletionKind::None,
                script_root: true,
                return_continuation: None,
                var_types: BTreeMap::from([(
                    "dst".to_string(),
                    ScriptType::Primitive {
                        name: "int".to_string(),
                    },
                )]),
            },
            RuntimeFrame {
                frame_id: 31,
                group_id: main_root,
                node_index: 0,
                scope: BTreeMap::from([("x".to_string(), SlValue::Number(5.0))]),
                completion: CompletionKind::None,
                script_root: true,
                return_continuation: Some(ContinuationFrame {
                    resume_frame_id: 30,
                    next_node_index: 1,
                    ref_bindings: BTreeMap::from([("x".to_string(), "dst.bad".to_string())]),
                }),
                var_types: BTreeMap::from([(
                    "x".to_string(),
                    ScriptType::Primitive {
                        name: "int".to_string(),
                    },
                )]),
            },
        ];
        let error = target_return_write_error
            .execute_return(Some(lit("next.next")), &[])
            .expect_err("target return ref write path should fail");
        assert_eq!(error.code, "ENGINE_REF_PATH_WRITE");

        let mut return_target_type_error = engine_from_sources(map(&[
            (
                "main.script.xml",
                r#"<script name="main"><return script="@next.next" args="'bad'"/></script>"#,
            ),
            (
                "next.script.xml",
                r#"<script name="next" args="int:x"><text>${x}</text></script>"#,
            ),
        ]));
        return_target_type_error
            .start("main.main", None)
            .expect("start");
        let error = return_target_type_error
            .next_output()
            .expect_err("return target scope creation should fail");
        assert_eq!(error.code, "ENGINE_TYPE_MISMATCH");
    }

    #[test]
    pub(super) fn ref_int_index_remains_usable_for_array_lookup_after_call() {
        let mut engine = engine_from_sources(map(&[
            (
                "main.script.xml",
                r#"
<!-- import bump from bump.xml -->
<script name="main">
  <temp name="arr" type="int[]">[10, 20, 30]</temp>
  <temp name="idx" type="int">0</temp>
  <call script="@bump.bump" args="ref:idx"/>
  <text>${arr[idx]}</text>
</script>
"#,
            ),
            (
                "bump.script.xml",
                r#"
<script name="bump" args="ref:int:i">
  <code>i += 1;</code>
  <return/>
</script>
"#,
            ),
        ]));

        engine.start("main.main", None).expect("start");
        let output = engine.next_output().expect("next output");
        assert!(matches!(output, EngineOutput::Text { text, .. } if text == "20"));
    }

    #[test]
    pub(super) fn resolve_target_script_qualifies_module_local_names_only_when_available() {
        let mut engine = engine_from_sources(map(&[(
            "battle.module.xml",
            r#"
<module name="battle" default_access="public">
  <script name="main"><temp name="cmd" type="string">""</temp><input var="cmd" text="go"/></script>
  <script name="next"><text>x</text></script>
</module>
"#,
        )]));
        engine.start("battle.main", None).expect("start");

        let qualified = engine
            .resolve_target_script(&lit("next"), "ERR", "err")
            .expect("module local name should qualify");
        assert_eq!(qualified, "battle.next");

        let explicit = engine
            .resolve_target_script(&lit("battle.next"), "ERR", "err")
            .expect("explicit qualified target should stay as is");
        assert_eq!(explicit, "battle.next");

        let missing_local = engine
            .resolve_target_script(&lit("other"), "ERR", "err")
            .expect("unknown local name should qualify to current module");
        assert_eq!(missing_local, "battle.other");

        let mut plain_engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><temp name="cmd" type="string">""</temp><input var="cmd" text="go"/></script>"#,
        )]));
        plain_engine.start("main.main", None).expect("start");
        let plain = plain_engine
            .resolve_target_script(&lit("next"), "ERR", "err")
            .expect("module-local script names should qualify");
        assert_eq!(plain, "main.next");

        let mut idle_engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>x</text></script>"#,
        )]));
        let idle = idle_engine
            .resolve_target_script(&lit("next"), "ERR", "err")
            .expect("target resolution without active frame should still work");
        assert_eq!(idle, "next");

        let mut missing_script_engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>x</text></script>"#,
        )]));
        let root_group_id = missing_script_engine
            .scripts
            .get("main")
            .expect("main script")
            .root_group_id
            .clone();
        missing_script_engine.frames.push(RuntimeFrame {
            frame_id: 1,
            group_id: root_group_id,
            node_index: 0,
            scope: BTreeMap::new(),
            completion: CompletionKind::None,
            script_root: true,
            return_continuation: None,
            var_types: BTreeMap::new(),
        });
        missing_script_engine.scripts.remove("main");
        let missing_script_result = missing_script_engine
            .resolve_target_script(&lit("next"), "ERR", "err")
            .expect("group metadata should still qualify to current module");
        assert_eq!(missing_script_result, "main.next");
    }

    #[test]
    pub(super) fn resolve_target_script_keeps_short_name_for_alias_without_module() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><temp name="cmd" type="string">""</temp><input var="cmd" text="go"/></script>"#,
        )]));
        engine.start("main.main", None).expect("start");
        let group_id = engine.frames.last().expect("frame").group_id.clone();
        engine.group_lookup.insert(
            group_id.clone(),
            super::lifecycle::GroupLookup {
                script_name: "main".to_string(),
                group_id,
            },
        );

        let target = engine
            .resolve_target_script(&lit("next"), "ERR", "err")
            .expect("alias-backed current script should keep short name");
        assert_eq!(target, "next");
    }

    #[test]
    pub(super) fn call_access_control_enforces_private_visibility_rules() {
        let mut same_module = engine_from_sources(map(&[(
            "main.xml",
            r#"<module name="main" default_access="private">
<script name="main" access="public"><call script="@hidden"/></script>
<script name="hidden" access="private"><text>ok</text></script>
</module>"#,
        )]));
        same_module.start("main.main", None).expect("start");
        let output = same_module
            .next_output()
            .expect("private sibling call should pass");
        assert!(matches!(output, EngineOutput::Text { text, .. } if text == "ok"));

        let mut cross_module = engine_from_sources(map(&[
            (
                "shared.xml",
                r#"<module name="shared" default_access="public"><script name="hidden" access="private"><text>hidden</text></script></module>"#,
            ),
            (
                "main.xml",
                r#"
<!-- import shared from shared.xml -->
<module name="main" default_access="public">
<script name="main"><text>x</text></script>
</module>
"#,
            ),
        ]));
        cross_module.start("main.main", None).expect("start");
        let group_id = cross_module
            .scripts
            .get("main.main")
            .expect("main script")
            .root_group_id
            .clone();
        cross_module.frames = vec![RuntimeFrame {
            frame_id: 1,
            group_id,
            node_index: 0,
            scope: BTreeMap::from([(
                "dst".to_string(),
                SlValue::String("@shared.hidden".to_string()),
            )]),
            completion: CompletionKind::None,
            script_root: true,
            return_continuation: None,
            var_types: BTreeMap::from([("dst".to_string(), ScriptType::Script)]),
        }];
        let cross_module_error = cross_module
            .execute_call(&var("dst"), &[])
            .expect_err("cross-module private call should fail");
        assert_eq!(cross_module_error.code, "ENGINE_SCRIPT_ACCESS_DENIED");

        let mut dynamic_cross_module = engine_from_sources(map(&[
            (
                "shared.xml",
                r#"<module name="shared" default_access="public"><script name="hidden" access="private"><text>hidden</text></script></module>"#,
            ),
            (
                "main.xml",
                r#"
<!-- import shared from shared.xml -->
<module name="main" default_access="public">
<script name="main"><text>x</text></script>
</module>
"#,
            ),
        ]));
        dynamic_cross_module
            .start("main.main", None)
            .expect("start");
        let group_id = dynamic_cross_module
            .scripts
            .get("main.main")
            .expect("main script")
            .root_group_id
            .clone();
        dynamic_cross_module.frames = vec![RuntimeFrame {
            frame_id: 1,
            group_id,
            node_index: 0,
            scope: BTreeMap::from([(
                "dst".to_string(),
                SlValue::String("@shared.hidden".to_string()),
            )]),
            completion: CompletionKind::None,
            script_root: true,
            return_continuation: None,
            var_types: BTreeMap::from([("dst".to_string(), ScriptType::Script)]),
        }];
        let dynamic_error = dynamic_cross_module
            .execute_call(&var("dst"), &[])
            .expect_err("dynamic cross-module private call should fail");
        assert_eq!(dynamic_error.code, "ENGINE_SCRIPT_ACCESS_DENIED");
    }

    #[test]
    fn private_script_alias_without_module_name_denies_access() {
        // When a module has exactly one private script with a unique local name,
        // the runtime creates a short alias with module_name = None.
        // This tests that a private script with module_name = None denies access.
        // Note: We use a script without module wrapper as the caller to avoid
        // the automatic module prefixing.
        let mut engine = engine_from_sources(map(&[
            (
                "lib.xml",
                r#"<module name="lib" default_access="public">
  <script name="secret" access="private"><text>hidden</text></script>
</module>"#,
            ),
            (
                "main.xml",
                r#"
<!-- import lib from lib.xml -->
<script name="main"><text>x</text></script>
"#,
            ),
        ]));
        engine.start("main.main", None).expect("start");
        let group_id = engine
            .scripts
            .get("main.main")
            .expect("main script")
            .root_group_id
            .clone();
        engine.frames = vec![RuntimeFrame {
            frame_id: 1,
            group_id,
            node_index: 0,
            scope: BTreeMap::from([(
                "dst".to_string(),
                SlValue::String("@lib.secret".to_string()),
            )]),
            completion: CompletionKind::None,
            script_root: true,
            return_continuation: None,
            var_types: BTreeMap::from([("dst".to_string(), ScriptType::Script)]),
        }];
        // The script lib.secret is private, and calling from non-module context
        // should deny access
        let error = engine
            .execute_call(&var("dst"), &[])
            .expect_err("private script call should deny access");
        assert_eq!(error.code, "ENGINE_SCRIPT_ACCESS_DENIED");
    }

    #[test]
    fn return_to_private_script_without_module_denies_access() {
        // Test line 230: return target validation when target has no module_name
        let mut engine = engine_from_sources(map(&[
            (
                "lib.xml",
                r#"<module name="lib" default_access="public">
  <script name="helper" access="private"><text>help</text></script>
</module>"#,
            ),
            (
                "main.xml",
                r#"
<!-- import lib from lib.xml -->
<script name="main">
  <text>x</text>
</script>
"#,
            ),
        ]));
        engine.start("main.main", None).expect("start");
        let group_id = engine
            .scripts
            .get("main.main")
            .expect("main script")
            .root_group_id
            .clone();
        engine.frames = vec![RuntimeFrame {
            frame_id: 1,
            group_id,
            node_index: 0,
            scope: BTreeMap::from([(
                "dst".to_string(),
                SlValue::String("@lib.helper".to_string()),
            )]),
            completion: CompletionKind::None,
            script_root: true,
            return_continuation: None,
            var_types: BTreeMap::from([("dst".to_string(), ScriptType::Script)]),
        }];
        // Return to private script from non-module context should deny access
        let error = engine
            .execute_return(Some(var("dst")), &[])
            .expect_err("return to private script should deny access");
        assert_eq!(error.code, "ENGINE_SCRIPT_ACCESS_DENIED");
    }

    #[test]
    fn private_script_from_standalone_denies_access_line29() {
        // Line 29: current script in group_lookup but not in scripts
        // This makes resolve_current_module_name() return None
        let mut engine = engine_from_sources(map(&[
            (
                "lib.xml",
                r#"<module name="lib" default_access="public">
  <script name="secret" access="private"><text>hidden</text></script>
</module>"#,
            ),
            (
                "main.xml",
                r#"
<!-- import lib from lib.xml -->
<script name="main"><text>x</text></script>
"#,
            ),
        ]));
        // Start engine - this populates group_lookup
        engine.start("main.main", None).expect("start");

        // Now remove the current script from scripts to simulate corrupted state
        // This makes resolve_current_module_name() return None
        engine.scripts.remove("main.main");

        let target = engine
            .scripts
            .get("lib.secret")
            .expect("target script should exist")
            .clone();
        let error = engine
            .validate_script_access_from_current("lib.secret", &target)
            .expect_err("private script access should deny when current module is unavailable");
        assert_eq!(error.code, "ENGINE_SCRIPT_ACCESS_DENIED");
    }

    #[test]
    fn private_script_alias_without_module_line20() {
        // Line 20: target has module_name = None (alias) and access = Private
        // Test by directly invoking the validation method with constructed inputs
        let mut engine = engine_from_sources(map(&[(
            "lib.xml",
            r#"<module name="lib" default_access="public">
  <script name="secret" access="private"><text>hidden</text></script>
</module>"#,
        )]));

        // The alias "secret" is created with module_name = None
        // We need to call validate_script_access_from_current directly
        // Let's first start the engine to set up frames
        engine
            .start("lib.secret", None)
            .expect_err("private entry should fail");

        // Create a ScriptIr manually with module_name = None and access = Private
        let private_alias = sl_core::ScriptIr {
            script_path: "lib.xml".to_string(),
            script_name: "secret".to_string(),
            access: sl_core::AccessLevel::Private,
            module_name: None, // This is the key - module_name is None
            local_script_name: Some("secret".to_string()),
            params: vec![],
            root_group_id: "g1".to_string(),
            groups: Default::default(),
            visible_functions: Default::default(),
            visible_module_vars: Default::default(),
            visible_module_consts: Default::default(),
            visible_globals: vec![],
            invoke_all_functions: Default::default(),
            invoke_public_functions: Default::default(),
        };

        // Directly call validate - should hit line 20 because module_name is None
        let result = engine.validate_script_access_from_current("secret", &private_alias);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, "ENGINE_SCRIPT_ACCESS_DENIED");
    }

    #[test]
    fn current_script_not_in_scripts_line29() {
        // Line 29: current script is in group_lookup but not in scripts
        // This makes resolve_current_module_name() return None
        let mut engine = engine_from_sources(map(&[(
            "lib.xml",
            r#"<module name="lib" default_access="public">
  <script name="secret" access="private"><text>hidden</text></script>
</module>"#,
        )]));

        // Start but then remove the script from scripts
        engine
            .start("lib.secret", None)
            .expect_err("private entry should fail");

        // Create another engine and set up frames manually
        let mut engine2 = engine_from_sources(map(&[
            (
                "main.xml",
                r#"<module name="main" default_access="public">
  <script name="main"><text>hello</text></script>
</module>"#,
            ),
            (
                "lib.xml",
                r#"<module name="lib" default_access="public">
  <script name="secret" access="private"><text>hidden</text></script>
</module>"#,
            ),
        ]));

        // Start the main script
        engine2.start("main.main", None).expect("start");

        // Remove main from scripts but keep group_lookup
        let _group_id = engine2.frames.last().unwrap().group_id.clone();
        engine2.scripts.remove("main.main");

        // Create a target that exists and is private
        let private_target = sl_core::ScriptIr {
            script_path: "lib.xml".to_string(),
            script_name: "secret".to_string(),
            access: sl_core::AccessLevel::Private,
            module_name: Some("lib".to_string()),
            local_script_name: Some("secret".to_string()),
            params: vec![],
            root_group_id: "g2".to_string(),
            groups: Default::default(),
            visible_functions: Default::default(),
            visible_module_vars: Default::default(),
            visible_module_consts: Default::default(),
            visible_globals: vec![],
            invoke_all_functions: Default::default(),
            invoke_public_functions: Default::default(),
        };

        // Now resolve_current_module_name() should return None (because main is not in scripts)
        // and target has module_name = Some("lib") - should hit line 29
        let result = engine2.validate_script_access_from_current("lib.secret", &private_target);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, "ENGINE_SCRIPT_ACCESS_DENIED");
    }
}
