impl ScriptLangEngine {
    fn resolve_current_script_name(&self) -> Option<String> {
        let top = self.frames.last()?;
        self.group_lookup
            .get(&top.group_id)
            .map(|entry| entry.script_name.clone())
    }

    fn is_visible_json_global(&self, script_name: Option<&str>, name: &str) -> bool {
        let Some(script_name) = script_name else {
            return false;
        };
        let Some(visible) = self.visible_json_by_script.get(script_name) else {
            return false;
        };
        visible.contains(name) && self.global_json.contains_key(name)
    }

    fn resolve_defs_global_alias(
        &self,
        script_name: Option<&str>,
        alias: &str,
    ) -> Option<String> {
        let script_name = script_name?;
        self.defs_global_alias_by_script
            .get(script_name)
            .and_then(|aliases| aliases.get(alias))
            .cloned()
    }

    fn read_variable(&self, name: &str) -> Result<SlValue, ScriptLangError> {
        for frame in self.frames.iter().rev() {
            if let Some(value) = frame.scope.get(name) {
                return Ok(value.clone());
            }
        }

        let script_name = self.resolve_current_script_name();
        if let Some(qualified_name) = self.resolve_defs_global_alias(script_name.as_deref(), name)
        {
            return self
                .defs_globals_value
                .get(&qualified_name)
                .cloned()
                .ok_or_else(|| {
                    ScriptLangError::new(
                        "ENGINE_DEFS_GLOBAL_MISSING",
                        format!("Defs global \"{}\" is not initialized.", qualified_name),
                    )
                });
        }
        if self.is_visible_json_global(script_name.as_deref(), name) {
            let value = self
                .global_json
                .get(name)
                .expect("visible global lookup should be present");
            return Ok(value.clone());
        }

        Err(ScriptLangError::new(
            "ENGINE_VAR_READ",
            format!("Variable \"{}\" is not defined.", name),
        ))
    }

    fn write_variable(&mut self, name: &str, value: SlValue) -> Result<(), ScriptLangError> {
        for frame in self.frames.iter_mut().rev() {
            if frame.scope.contains_key(name) {
                if let Some(declared_type) = frame.var_types.get(name) {
                    if !is_type_compatible(&value, declared_type) {
                        return Err(ScriptLangError::new(
                            "ENGINE_TYPE_MISMATCH",
                            format!("Variable \"{}\" does not match declared type.", name),
                        ));
                    }
                }
                frame.scope.insert(name.to_string(), value);
                return Ok(());
            }
        }

        let script_name = self.resolve_current_script_name();
        if let Some(qualified_name) = self.resolve_defs_global_alias(script_name.as_deref(), name)
        {
            if let Some(declared_type) = self.defs_globals_type.get(&qualified_name) {
                if !is_type_compatible(&value, declared_type) {
                    return Err(ScriptLangError::new(
                        "ENGINE_TYPE_MISMATCH",
                        format!("Variable \"{}\" does not match declared type.", name),
                    ));
                }
            }
            self.defs_globals_value.insert(qualified_name, value);
            return Ok(());
        }
        if self.is_visible_json_global(script_name.as_deref(), name) {
            return Err(ScriptLangError::new(
                "ENGINE_GLOBAL_READONLY",
                format!(
                    "Global JSON \"{}\" is readonly and cannot be mutated.",
                    name
                ),
            ));
        }

        Err(ScriptLangError::new(
            "ENGINE_VAR_WRITE",
            format!("Variable \"{}\" is not defined.", name),
        ))
    }

    fn read_path(&self, path: &str) -> Result<SlValue, ScriptLangError> {
        let parts = parse_ref_path(path);
        if parts.is_empty() {
            return Err(ScriptLangError::new(
                "ENGINE_REF_PATH",
                format!("Invalid ref path \"{}\".", path),
            ));
        }

        let (root_name, nested_start_index) = self.resolve_path_root_alias(&parts);
        let mut current = self.read_variable(&root_name)?;
        for part in parts.iter().skip(nested_start_index) {
            let SlValue::Map(entries) = current else {
                return Err(ScriptLangError::new(
                    "ENGINE_REF_PATH_READ",
                    format!("Cannot resolve path \"{}\".", path),
                ));
            };
            current = entries.get(part).cloned().ok_or_else(|| {
                ScriptLangError::new(
                    "ENGINE_REF_PATH_READ",
                    format!("Cannot resolve path \"{}\".", path),
                )
            })?;
        }

        Ok(current)
    }

