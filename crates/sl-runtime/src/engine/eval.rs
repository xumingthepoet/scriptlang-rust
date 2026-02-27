fn text_interpolation_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"\$\{([^{}]+)\}").expect("template regex must compile"))
}

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
        let mut output = String::new();
        let mut last_index = 0usize;
        for captures in text_interpolation_regex().captures_iter(template) {
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

    fn eval_defs_global_initializer(&mut self, expr: &str) -> Result<SlValue, ScriptLangError> {
        if !self.host_functions.names().is_empty() {
            return Err(ScriptLangError::new(
                "ENGINE_HOST_FUNCTION_UNSUPPORTED",
                "Host function invocation is not yet supported in this runtime build.",
            ));
        }

        let mut namespace_values: BTreeMap<String, BTreeMap<String, SlValue>> = BTreeMap::new();
        let mut qualified_rewrite_map = BTreeMap::new();
        for (qualified_name, value) in &self.defs_globals_value {
            let Some((namespace, name)) = qualified_name.split_once('.') else {
                continue;
            };
            namespace_values
                .entry(namespace.to_string())
                .or_default()
                .insert(name.to_string(), value.clone());
            qualified_rewrite_map.insert(
                qualified_name.clone(),
                format!("{}.{}", defs_namespace_symbol(namespace), name),
            );
        }

        let mut scope = Scope::new();
        for (namespace, values) in &namespace_values {
            scope.push_dynamic(
                defs_namespace_symbol(namespace),
                slvalue_to_dynamic(&SlValue::Map(values.clone()))?,
            );
        }

        for (alias, qualified_name) in self.collect_bundle_defs_short_aliases() {
            if let Some(value) = self.defs_globals_value.get(&qualified_name) {
                scope.push_dynamic(alias, slvalue_to_dynamic(value)?);
            }
        }

        let mut global_snapshot = BTreeMap::new();
        for (name, value) in &self.global_json {
            global_snapshot.insert(name.clone(), value.clone());
            scope.push_dynamic(name.clone(), slvalue_to_dynamic(value)?);
        }

        let rewritten = rewrite_defs_global_qualified_access(expr, &qualified_rewrite_map)?;
        *self.shared_rng_state.borrow_mut() = self.rng_state;
        let result = self
            .rhai_engine
            .eval_with_scope::<Dynamic>(&mut scope, &format!("({})", rewritten))
            .map_err(|error| {
                ScriptLangError::new(
                    "ENGINE_EVAL_ERROR",
                    format!("Defs global initializer eval failed: {}", error),
                )
            })
            .and_then(dynamic_to_slvalue);
        self.rng_state = *self.shared_rng_state.borrow();

        for (name, before) in global_snapshot {
            let after_dynamic = scope
                .get_value::<Dynamic>(&name)
                .expect("scope should still contain global snapshot bindings");
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

        result
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
        let visible_defs = self
            .visible_defs_by_script
            .get(&script_name)
            .cloned()
            .unwrap_or_default();
        let defs_alias_map = self
            .defs_global_alias_by_script
            .get(&script_name)
            .cloned()
            .unwrap_or_default();
        let qualified_rewrite_map = self.build_defs_global_qualified_rewrite_map(&script_name);

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

        let mut defs_namespace_snapshot = BTreeMap::new();
        for qualified_name in &visible_defs {
            let Some((namespace, name)) = qualified_name.split_once('.') else {
                continue;
            };
            let value = self
                .defs_globals_value
                .get(qualified_name)
                .cloned()
                .ok_or_else(|| {
                    ScriptLangError::new(
                        "ENGINE_DEFS_GLOBAL_MISSING",
                        format!("Defs global \"{}\" is not initialized.", qualified_name),
                    )
                })?;
            defs_namespace_snapshot
                .entry(namespace.to_string())
                .or_insert_with(BTreeMap::new)
                .insert(name.to_string(), value);
        }

        let mut defs_namespace_symbols = BTreeMap::new();
        for (namespace, values) in &defs_namespace_snapshot {
            let symbol = defs_namespace_symbol(namespace);
            defs_namespace_symbols.insert(namespace.clone(), symbol.clone());
            scope.push_dynamic(symbol, slvalue_to_dynamic(&SlValue::Map(values.clone()))?);
        }

        let mut short_defs_aliases = BTreeMap::new();
        for (alias, qualified_name) in defs_alias_map {
            if alias.contains('.') || mutable_bindings.contains_key(&alias) {
                continue;
            }
            if !visible_defs.contains(&qualified_name) {
                continue;
            }

            let value = self
                .defs_globals_value
                .get(&qualified_name)
                .cloned()
                .ok_or_else(|| {
                    ScriptLangError::new(
                        "ENGINE_DEFS_GLOBAL_MISSING",
                        format!("Defs global \"{}\" is not initialized.", qualified_name),
                    )
                })?;
            scope.push_dynamic(alias.clone(), slvalue_to_dynamic(&value)?);
            short_defs_aliases.insert(alias, (qualified_name, value));
        }

        let mut global_snapshot = BTreeMap::new();
        for name in visible_globals {
            if mutable_bindings.contains_key(&name) || short_defs_aliases.contains_key(&name) {
                continue;
            }
            let value = self
                .global_json
                .get(&name)
                .expect("visible globals should exist in global json map");
            global_snapshot.insert(name.clone(), value.clone());
            scope.push_dynamic(name, slvalue_to_dynamic(value)?);
        }

        let source = {
            let prelude = self.get_or_build_defs_prelude(&script_name, &function_symbol_map)?;
            let rewritten_script = rewrite_function_calls(script, &function_symbol_map)?;
            let rewritten_script =
                rewrite_defs_global_qualified_access(&rewritten_script, &qualified_rewrite_map)?;
            if is_expression {
                if prelude.is_empty() {
                    format!("({})", rewritten_script)
                } else {
                    format!("{}\n({})", prelude, rewritten_script)
                }
            } else if prelude.is_empty() {
                rewritten_script
            } else {
                format!("{}\n{}", prelude, rewritten_script)
            }
        };

        *self.shared_rng_state.borrow_mut() = self.rng_state;
        let run_result = if is_expression {
            self.rhai_engine
                .eval_with_scope::<Dynamic>(&mut scope, &source)
                .map_err(|error| {
                    ScriptLangError::new(
                        "ENGINE_EVAL_ERROR",
                        format!("Expression eval failed: {}", error),
                    )
                })
                .and_then(dynamic_to_slvalue)
        } else {
            self.rhai_engine
                .run_with_scope(&mut scope, &source)
                .map_err(|error| {
                    ScriptLangError::new(
                        "ENGINE_EVAL_ERROR",
                        format!("Code eval failed: {}", error),
                    )
                })
                .map(|_| SlValue::Bool(true))
        };

        self.rng_state = *self.shared_rng_state.borrow();

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

        for (namespace, symbol) in defs_namespace_symbols {
            let after_dynamic = scope
                .get_value::<Dynamic>(&symbol)
                .expect("scope should still contain defs global namespace symbols");
            let after = dynamic_to_slvalue(after_dynamic)?;
            let SlValue::Map(entries) = after else {
                return Err(ScriptLangError::new(
                    "ENGINE_DEFS_GLOBAL_NAMESPACE_TYPE",
                    format!("Defs global namespace \"{}\" is not a map value.", namespace),
                ));
            };

            for (name, value) in entries {
                let qualified_name = format!("{}.{}", namespace, name);
                if !visible_defs.contains(&qualified_name) {
                    continue;
                }
                let declared_type = self.defs_globals_type.get(&qualified_name).ok_or_else(|| {
                    ScriptLangError::new(
                        "ENGINE_DEFS_GLOBAL_DECL_MISSING",
                        format!(
                            "Defs global \"{}\" is visible but declaration is missing.",
                            qualified_name
                        ),
                    )
                })?;
                if !is_type_compatible(&value, declared_type) {
                    return Err(ScriptLangError::new(
                        "ENGINE_TYPE_MISMATCH",
                        format!(
                            "Defs global \"{}\" does not match declared type.",
                            qualified_name
                        ),
                    ));
                }
                self.defs_globals_value.insert(qualified_name, value);
            }
        }

        for (alias, (qualified_name, before_value)) in short_defs_aliases {
            let after_dynamic = scope
                .get_value::<Dynamic>(&alias)
                .expect("scope should still contain short defs alias");
            let after = dynamic_to_slvalue(after_dynamic)?;
            if after == before_value {
                continue;
            }
            let declared_type = self.defs_globals_type.get(&qualified_name).ok_or_else(|| {
                ScriptLangError::new(
                    "ENGINE_DEFS_GLOBAL_DECL_MISSING",
                    format!(
                        "Defs global \"{}\" is visible but declaration is missing.",
                        qualified_name
                    ),
                )
            })?;
            if !is_type_compatible(&after, declared_type) {
                return Err(ScriptLangError::new(
                    "ENGINE_TYPE_MISMATCH",
                    format!("Defs global \"{}\" does not match declared type.", qualified_name),
                ));
            }
            self.defs_globals_value.insert(qualified_name, after);
        }

        run_result
    }

    fn get_or_build_defs_prelude(
        &mut self,
        script_name: &str,
        function_symbol_map: &BTreeMap<String, String>,
    ) -> Result<&str, ScriptLangError> {
        if !self.defs_prelude_by_script.contains_key(script_name) {
            let prelude = self.build_defs_prelude(script_name, function_symbol_map)?;
            self.defs_prelude_by_script
                .insert(script_name.to_string(), prelude);
        }
        Ok(self
            .defs_prelude_by_script
            .get(script_name)
            .map(String::as_str)
            .expect("defs prelude should be cached"))
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
            .expect("script visibility should exist for registered script");
        let qualified_rewrite_map = self.build_defs_global_qualified_rewrite_map(script_name);

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

            for json_symbol in visible_json {
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
            let rewritten = rewrite_function_calls(&decl.code, function_symbol_map)?;
            let rewritten =
                rewrite_defs_global_qualified_access(&rewritten, &qualified_rewrite_map)?;
            out.push_str(&rewritten);
            out.push('\n');
            out.push_str(&decl.return_binding.name);
            out.push_str("\n}\n");
        }

        Ok(out)
    }

    fn build_defs_global_qualified_rewrite_map(&self, script_name: &str) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        let Some(visible_defs) = self.visible_defs_by_script.get(script_name) else {
            return out;
        };

        for qualified_name in visible_defs {
            let Some((namespace, name)) = qualified_name.split_once('.') else {
                continue;
            };
            out.insert(
                qualified_name.clone(),
                format!("{}.{}", defs_namespace_symbol(namespace), name),
            );
        }

        out
    }

    fn collect_bundle_defs_short_aliases(&self) -> BTreeMap<String, String> {
        let mut candidates: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for decl in self.defs_global_declarations.values() {
            candidates
                .entry(decl.name.clone())
                .or_default()
                .push(decl.qualified_name.clone());
        }

        candidates
            .into_iter()
            .filter_map(|(short_name, qualified_names)| {
                if qualified_names.len() == 1 {
                    Some((short_name, qualified_names[0].clone()))
                } else {
                    None
                }
            })
            .collect()
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
    
        let files = map(&[(
            "main.script.xml",
            r#"
<script name="main">
  <var name="count" type="int">1</var>
  <code>count = count + 1;</code>
</script>
"#,
        )]);
        let compiled = compile_project_bundle_from_xml_map(&files).expect("compile");
        let mut host_unsupported = ScriptLangEngine::new(ScriptLangEngineOptions {
            scripts: compiled.scripts,
            global_json: compiled.global_json,
            defs_global_declarations: compiled.defs_global_declarations,
            defs_global_init_order: compiled.defs_global_init_order,
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
    fn defs_global_eval_and_internal_error_paths_are_covered() {
        let host_blocked_files = map(&[
            (
                "shared.defs.xml",
                r#"<defs name="shared"><var name="hp" type="int">1</var></defs>"#,
            ),
            (
                "main.script.xml",
                r#"
<!-- include: shared.defs.xml -->
<script name="main"><text>ok</text></script>
"#,
            ),
        ]);
        let host_blocked_compiled =
            compile_project_bundle_from_xml_map(&host_blocked_files).expect("compile");
        let mut host_blocked = ScriptLangEngine::new(ScriptLangEngineOptions {
            scripts: host_blocked_compiled.scripts,
            global_json: host_blocked_compiled.global_json,
            defs_global_declarations: host_blocked_compiled.defs_global_declarations,
            defs_global_init_order: host_blocked_compiled.defs_global_init_order,
            host_functions: Some(Arc::new(TestRegistry {
                names: vec!["ext_fn".to_string()],
            })),
            random_seed: Some(1),
            compiler_version: None,
        })
        .expect("engine");
        let error = host_blocked
            .start("main", None)
            .expect_err("initializer should reject host function mode");
        assert_eq!(error.code, "ENGINE_HOST_FUNCTION_UNSUPPORTED");

        let initializer_files = map(&[
            ("game.json", r#"{ "hp": 5 }"#),
            (
                "shared.defs.xml",
                r#"
<defs name="shared">
  <var name="a" type="int">1</var>
  <var name="b" type="int">a + game.hp</var>
</defs>
"#,
            ),
            (
                "main.script.xml",
                r#"
<!-- include: shared.defs.xml -->
<!-- include: game.json -->
<script name="main"><text>${shared.b}</text></script>
"#,
            ),
        ]);
        let mut initializer_engine = engine_from_sources(initializer_files);
        initializer_engine.start("main", None).expect("start");
        assert_eq!(
            initializer_engine.defs_globals_value.get("shared.b"),
            Some(&SlValue::Number(6.0))
        );

        let bad_initializer = map(&[
            (
                "shared.defs.xml",
                r#"<defs name="shared"><var name="hp" type="int">unknown +</var></defs>"#,
            ),
            (
                "main.script.xml",
                r#"
<!-- include: shared.defs.xml -->
<script name="main"><text>ok</text></script>
"#,
            ),
        ]);
        let mut bad_initializer_engine = engine_from_sources(bad_initializer);
        let error = bad_initializer_engine
            .start("main", None)
            .expect_err("bad initializer should fail");
        assert_eq!(error.code, "ENGINE_EVAL_ERROR");

        let readonly_initializer = map(&[
            ("game.json", r#"{ "hp": 5 }"#),
            (
                "shared.defs.xml",
                r#"<defs name="shared"><var name="hp" type="int">{ game = 1; 1 }</var></defs>"#,
            ),
            (
                "main.script.xml",
                r#"
<!-- include: shared.defs.xml -->
<!-- include: game.json -->
<script name="main"><text>ok</text></script>
"#,
            ),
        ]);
        let mut readonly_initializer_engine = engine_from_sources(readonly_initializer);
        let error = readonly_initializer_engine
            .start("main", None)
            .expect_err("json mutation in initializer should fail");
        assert_eq!(error.code, "ENGINE_GLOBAL_READONLY");

        let defs_files = map(&[
            (
                "shared.defs.xml",
                r#"<defs name="shared"><var name="hp" type="int">7</var></defs>"#,
            ),
            (
                "main.script.xml",
                r#"
<!-- include: shared.defs.xml -->
<script name="main">
  <code>shared.hp = shared.hp + 1;</code>
  <text>${shared.hp}</text>
</script>
"#,
            ),
        ]);
        let mut defs_engine = engine_from_sources(defs_files.clone());
        defs_engine.start("main", None).expect("start");
        defs_engine.defs_globals_value.clear();
        let error = defs_engine
            .eval_expression("shared.hp")
            .expect_err("missing defs global should fail");
        assert_eq!(error.code, "ENGINE_DEFS_GLOBAL_MISSING");

        let mut invalid_visible_defs = engine_from_sources(defs_files.clone());
        invalid_visible_defs.start("main", None).expect("start");
        invalid_visible_defs.visible_defs_by_script.insert(
            "main".to_string(),
            BTreeSet::from(["bad".to_string()]),
        );
        invalid_visible_defs.defs_global_alias_by_script.insert(
            "main".to_string(),
            BTreeMap::from([("hp".to_string(), "bad".to_string())]),
        );
        invalid_visible_defs.defs_globals_value.clear();
        let error = invalid_visible_defs
            .eval_expression("1")
            .expect_err("invalid alias target should fail");
        assert_eq!(error.code, "ENGINE_DEFS_GLOBAL_MISSING");

        let json_shadow = map(&[
            ("game.json", r#"{ "hp": 5 }"#),
            (
                "main.script.xml",
                r#"
<!-- include: game.json -->
<script name="main">
  <var name="game" type="int">1</var>
  <code>game = game + 1;</code>
  <text>${game}</text>
</script>
"#,
            ),
        ]);
        let mut json_shadow_engine = engine_from_sources(json_shadow);
        json_shadow_engine.start("main", None).expect("start");
        let output = json_shadow_engine.next_output().expect("text");
        assert!(matches!(output, EngineOutput::Text { text, .. } if text == "2"));

        let namespace_type_error = map(&[
            (
                "shared.defs.xml",
                r#"<defs name="shared"><var name="hp" type="int">7</var></defs>"#,
            ),
            (
                "main.script.xml",
                r#"
<!-- include: shared.defs.xml -->
<script name="main">
  <code>__sl_defs_ns_shared = 1;</code>
</script>
"#,
            ),
        ]);
        let mut namespace_type_error_engine = engine_from_sources(namespace_type_error);
        namespace_type_error_engine.start("main", None).expect("start");
        let error = namespace_type_error_engine
            .next_output()
            .expect_err("namespace type should fail");
        assert_eq!(error.code, "ENGINE_DEFS_GLOBAL_NAMESPACE_TYPE");

        let namespace_extra_field = map(&[
            (
                "shared.defs.xml",
                r#"<defs name="shared"><var name="hp" type="int">7</var></defs>"#,
            ),
            (
                "main.script.xml",
                r#"
<!-- include: shared.defs.xml -->
<script name="main">
  <code>__sl_defs_ns_shared.extra = 1;</code>
  <text>${shared.hp}</text>
</script>
"#,
            ),
        ]);
        let mut namespace_extra_engine = engine_from_sources(namespace_extra_field);
        namespace_extra_engine.start("main", None).expect("start");
        let text = namespace_extra_engine.next_output().expect("text");
        assert!(matches!(text, EngineOutput::Text { text, .. } if text == "7"));

        let mut missing_decl_engine = engine_from_sources(defs_files.clone());
        missing_decl_engine.start("main", None).expect("start");
        missing_decl_engine.defs_globals_type.clear();
        let error = missing_decl_engine
            .next_output()
            .expect_err("missing type declaration should fail");
        assert_eq!(error.code, "ENGINE_DEFS_GLOBAL_DECL_MISSING");

        let full_alias_type_mismatch = map(&[
            (
                "shared.defs.xml",
                r#"<defs name="shared"><var name="hp" type="int">7</var></defs>"#,
            ),
            (
                "main.script.xml",
                r#"
<!-- include: shared.defs.xml -->
<script name="main">
  <code>shared.hp = "bad";</code>
</script>
"#,
            ),
        ]);
        let mut mismatch_full_engine = engine_from_sources(full_alias_type_mismatch);
        mismatch_full_engine.start("main", None).expect("start");
        let error = mismatch_full_engine
            .next_output()
            .expect_err("full-name type mismatch should fail");
        assert_eq!(error.code, "ENGINE_TYPE_MISMATCH");

        let short_alias_files = map(&[
            (
                "shared.defs.xml",
                r#"<defs name="shared"><var name="hp" type="int">7</var></defs>"#,
            ),
            (
                "main.script.xml",
                r#"
<!-- include: shared.defs.xml -->
<script name="main">
  <code>hp = hp + 1;</code>
  <text>${shared.hp}</text>
</script>
"#,
            ),
        ]);
        let mut missing_short_decl_engine = engine_from_sources(short_alias_files.clone());
        missing_short_decl_engine.start("main", None).expect("start");
        missing_short_decl_engine.defs_globals_type.clear();
        let error = missing_short_decl_engine
            .next_output()
            .expect_err("missing short alias decl should fail");
        assert_eq!(error.code, "ENGINE_DEFS_GLOBAL_DECL_MISSING");

        let short_alias_type_mismatch = map(&[
            (
                "shared.defs.xml",
                r#"<defs name="shared"><var name="hp" type="int">7</var></defs>"#,
            ),
            (
                "main.script.xml",
                r#"
<!-- include: shared.defs.xml -->
<script name="main">
  <code>hp = "bad";</code>
</script>
"#,
            ),
        ]);
        let mut short_alias_type_mismatch_engine = engine_from_sources(short_alias_type_mismatch);
        short_alias_type_mismatch_engine
            .start("main", None)
            .expect("start");
        let error = short_alias_type_mismatch_engine
            .next_output()
            .expect_err("short alias type mismatch should fail");
        assert_eq!(error.code, "ENGINE_TYPE_MISMATCH");

        let mut map_helpers_engine = engine_from_sources(defs_files);
        map_helpers_engine.start("main", None).expect("start");
        assert!(map_helpers_engine
            .build_defs_global_qualified_rewrite_map("missing")
            .is_empty());
        map_helpers_engine.visible_defs_by_script.insert(
            "main".to_string(),
            BTreeSet::from(["bad".to_string()]),
        );
        let rewritten = map_helpers_engine.build_defs_global_qualified_rewrite_map("main");
        assert!(rewritten.is_empty());

        map_helpers_engine.defs_global_declarations.insert(
            "other.hp".to_string(),
            sl_core::DefsGlobalVarDecl {
                namespace: "other".to_string(),
                name: "hp".to_string(),
                qualified_name: "other.hp".to_string(),
                r#type: ScriptType::Primitive {
                    name: "int".to_string(),
                },
                initial_value_expr: None,
                location: sl_core::SourceSpan::synthetic(),
            },
        );
        let aliases = map_helpers_engine.collect_bundle_defs_short_aliases();
        assert!(!aliases.contains_key("hp"));

        let mut invalid_initializer_engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>ok</text></script>"#,
        )]));
        invalid_initializer_engine.defs_global_declarations = BTreeMap::from([
            (
                "bad".to_string(),
                sl_core::DefsGlobalVarDecl {
                    namespace: "shared".to_string(),
                    name: "bad".to_string(),
                    qualified_name: "bad".to_string(),
                    r#type: ScriptType::Primitive {
                        name: "int".to_string(),
                    },
                    initial_value_expr: None,
                    location: sl_core::SourceSpan::synthetic(),
                },
            ),
            (
                "shared.ok".to_string(),
                sl_core::DefsGlobalVarDecl {
                    namespace: "shared".to_string(),
                    name: "ok".to_string(),
                    qualified_name: "shared.ok".to_string(),
                    r#type: ScriptType::Primitive {
                        name: "int".to_string(),
                    },
                    initial_value_expr: Some("1".to_string()),
                    location: sl_core::SourceSpan::synthetic(),
                },
            ),
        ]);
        invalid_initializer_engine.defs_global_init_order =
            vec!["bad".to_string(), "shared.ok".to_string()];
        invalid_initializer_engine.start("main", None).expect("start");

        let mut alias_visibility_engine = engine_from_sources(map(&[
            (
                "shared.defs.xml",
                r#"<defs name="shared"><var name="hp" type="int">1</var></defs>"#,
            ),
            (
                "main.script.xml",
                r#"
<!-- include: shared.defs.xml -->
<script name="main"><text>ok</text></script>
"#,
            ),
        ]));
        alias_visibility_engine.start("main", None).expect("start");
        alias_visibility_engine.defs_global_alias_by_script.insert(
            "main".to_string(),
            BTreeMap::from([
                ("ghost".to_string(), "ghost.hp".to_string()),
                ("hp".to_string(), "shared.hp".to_string()),
            ]),
        );
        let _ = alias_visibility_engine
            .execute_rhai("hp + 1", true)
            .expect("eval should pass");

        let mut short_decl_missing_engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>ok</text></script>"#,
        )]));
        short_decl_missing_engine.start("main", None).expect("start");
        short_decl_missing_engine.visible_defs_by_script.insert(
            "main".to_string(),
            BTreeSet::from(["bad".to_string()]),
        );
        short_decl_missing_engine.defs_global_alias_by_script.insert(
            "main".to_string(),
            BTreeMap::from([("hp".to_string(), "bad".to_string())]),
        );
        short_decl_missing_engine
            .defs_globals_value
            .insert("bad".to_string(), SlValue::Number(1.0));
        let error = short_decl_missing_engine
            .execute_rhai("hp = hp + 1;", false)
            .expect_err("short alias missing decl should fail");
        assert_eq!(error.code, "ENGINE_DEFS_GLOBAL_DECL_MISSING");
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
        let lookup = engine
            .group_lookup
            .get_mut(&key)
            .expect("group lookup entry should exist");
        lookup.script_name = "missing".to_string();
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
        let lookup = engine
            .group_lookup
            .get_mut(&key)
            .expect("group lookup entry should exist");
        lookup.group_id = "missing-group".to_string();
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

    #[test]
    fn code_eval_with_defs_prelude_and_visible_json_is_covered() {
        let mut engine = engine_from_sources(map(&[
            ("game.json", r#"{ "bonus": 10 }"#),
            (
                "shared.defs.xml",
                r#"
<defs name="shared">
  <function name="add_bonus" args="int:x" return="int:out">
    out = x + game.bonus;
  </function>
</defs>
"#,
            ),
            (
                "main.script.xml",
                r#"
<!-- include: shared.defs.xml -->
<!-- include: game.json -->
<script name="main">
  <var name="hp" type="int">1</var>
  <code>hp = shared.add_bonus(hp);</code>
  <text>${hp}</text>
</script>
"#,
            ),
        ]));

        engine.start("main", None).expect("start");
        let output = engine.next_output().expect("text");
        assert!(matches!(output, EngineOutput::Text { text, .. } if text == "11"));
    }

}
