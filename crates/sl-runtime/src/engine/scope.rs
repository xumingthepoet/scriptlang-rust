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

    fn read_variable(&self, name: &str) -> Result<SlValue, ScriptLangError> {
        for frame in self.frames.iter().rev() {
            if let Some(value) = frame.scope.get(name) {
                return Ok(value.clone());
            }
        }

        let script_name = self.resolve_current_script_name();
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

        let mut current = self.read_variable(&parts[0])?;
        for part in parts.iter().skip(1) {
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

        let head = &parts[0];
        let mut root_value = self.read_variable(head)?;
        assign_nested_path(&mut root_value, &parts[1..], value).map_err(|message| {
            ScriptLangError::new(
                "ENGINE_REF_PATH_WRITE",
                format!("Cannot resolve write path \"{}\": {}", path, message),
            )
        })?;
        self.write_variable(head, root_value)
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

}