    fn write_path(&mut self, path: &str, value: SlValue) -> Result<(), ScriptLangError> {
        let parts = parse_ref_path(path);
        if parts.is_empty() {
            return Err(ScriptLangError::new(
                "ENGINE_REF_PATH",
                format!("Invalid ref path \"{}\".", path),
            ));
        }

        if parts.len() == 1 {
            return self.write_variable(&parts[0], value);
        }

        let (root_name, nested_start_index) = self.resolve_path_root_alias(&parts);
        if nested_start_index >= parts.len() {
            return self.write_variable(&root_name, value);
        }
        let mut root_value = self.read_variable(&root_name)?;
        assign_nested_path(&mut root_value, &parts[nested_start_index..], value).map_err(
            |message| {
            ScriptLangError::new(
                "ENGINE_REF_PATH_WRITE",
                format!("Cannot resolve write path \"{}\": {}", path, message),
            )
        },
        )?;
        self.write_variable(&root_name, root_value)
    }

    fn resolve_path_root_alias(&self, parts: &[String]) -> (String, usize) {
        if parts.len() >= 2 {
            let script_name = self.resolve_current_script_name();
            let qualified = format!("{}.{}", parts[0], parts[1]);
            if self
                .resolve_defs_global_alias(script_name.as_deref(), &qualified)
                .is_some()
            {
                return (qualified, 2);
            }
        }

        (parts[0].clone(), 1)
    }

}

#[cfg(test)]
mod scope_tests {
    use super::*;
    use super::runtime_test_support::*;

    #[test]
    fn runtime_errors_cover_var_and_ref_path_failures() {
        let mut duplicate_var = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <var name="x" type="int">1</var>
      <var name="x" type="int">2</var>
    </script>
    "#,
        )]));
        duplicate_var.start("main", None).expect("start");
        let error = duplicate_var
            .next_output()
            .expect_err("duplicate var should fail");
        assert_eq!(error.code, "ENGINE_VAR_DUPLICATE");
    
        let mut bad_type = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><var name="x" type="int">&quot;str&quot;</var></script>"#,
        )]));
        bad_type.start("main", None).expect("start");
        let error = bad_type
            .next_output()
            .expect_err("type mismatch should fail");
        assert_eq!(error.code, "ENGINE_TYPE_MISMATCH");
    
        let mut bad_ref_read = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>${missing.value}</text></script>"#,
        )]));
        bad_ref_read.start("main", None).expect("start");
        let error = bad_ref_read
            .next_output()
            .expect_err("missing ref path should fail");
        assert!(
            error.code == "ENGINE_VAR_READ"
                || error.code == "ENGINE_REF_PATH_READ"
                || error.code == "ENGINE_EVAL_ERROR"
        );
    
        let mut bad_ref_write = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <var name="x" type="int">1</var>
      <code>x.value = 1;</code>
    </script>
    "#,
        )]));
        bad_ref_write.start("main", None).expect("start");
        let error = bad_ref_write
            .next_output()
            .expect_err("write path should fail");
        assert!(error.code == "ENGINE_REF_PATH_WRITE" || error.code == "ENGINE_EVAL_ERROR");
    }

    #[test]
    fn group_scope_allows_redeclaring_same_var_name_in_separate_groups() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
<script name="main">
  <group>
    <var name="same" type="int">1</var>
    <text>${same}</text>
  </group>
  <group>
    <var name="same" type="int">2</var>
    <text>${same}</text>
  </group>
