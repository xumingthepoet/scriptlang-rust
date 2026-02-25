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
