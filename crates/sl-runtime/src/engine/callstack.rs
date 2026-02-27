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
        let caller_group_len = {
            let (_, caller_group) = self.lookup_group(&caller_group_id)?;
            caller_group.nodes.len()
        };

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

#[cfg(test)]
mod callstack_tests {
    use super::*;
    use super::runtime_test_support::*;

    #[test]
    fn nested_script_calls_covered() {
        // Test nested script calls
        let mut engine = engine_from_sources(map(&[
            (
                "main.script.xml",
                r#"
    <script name="main">
      <call script="greeting"/>
    </script>
    "#,
            ),
            (
                "greeting.script.xml",
                r#"<script name="greeting"><text>Hi</text></script>"#,
            ),
        ]));
        engine.start("main", None).expect("start");
    
        let output = engine.next_output().expect("next should pass");
        assert!(matches!(output, EngineOutput::Text { text, .. } if text == "Hi"));
    }

    #[test]
    fn runtime_errors_cover_call_argument_and_return_target_paths() {
        let mut call_missing_target = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><call script="missing"/></script>"#,
        )]));
        call_missing_target.start("main", None).expect("start");
        let error = call_missing_target
            .next_output()
            .expect_err("missing call target should fail");
        assert_eq!(error.code, "ENGINE_CALL_TARGET");
    
        let mut call_arg_mismatch = engine_from_sources(map(&[
            (
                "main.script.xml",
                r#"
    <!-- include: callee.script.xml -->
    <script name="main">
      <var name="hp" type="int">1</var>
      <call script="callee" args="hp"/>
    </script>
    "#,
            ),
            (
                "callee.script.xml",
                r#"<script name="callee" args="ref:int:x"><return/></script>"#,
            ),
        ]));
        call_arg_mismatch.start("main", None).expect("start");
        let error = call_arg_mismatch
            .next_output()
            .expect_err("ref mismatch should fail");
        assert_eq!(error.code, "ENGINE_CALL_REF_MISMATCH");
    
        let mut return_target_missing = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><return script="missing"/></script>"#,
        )]));
        return_target_missing.start("main", None).expect("start");
        let error = return_target_missing
            .next_output()
            .expect_err("missing return target should fail");
        assert_eq!(error.code, "ENGINE_RETURN_TARGET");
    }

    #[test]
    fn finish_frame_and_return_paths_are_covered() {
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
            .execute_return(Some("next".to_string()), &[])
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
    fn return_forwarding_and_root_index_success_paths_are_covered() {
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
            .execute_return(Some("next".to_string()), &[])
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
    }

    #[test]
    fn call_helpers_and_value_path_branches_are_covered() {
        let mut no_frame = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>x</text></script>"#,
        )]));
        no_frame.frames.clear();
        let error = no_frame
            .execute_call("main", &[])
            .expect_err("execute_call without frame should fail");
        assert_eq!(error.code, "ENGINE_CALL_NO_FRAME");
    
        let mut ref_mismatch = engine_from_sources(map(&[
            (
                "main.script.xml",
                r#"
    <!-- include: callee.script.xml -->
    <script name="main">
      <var name="x" type="int">1</var>
      <call script="callee" args="ref:x"/>
    </script>
    "#,
            ),
            (
                "callee.script.xml",
                r#"<script name="callee" args="int:x"><return/></script>"#,
            ),
        ]));
        ref_mismatch.start("main", None).expect("start");
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
                "callee",
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
                "callee",
                &[sl_core::CallArgument {
                    value_expr: "x".to_string(),
                    is_ref: false,
                }],
            )
            .expect("tail call optimization path should pass");
        assert_eq!(tail_ok.frames.len(), 1);
    
        let mut globals = engine_from_sources(map(&[
            ("game.json", r#"{ "score": 10 }"#),
            (
                "main.script.xml",
                r#"
    <!-- include: game.json -->
    <script name="main">
      <var name="x" type="int">1</var>
      <code>x = x + game.score;</code>
      <text>${x}</text>
    </script>
    "#,
            ),
        ]));
        globals.start("main", None).expect("start");
        let output = globals.next_output().expect("next");
        assert!(matches!(output, EngineOutput::Text { text, .. } if text == "11"));
        assert!(!globals.is_visible_json_global(None, "game"));
        assert!(!globals.is_visible_json_global(Some("missing"), "game"));
        assert!(globals.is_visible_json_global(Some("main"), "game"));
    
        let value = globals
            .read_variable("game")
            .expect("visible json global should be readable");
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
                .build_defs_prelude("missing-script", &BTreeMap::new())
                .expect("missing script prelude should be empty"),
            ""
        );
        let defs_engine = engine_from_sources(map(&[
            (
                "main.script.xml",
                r#"
    <!-- include: shared.defs.xml -->
    <script name="main"><text>x</text></script>
    "#,
            ),
            (
                "shared.defs.xml",
                r#"<defs name="shared"><function name="make" return="int:out">out = 1;</function></defs>"#,
            ),
        ]));
        let error = defs_engine
            .build_defs_prelude("main", &BTreeMap::new())
            .expect_err("missing symbol mapping should fail");
        assert_eq!(error.code, "ENGINE_DEFS_FUNCTION_SYMBOL_MISSING");
    
        let registry = TestRegistry {
            names: vec!["f".to_string()],
        };
        let call_value = registry.call("f", &[]).expect("test registry call");
        assert_eq!(call_value, SlValue::Bool(true));
    }

    #[test]
    fn defs_function_call_execution_is_covered() {
        // Test actual defs function call to cover rhai_bridge.rs rewrite code
        let mut engine = engine_from_sources(map(&[
            (
                "shared.defs.xml",
                r#"<defs name="shared">
  <function name="add" args="int:a,int:b" return="int:result">
    result = a + b;
  </function>
</defs>"#,
            ),
            (
                "main.script.xml",
                r#"<!-- include: shared.defs.xml -->
<script name="main">
  <text>${shared.add(1, 2)}</text>
</script>"#,
            ),
        ]));
        engine.start("main", None).expect("start");
        let output = engine.next_output().expect("next");
        assert!(matches!(output, EngineOutput::Text { text, .. } if text == "3"));
    }

}
