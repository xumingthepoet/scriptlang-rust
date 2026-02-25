impl ScriptLangEngine {
    fn create_script_root_scope(
        &self,
        script_name: &str,
        arg_values: BTreeMap<String, SlValue>,
    ) -> Result<ScopeInit, ScriptLangError> {
        let script = self.scripts.get(script_name).ok_or_else(|| {
            ScriptLangError::new(
                "ENGINE_SCRIPT_NOT_FOUND",
                format!("Script \"{}\" not found.", script_name),
            )
        })?;

        let mut scope = BTreeMap::new();
        let mut var_types = BTreeMap::new();

        for param in &script.params {
            let value = default_value_from_type(&param.r#type);
            scope.insert(param.name.clone(), value);
            var_types.insert(param.name.clone(), param.r#type.clone());
        }

        for (name, value) in arg_values {
            if !scope.contains_key(&name) {
                return Err(ScriptLangError::new(
                    "ENGINE_CALL_ARG_UNKNOWN",
                    format!(
                        "Call argument \"{}\" is not declared in target script.",
                        name
                    ),
                ));
            }
            let expected_type = var_types
                .get(&name)
                .expect("script scope types should include all declared params");
            if !is_type_compatible(&value, expected_type) {
                return Err(ScriptLangError::new(
                    "ENGINE_TYPE_MISMATCH",
                    format!("Call argument \"{}\" does not match declared type.", name),
                ));
            }
            scope.insert(name, value);
        }

        Ok((scope, var_types))
    }

    fn render_text(&mut self, template: &str) -> Result<String, ScriptLangError> {
        let regex = Regex::new(r"\$\{([^{}]+)\}").expect("template regex must compile");
        let mut output = String::new();
        let mut last_index = 0usize;
        for captures in regex.captures_iter(template) {
            let full = captures
                .get(0)
                .expect("capture group 0 must exist for each regex capture");
            let expr = captures
                .get(1)
                .expect("capture group 1 must exist for each regex capture");
            output.push_str(&template[last_index..full.start()]);
            let value = self.eval_expression(expr.as_str())?;
            output.push_str(&slvalue_to_text(&value));
            last_index = full.end();
        }
        output.push_str(&template[last_index..]);
        Ok(output)
    }

    fn eval_boolean(&mut self, expr: &str) -> Result<bool, ScriptLangError> {
        let value = self.eval_expression(expr)?;
        match value {
            SlValue::Bool(value) => Ok(value),
            _ => Err(ScriptLangError::new(
                "ENGINE_BOOLEAN_EXPECTED",
                format!("Expression \"{}\" must evaluate to boolean.", expr),
            )),
        }
    }

    fn run_code(&mut self, code: &str) -> Result<(), ScriptLangError> {
        self.execute_rhai(code, false).map(|_| ())
    }

    fn eval_expression(&mut self, expr: &str) -> Result<SlValue, ScriptLangError> {
        self.execute_rhai(expr, true)
    }

    fn execute_rhai(
        &mut self,
        script: &str,
        is_expression: bool,
    ) -> Result<SlValue, ScriptLangError> {
        let script_name = self.resolve_current_script_name().unwrap_or_default();
        let function_symbol_map = self
            .visible_function_symbols_by_script
            .get(&script_name)
            .cloned()
            .unwrap_or_default();

        if !self.host_functions.names().is_empty() {
            return Err(ScriptLangError::new(
                "ENGINE_HOST_FUNCTION_UNSUPPORTED",
                "Host function invocation is not yet supported in this runtime build.",
            ));
        }

        let (mutable_bindings, mutable_order) = self.collect_mutable_bindings();
        let visible_globals = self
            .visible_json_by_script
            .get(&script_name)
            .cloned()
            .unwrap_or_default();

        let mut scope = Scope::new();
        for name in &mutable_order {
            let binding = mutable_bindings
                .get(name)
                .expect("mutable order should only contain known bindings");
            scope.push_dynamic(name.to_string(), slvalue_to_dynamic(&binding.value)?);
        }

        let mut global_snapshot = BTreeMap::new();
        for name in visible_globals {
            let value = self
                .global_json
                .get(&name)
                .expect("visible globals should exist in global json map");
            global_snapshot.insert(name.clone(), value.clone());
            scope.push_dynamic(name, slvalue_to_dynamic(value)?);
        }

        let mut engine = Engine::new();
        engine.set_strict_variables(true);

        let rng_state = Rc::new(RefCell::new(self.rng_state));
        let rng_state_clone = Rc::clone(&rng_state);
        engine.register_fn(
            "random",
            move |bound: INT| -> Result<INT, Box<EvalAltResult>> {
                if bound <= 0 {
                    return Err(Box::new(EvalAltResult::ErrorRuntime(
                        Dynamic::from("random(n) expects positive integer n."),
                        Position::NONE,
                    )));
                }

                let mut state = rng_state_clone.borrow_mut();
                let value = next_random_bounded(&mut state, bound as u32);
                Ok(value as INT)
            },
        );

        let prelude = self.build_defs_prelude(&script_name, &function_symbol_map)?;
        let rewritten_script = rewrite_function_calls(script, &function_symbol_map)?;
        let source = if is_expression {
            if prelude.is_empty() {
                format!("({})", rewritten_script)
            } else {
                format!("{}\n({})", prelude, rewritten_script)
            }
        } else if prelude.is_empty() {
            rewritten_script
        } else {
            format!("{}\n{}", prelude, rewritten_script)
        };

        let run_result = if is_expression {
            engine
                .eval_with_scope::<Dynamic>(&mut scope, &source)
                .map_err(|error| {
                    ScriptLangError::new(
                        "ENGINE_EVAL_ERROR",
                        format!("Expression eval failed: {}", error),
                    )
                })
                .and_then(dynamic_to_slvalue)
        } else {
            engine
                .run_with_scope(&mut scope, &source)
                .map_err(|error| {
                    ScriptLangError::new(
                        "ENGINE_EVAL_ERROR",
                        format!("Code eval failed: {}", error),
                    )
                })
                .map(|_| SlValue::Bool(true))
        };

        self.rng_state = *rng_state.borrow();

        for (name, before) in global_snapshot {
            let after_dynamic = scope
                .get_value::<Dynamic>(&name)
                .expect("scope should still contain visible globals");
            let after = dynamic_to_slvalue(after_dynamic)?;
            if after != before {
                return Err(ScriptLangError::new(
                    "ENGINE_GLOBAL_READONLY",
                    format!(
                        "Global JSON \"{}\" is readonly and cannot be mutated.",
                        name
                    ),
                ));
            }
        }

        for name in mutable_order {
            let after_dynamic = scope
                .get_value::<Dynamic>(&name)
                .expect("scope should still contain mutable bindings");
            let after = dynamic_to_slvalue(after_dynamic)?;
            self.write_variable(&name, after)?;
        }

        run_result
    }

    fn build_defs_prelude(
        &self,
        script_name: &str,
        function_symbol_map: &BTreeMap<String, String>,
    ) -> Result<String, ScriptLangError> {
        let Some(script) = self.scripts.get(script_name) else {
            return Ok(String::new());
        };
        let visible_json = self
            .visible_json_by_script
            .get(script_name)
            .cloned()
            .unwrap_or_default();

        let mut out = String::new();
        for (name, decl) in &script.visible_functions {
            let rhai_name = function_symbol_map.get(name).cloned().ok_or_else(|| {
                ScriptLangError::new(
                    "ENGINE_DEFS_FUNCTION_SYMBOL_MISSING",
                    format!("Missing Rhai function symbol mapping for \"{}\".", name),
                )
            })?;
            out.push_str("fn ");
            out.push_str(&rhai_name);
            out.push('(');
            out.push_str(
                &decl
                    .params
                    .iter()
                    .map(|param| param.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            out.push_str(") {\n");

            for json_symbol in &visible_json {
                if let Some(value) = self.global_json.get(json_symbol) {
                    out.push_str(&format!(
                        "let {} = {};\n",
                        json_symbol,
                        slvalue_to_rhai_literal(value)
                    ));
                }
            }

            let default_value = default_value_from_type(&decl.return_binding.r#type);
            out.push_str(&format!(
                "let {} = {};\n",
                decl.return_binding.name,
                slvalue_to_rhai_literal(&default_value)
            ));
            out.push_str(&rewrite_function_calls(&decl.code, function_symbol_map)?);
            out.push('\n');
            out.push_str(&decl.return_binding.name);
            out.push_str("\n}\n");
        }

        Ok(out)
    }

    fn collect_mutable_bindings(&self) -> (BTreeMap<String, BindingOwner>, Vec<String>) {
        let mut map = BTreeMap::new();
        let mut order = Vec::new();
        for frame in self.frames.iter().rev() {
            for (name, value) in &frame.scope {
                if map.contains_key(name) {
                    continue;
                }
                map.insert(
                    name.clone(),
                    BindingOwner {
                        value: value.clone(),
                    },
                );
                order.push(name.clone());
            }
        }
        (map, order)
    }

}

#[cfg(test)]
mod eval_tests {
    use super::*;
    use super::runtime_test_support::*;

    #[test]
    fn global_json_is_readonly_during_code_execution() {
        let mut engine = engine_from_sources(map(&[
            ("game.json", r#"{ "bonus": 10 }"#),
            (
                "main.script.xml",
                r#"
    <!-- include: game.json -->
    <script name="main">
      <code>game.bonus = 11;</code>
    </script>
    "#,
            ),
        ]));
        engine.start("main", None).expect("start");
        let error = engine
            .next_output()
            .expect_err("global mutation should fail");
        assert_eq!(error.code, "ENGINE_GLOBAL_READONLY");
    }

    #[test]
    fn helper_functions_cover_paths_values_and_rng() {
        assert_eq!(
            parse_ref_path(" player . hp . current "),
            vec![
                "player".to_string(),
                "hp".to_string(),
                "current".to_string()
            ]
        );
        assert!(parse_ref_path(" . ").is_empty());
    
        let mut root = SlValue::Map(BTreeMap::from([(
            "player".to_string(),
            SlValue::Map(BTreeMap::from([("hp".to_string(), SlValue::Number(10.0))])),
        )]));
        assign_nested_path(
            &mut root,
            &["player".to_string(), "hp".to_string()],
            SlValue::Number(9.0),
        )
        .expect("assign nested should pass");
        assert_eq!(
            root,
            SlValue::Map(BTreeMap::from([(
                "player".to_string(),
                SlValue::Map(BTreeMap::from([("hp".to_string(), SlValue::Number(9.0))]))
            )]))
        );
    
        let mut replacement = SlValue::String("old".to_string());
        assign_nested_path(&mut replacement, &[], SlValue::String("new".to_string()))
            .expect("empty path should replace root");
        assert_eq!(replacement, SlValue::String("new".to_string()));
    
        let mut not_map = SlValue::Number(1.0);
        let error = assign_nested_path(&mut not_map, &["x".to_string()], SlValue::Number(2.0))
            .expect_err("non-map should fail");
        assert_eq!(error, "target is not an object/map");
    
        let mut missing = SlValue::Map(BTreeMap::new());
        let error = assign_nested_path(
            &mut missing,
            &["unknown".to_string(), "v".to_string()],
            SlValue::Number(2.0),
        )
        .expect_err("missing key should fail");
        assert!(error.contains("missing key"));
    
        assert_eq!(slvalue_to_text(&SlValue::Number(3.0)), "3");
        assert_eq!(slvalue_to_text(&SlValue::Number(3.5)), "3.5");
        assert_eq!(slvalue_to_text(&SlValue::Bool(true)), "true");
    
        let value = SlValue::Map(BTreeMap::from([
            ("a".to_string(), SlValue::Number(1.0)),
            (
                "b".to_string(),
                SlValue::Array(vec![SlValue::Bool(false), SlValue::String("x".to_string())]),
            ),
        ]));
        let dynamic = slvalue_to_dynamic(&value).expect("to dynamic");
        let roundtrip = dynamic_to_slvalue(dynamic).expect("from dynamic");
        assert_eq!(roundtrip, value);
    
        let unsupported = dynamic_to_slvalue(Dynamic::UNIT).expect_err("unsupported type");
        assert_eq!(unsupported.code, "ENGINE_VALUE_UNSUPPORTED");
    
        let literal = slvalue_to_rhai_literal(&SlValue::Map(BTreeMap::from([(
            "name".to_string(),
            SlValue::String("A\"B".to_string()),
        )])));
        assert_eq!(literal, "#{name: \"A\\\"B\"}");
    
        let mut state = 1u32;
        let a = next_random_u32(&mut state);
        let b = next_random_u32(&mut state);
        assert_ne!(a, b);
        let bounded = next_random_bounded(&mut state, 7);
        assert!(bounded < 7);
    
        let mut deterministic_state = 0u32;
        let mut sequence = [u32::MAX, 3u32].into_iter();
        let bounded_retry = next_random_bounded_with(&mut deterministic_state, 10, |_| {
            sequence
                .next()
                .expect("deterministic sequence should have two draws")
        });
        assert_eq!(bounded_retry, 3);
    }

    #[test]
    fn runtime_errors_cover_input_boolean_random_and_host_unsupported() {
        let mut input_type = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <var name="hp" type="int">1</var>
      <input var="hp" text="bad"/>
    </script>
    "#,
        )]));
        input_type.start("main", None).expect("start");
        let error = input_type
            .next_output()
            .expect_err("input on non-string should fail");
        assert_eq!(error.code, "ENGINE_INPUT_VAR_TYPE");
    
        let mut if_non_bool = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><if when="1"><text>A</text></if></script>"#,
        )]));
        if_non_bool.start("main", None).expect("start");
        let error = if_non_bool
            .next_output()
            .expect_err("non-boolean if should fail");
        assert_eq!(error.code, "ENGINE_BOOLEAN_EXPECTED");
    
        let mut random_bad = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><var name="x" type="int">random(0)</var></script>"#,
        )]));
        random_bad.start("main", None).expect("start");
        let error = random_bad.next_output().expect_err("random(0) should fail");
        assert_eq!(error.code, "ENGINE_EVAL_ERROR");
    
        let files = sources_from_example_dir("01-text-code");
        let compiled = compile_project_bundle_from_xml_map(&files).expect("compile");
        let mut host_unsupported = ScriptLangEngine::new(ScriptLangEngineOptions {
            scripts: compiled.scripts,
            global_json: compiled.global_json,
            host_functions: Some(Arc::new(TestRegistry {
                names: vec!["ext_fn".to_string()],
            })),
            random_seed: Some(1),
            compiler_version: Some(DEFAULT_COMPILER_VERSION.to_string()),
        })
        .expect("engine should build");
        host_unsupported.start("main", None).expect("start");
        let error = host_unsupported
            .next_output()
            .expect_err("host functions unsupported");
        assert_eq!(error.code, "ENGINE_HOST_FUNCTION_UNSUPPORTED");
    }

    #[test]
    fn runtime_private_helpers_cover_additional_error_paths() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>Hello</text></script>"#,
        )]));
        engine.start("main", None).expect("start");
    
        // lookup_group: script missing
        let key = engine
            .group_lookup
            .keys()
            .next()
            .expect("group key")
            .to_string();
        if let Some(lookup) = engine.group_lookup.get_mut(&key) {
            lookup.script_name = "missing".to_string();
        }
        let error = engine
            .lookup_group(&key)
            .expect_err("script should be missing");
        assert_eq!(error.code, "ENGINE_SCRIPT_NOT_FOUND");
    
        // restore engine for following checks
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>Hello</text></script>"#,
        )]));
        engine.start("main", None).expect("start");
        let key = engine
            .group_lookup
            .keys()
            .next()
            .expect("group key")
            .to_string();
        if let Some(lookup) = engine.group_lookup.get_mut(&key) {
            lookup.group_id = "missing-group".to_string();
        }
        let error = engine
            .lookup_group(&key)
            .expect_err("group should be missing");
        assert_eq!(error.code, "ENGINE_GROUP_NOT_FOUND");
    
        // execute_continue_while: while body at index 0 has no owner
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>Hello</text></script>"#,
        )]));
        engine.frames = vec![RuntimeFrame {
            frame_id: 1,
            group_id: "main.script.xml::g0".to_string(),
            node_index: 0,
            scope: BTreeMap::new(),
            completion: CompletionKind::WhileBody,
            script_root: false,
            return_continuation: None,
            var_types: BTreeMap::new(),
        }];
        let error = engine
            .execute_continue_while()
            .expect_err("no owning while frame");
        assert_eq!(error.code, "ENGINE_WHILE_CONTROL_TARGET_MISSING");
    
        // execute_break: owner exists but node is not while
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>Hello</text></script>"#,
        )]));
        engine.frames = vec![
            RuntimeFrame {
                frame_id: 1,
                group_id: "main.script.xml::g0".to_string(),
                node_index: 0,
                scope: BTreeMap::new(),
                completion: CompletionKind::None,
                script_root: true,
                return_continuation: None,
                var_types: BTreeMap::new(),
            },
            RuntimeFrame {
                frame_id: 2,
                group_id: "main.script.xml::g0".to_string(),
                node_index: 0,
                scope: BTreeMap::new(),
                completion: CompletionKind::WhileBody,
                script_root: false,
                return_continuation: None,
                var_types: BTreeMap::new(),
            },
        ];
        let error = engine
            .execute_break()
            .expect_err("while owner node missing");
        assert_eq!(error.code, "ENGINE_WHILE_CONTROL_TARGET_MISSING");
    
        // execute_continue_choice without choice context
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>Hello</text></script>"#,
        )]));
        engine.start("main", None).expect("start");
        let error = engine
            .execute_continue_choice()
            .expect_err("no choice context");
        assert_eq!(error.code, "ENGINE_CHOICE_CONTINUE_TARGET_MISSING");
    }

}