</script>
"#,
        )]));

        engine.start("main", None).expect("start");
        let first = engine.next_output().expect("first text");
        assert!(matches!(first, EngineOutput::Text { text, .. } if text == "1"));
        let second = engine.next_output().expect("second text");
        assert!(matches!(second, EngineOutput::Text { text, .. } if text == "2"));
        let end = engine.next_output().expect("end");
        assert!(matches!(end, EngineOutput::End));
    }

    #[test]
    fn call_and_scope_validation_error_paths_are_covered() {
        // create_script_root_scope unknown arg / type mismatch
        let engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main" args="int:x"><text>${x}</text></script>"#,
        )]));
        let error = engine
            .create_script_root_scope(
                "main",
                BTreeMap::from([("unknown".to_string(), SlValue::Number(1.0))]),
            )
            .expect_err("unknown arg should fail");
        assert_eq!(error.code, "ENGINE_CALL_ARG_UNKNOWN");
    
        let error = engine
            .create_script_root_scope(
                "main",
                BTreeMap::from([("x".to_string(), SlValue::String("bad".to_string()))]),
            )
            .expect_err("type mismatch should fail");
        assert_eq!(error.code, "ENGINE_TYPE_MISMATCH");
    
        // execute_call arg unknown
        let mut engine = engine_from_sources(map(&[
            (
                "main.script.xml",
                r#"
    <!-- include: callee.script.xml -->
    <script name="main">
      <call script="callee" args="1"/>
    </script>
    "#,
            ),
            (
                "callee.script.xml",
                r#"<script name="callee"><return/></script>"#,
            ),
        ]));
        engine.start("main", None).expect("start");
        let error = engine.next_output().expect_err("arg unknown should fail");
        assert_eq!(error.code, "ENGINE_CALL_ARG_UNKNOWN");
    
        // execute_call arg type mismatch at scope creation
        let mut engine = engine_from_sources(map(&[
            (
                "main.script.xml",
                r#"
    <!-- include: callee.script.xml -->
    <script name="main">
      <call script="callee" args="&quot;str&quot;"/>
    </script>
    "#,
            ),
            (
                "callee.script.xml",
                r#"<script name="callee" args="int:x"><return/></script>"#,
            ),
        ]));
        engine.start("main", None).expect("start");
        let error = engine
            .next_output()
            .expect_err("arg type mismatch should fail");
        assert_eq!(error.code, "ENGINE_TYPE_MISMATCH");
    }

    #[test]
    fn defs_globals_support_shadowing_short_name_and_qualified_paths() {
        let mut engine = engine_from_sources(map(&[
            (
                "shared.defs.xml",
                r#"
<defs name="shared">
  <var name="hp" type="int">100</var>
</defs>
"#,
            ),
            (
                "main.script.xml",
                r#"
<!-- include: shared.defs.xml -->
<script name="main">
  <var name="hp" type="int">1</var>
  <code>hp = hp + 1; shared.hp = shared.hp + 5;</code>
  <text>${hp}</text>
  <text>${shared.hp}</text>
</script>
"#,
            ),
        ]));

        engine.start("main", None).expect("start");
        let first = engine.next_output().expect("first text");
        assert!(matches!(first, EngineOutput::Text { text, .. } if text == "2"));
        let second = engine.next_output().expect("second text");
        assert!(matches!(second, EngineOutput::Text { text, .. } if text == "105"));
        assert_eq!(
            engine.read_path("shared.hp").expect("qualified read"),
            SlValue::Number(105.0)
        );

        engine
            .write_path("shared.hp", SlValue::Number(110.0))
            .expect("qualified write");
        assert_eq!(
            engine.read_variable("shared.hp").expect("qualified variable read"),
            SlValue::Number(110.0)
        );
    }

    #[test]
    fn defs_global_short_alias_writes_back_when_not_shadowed() {
        let mut engine = engine_from_sources(map(&[
            (
                "shared.defs.xml",
                r#"
<defs name="shared">
  <var name="hp" type="int">7</var>
</defs>
"#,
            ),
            (
                "main.script.xml",
                r#"
<!-- include: shared.defs.xml -->
<script name="main">
  <code>hp = hp + 3;</code>
  <text>${shared.hp}</text>
</script>
"#,
            ),
        ]));

        engine.start("main", None).expect("start");
        let first = engine.next_output().expect("text");
        assert!(matches!(first, EngineOutput::Text { text, .. } if text == "10"));
        assert_eq!(
            engine.read_variable("hp").expect("short alias read"),
            SlValue::Number(10.0)
        );
    }

    #[test]
    fn defs_global_missing_and_type_mismatch_paths_are_covered() {
        let files = map(&[
            (
                "shared.defs.xml",
                r#"<defs name="shared"><var name="hp" type="int">7</var></defs>"#,
            ),
            (
                "main.script.xml",
                r#"
<!-- include: shared.defs.xml -->
<script name="main"><text>${shared.hp}</text></script>
"#,
            ),
        ]);

        let mut missing_engine = engine_from_sources(files.clone());
        missing_engine.start("main", None).expect("start");
        missing_engine.defs_globals_value.clear();
        let error = missing_engine
            .read_variable("shared.hp")
            .expect_err("missing defs global should fail");
        assert_eq!(error.code, "ENGINE_DEFS_GLOBAL_MISSING");

        let mut mismatch_engine = engine_from_sources(files);
        mismatch_engine.start("main", None).expect("start");
        let error = mismatch_engine
            .write_variable("shared.hp", SlValue::String("bad".to_string()))
            .expect_err("type mismatch should fail");
        assert_eq!(error.code, "ENGINE_TYPE_MISMATCH");
    }

    #[test]
    fn write_defs_global_variable_succeeds() {
        // Test successful write to defs global variable (covers scope.rs line 92)
        let files = map(&[(
            "shared.defs.xml",
            r#"<defs name="shared"><var name="score" type="int">0</var></defs>"#,
        ), (
            "main.script.xml",
            r#"<!-- include: shared.defs.xml -->
<script name="main">
  <code>shared.score = 100;</code>
  <text>${shared.score}</text>
</script>"#,
        )]);
        let mut engine = engine_from_sources(files);
        engine.start("main", None).expect("start");

        // Execute to get the text output
        let output = engine.next_output().expect("next");
        // Should output "100" from the text node
        assert!(matches!(output, EngineOutput::Text { ref text, .. } if text == "100"));
    }

}
