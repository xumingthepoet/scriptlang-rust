#[cfg(test)]
mod tests {
    use super::*;
    use sl_compiler::compile_project_bundle_from_xml_map;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn map(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    fn engine_from_sources(files: BTreeMap<String, String>) -> ScriptLangEngine {
        let compiled = compile_project_bundle_from_xml_map(&files).expect("compile should pass");
        ScriptLangEngine::new(ScriptLangEngineOptions {
            scripts: compiled.scripts,
            global_json: compiled.global_json,
            host_functions: None,
            random_seed: Some(1),
            compiler_version: None,
        })
        .expect("engine should build")
    }

    fn read_sources_recursive(
        root: &Path,
        current: &Path,
        out: &mut BTreeMap<String, String>,
    ) -> Result<(), std::io::Error> {
        for entry in fs::read_dir(current)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                read_sources_recursive(root, &path, out)?;
                continue;
            }
            let relative = path
                .strip_prefix(root)
                .expect("path should be under root")
                .to_string_lossy()
                .replace('\\', "/");
            out.insert(relative, fs::read_to_string(path)?);
        }
        Ok(())
    }

    fn sources_from_example_dir(name: &str) -> BTreeMap<String, String> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("examples")
            .join("scripts-rhai")
            .join(name);
        let mut files = BTreeMap::new();
        read_sources_recursive(&root, &root, &mut files).expect("example should load");
        files
    }

    fn drive_engine_to_end(engine: &mut ScriptLangEngine) {
        for _ in 0..5_000usize {
            match engine.next_output().expect("next should pass") {
                EngineOutput::Text { .. } => {}
                EngineOutput::Choices { items, .. } => {
                    let index = items.first().map(|item| item.index).unwrap_or(0);
                    engine.choose(index).expect("choose should pass");
                }
                EngineOutput::Input { .. } => {
                    engine.submit_input("").expect("input should pass");
                }
                EngineOutput::End => return,
            }
        }
    }

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
    fn snapshot_resume_choice_roundtrip() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
<script name="main">
  <choice text="Pick">
    <option text="A"><text>Alpha</text></option>
    <option text="B"><text>Beta</text></option>
  </choice>
</script>
"#,
        )]));
        engine.start("main", None).expect("start");

        let first = engine.next_output().expect("next");
        assert!(matches!(first, EngineOutput::Choices { .. }));
        let snapshot = engine.snapshot().expect("snapshot");

        let mut resumed = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
<script name="main">
  <choice text="Pick">
    <option text="A"><text>Alpha</text></option>
    <option text="B"><text>Beta</text></option>
  </choice>
</script>
"#,
        )]));
        resumed.resume(snapshot).expect("resume");
        resumed.choose(0).expect("choose");
        let next = resumed.next_output().expect("next");
        assert!(matches!(next, EngineOutput::Text { .. }));
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

    #[derive(Debug)]
    struct TestRegistry {
        names: Vec<String>,
    }

    impl HostFunctionRegistry for TestRegistry {
        fn call(&self, _name: &str, _args: &[SlValue]) -> Result<SlValue, ScriptLangError> {
            Ok(SlValue::Bool(true))
        }

        fn names(&self) -> &[String] {
            &self.names
        }
    }

    #[test]
    fn new_rejects_reserved_host_function_name_random() {
        let files = map(&[(
            "main.script.xml",
            r#"<script name="main"><text>Hello</text></script>"#,
        )]);
        let compiled = compile_project_bundle_from_xml_map(&files).expect("compile should pass");
        let result = ScriptLangEngine::new(ScriptLangEngineOptions {
            scripts: compiled.scripts,
            global_json: compiled.global_json,
            host_functions: Some(Arc::new(TestRegistry {
                names: vec!["random".to_string()],
            })),
            random_seed: Some(1),
            compiler_version: Some(DEFAULT_COMPILER_VERSION.to_string()),
        });
        assert!(result.is_err());
        let error = result.err().expect("reserved random name should fail");
        assert_eq!(error.code, "ENGINE_HOST_FUNCTION_RESERVED");
    }

    #[test]
    fn new_rejects_host_function_conflicting_with_defs_function() {
        let files = map(&[
            (
                "main.script.xml",
                r#"
<!-- include: shared.defs.xml -->
<script name="main"><text>Hello</text></script>
"#,
            ),
            (
                "shared.defs.xml",
                r#"
<defs name="shared">
  <function name="addWithGameBonus" args="int:a1,int:a2" return="int:out">
    out = a1 + a2;
  </function>
</defs>
"#,
            ),
        ]);
        let compiled = compile_project_bundle_from_xml_map(&files).expect("compile should pass");
        let result = ScriptLangEngine::new(ScriptLangEngineOptions {
            scripts: compiled.scripts,
            global_json: compiled.global_json,
            host_functions: Some(Arc::new(TestRegistry {
                names: vec!["addWithGameBonus".to_string()],
            })),
            random_seed: Some(1),
            compiler_version: Some(DEFAULT_COMPILER_VERSION.to_string()),
        });
        assert!(result.is_err());
        let error = result.err().expect("conflicting defs function should fail");
        assert_eq!(error.code, "ENGINE_HOST_FUNCTION_CONFLICT");
    }

    #[test]
    fn new_rejects_defs_function_symbol_conflict_after_normalization() {
        let files = map(&[
            (
                "main.script.xml",
                r#"
<!-- include: a.defs.xml -->
<!-- include: x.defs.xml -->
<script name="main"><text>Hello</text></script>
"#,
            ),
            (
                "a.defs.xml",
                r#"
<defs name="a">
  <function name="b" return="int:out">out = 1;</function>
</defs>
"#,
            ),
            (
                "x.defs.xml",
                r#"
<defs name="x">
  <function name="a_b" return="int:out">out = 2;</function>
</defs>
"#,
            ),
        ]);
        let compiled = compile_project_bundle_from_xml_map(&files).expect("compile should pass");
        let result = ScriptLangEngine::new(ScriptLangEngineOptions {
            scripts: compiled.scripts,
            global_json: compiled.global_json,
            host_functions: None,
            random_seed: Some(1),
            compiler_version: Some(DEFAULT_COMPILER_VERSION.to_string()),
        });
        let error = match result {
            Ok(_) => panic!("normalized symbol conflict should fail"),
            Err(error) => error,
        };
        assert_eq!(error.code, "ENGINE_DEFS_FUNCTION_SYMBOL_CONFLICT");
    }

    #[test]
    fn start_returns_error_for_missing_script() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>Hello</text></script>"#,
        )]));
        let error = engine
            .start("missing", None)
            .expect_err("unknown entry should fail");
        assert_eq!(error.code, "ENGINE_SCRIPT_NOT_FOUND");
    }

    #[test]
    fn choose_and_input_validate_pending_boundary_state() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>Hello</text></script>"#,
        )]));
        engine.start("main", None).expect("start");

        let choose_error = engine.choose(0).expect_err("no pending choice");
        assert_eq!(choose_error.code, "ENGINE_NO_PENDING_CHOICE");
        let input_error = engine.submit_input("x").expect_err("no pending input");
        assert_eq!(input_error.code, "ENGINE_NO_PENDING_INPUT");
    }

    #[test]
    fn choose_rejects_out_of_range_index() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
<script name="main">
  <choice text="Pick">
    <option text="A"><text>A</text></option>
  </choice>
</script>
"#,
        )]));
        engine.start("main", None).expect("start");
        let first = engine.next_output().expect("next");
        assert!(matches!(first, EngineOutput::Choices { .. }));
        let error = engine.choose(9).expect_err("index out of range");
        assert_eq!(error.code, "ENGINE_CHOICE_INDEX");
    }

    #[test]
    fn submit_input_uses_default_value_for_blank_input() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
<script name="main">
  <var name="heroName" type="string">&quot;Traveler&quot;</var>
  <input var="heroName" text="Name your hero"/>
  <text>Hello ${heroName}</text>
</script>
"#,
        )]));
        engine.start("main", None).expect("start");
        let first = engine.next_output().expect("next");
        assert!(matches!(first, EngineOutput::Input { .. }));
        engine.submit_input("   ").expect("submit input");
        let second = engine.next_output().expect("next");
        let mut text = String::new();
        if let EngineOutput::Text { text: output } = second {
            text = output;
        }
        assert_eq!(text, "Hello Traveler");
    }

    #[test]
    fn submit_input_uses_provided_non_empty_value() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
<script name="main">
  <var name="heroName" type="string">&quot;Traveler&quot;</var>
  <input var="heroName" text="Name your hero"/>
  <text>Hello ${heroName}</text>
</script>
"#,
        )]));
        engine.start("main", None).expect("start");
        let first = engine.next_output().expect("next");
        assert!(matches!(first, EngineOutput::Input { .. }));
        engine.submit_input("Guild").expect("submit input");
        let second = engine.next_output().expect("next");
        let mut text = String::new();
        if let EngineOutput::Text { text: output } = second {
            text = output;
        }
        assert_eq!(text, "Hello Guild");
    }

    #[test]
    fn snapshot_and_resume_cover_while_completion_and_once_state() {
        let files = map(&[(
            "main.script.xml",
            r#"
<script name="main">
  <var name="n" type="int">1</var>
  <text once="true">Intro</text>
  <while when="n > 0">
    <choice text="Pick">
      <option text="Stop"><code>n = 0;</code></option>
    </choice>
  </while>
</script>
"#,
        )]);

        let mut engine = engine_from_sources(files.clone());
        engine.start("main", None).expect("start");
        assert!(matches!(
            engine.next_output().expect("text"),
            EngineOutput::Text { .. }
        ));
        assert!(matches!(
            engine.next_output().expect("choice"),
            EngineOutput::Choices { .. }
        ));
        let snapshot = engine.snapshot().expect("snapshot");
        assert!(!snapshot.once_state_by_script.is_empty());

        let mut resumed = engine_from_sources(files);
        resumed.resume(snapshot).expect("resume");
        resumed.choose(0).expect("choose should pass");
        assert!(matches!(
            resumed.next_output().expect("end"),
            EngineOutput::End
        ));
    }

    #[test]
    fn resume_restores_pending_input_boundary() {
        let files = map(&[(
            "main.script.xml",
            r#"
<script name="main">
  <var name="heroName" type="string">&quot;Traveler&quot;</var>
  <input var="heroName" text="Name your hero"/>
  <text>Hello ${heroName}</text>
</script>
"#,
        )]);

        let mut engine = engine_from_sources(files.clone());
        engine.start("main", None).expect("start");
        assert!(matches!(
            engine.next_output().expect("input"),
            EngineOutput::Input { .. }
        ));
        let snapshot = engine.snapshot().expect("snapshot");

        let mut resumed = engine_from_sources(files);
        resumed.resume(snapshot).expect("resume");
        assert!(matches!(
            resumed.next_output().expect("input"),
            EngineOutput::Input { .. }
        ));
        resumed.submit_input("Guild").expect("submit input");
        assert!(matches!(
            resumed.next_output().expect("text"),
            EngineOutput::Text { .. }
        ));
    }

    #[test]
    fn snapshot_requires_pending_boundary() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>Hello</text></script>"#,
        )]));
        engine.start("main", None).expect("start");
        let error = engine.snapshot().expect_err("snapshot should fail");
        assert_eq!(error.code, "SNAPSHOT_NOT_ALLOWED");
    }

    #[test]
    fn resume_validates_schema_and_compiler_version() {
        let sources = map(&[(
            "main.script.xml",
            r#"
<script name="main">
  <choice text="Pick">
    <option text="A"><text>A</text></option>
  </choice>
</script>
"#,
        )]);

        let mut base = engine_from_sources(sources.clone());
        base.start("main", None).expect("start");
        let first = base.next_output().expect("next");
        assert!(matches!(first, EngineOutput::Choices { .. }));
        let snapshot = base.snapshot().expect("snapshot");

        let mut schema_mismatch = engine_from_sources(sources.clone());
        let mut bad_schema = snapshot.clone();
        bad_schema.schema_version = "snapshot.bad".to_string();
        let error = schema_mismatch
            .resume(bad_schema)
            .expect_err("schema mismatch should fail");
        assert_eq!(error.code, "SNAPSHOT_SCHEMA");

        let mut compiler_mismatch = engine_from_sources(sources);
        let mut bad_compiler = snapshot;
        bad_compiler.compiler_version = "player.bad".to_string();
        let error = compiler_mismatch
            .resume(bad_compiler)
            .expect_err("compiler mismatch should fail");
        assert_eq!(error.code, "SNAPSHOT_COMPILER_VERSION");
    }

    #[test]
    fn resume_rejects_pending_boundary_node_mismatch() {
        let sources = map(&[(
            "main.script.xml",
            r#"
<script name="main">
  <choice text="Pick">
    <option text="A"><text>A</text></option>
  </choice>
</script>
"#,
        )]);
        let mut engine = engine_from_sources(sources.clone());
        engine.start("main", None).expect("start");
        let first = engine.next_output().expect("next");
        assert!(matches!(first, EngineOutput::Choices { .. }));
        let mut snapshot = engine.snapshot().expect("snapshot");
        if let PendingBoundaryV3::Choice { node_id, .. } = &mut snapshot.pending_boundary {
            *node_id = "invalid-node-id".to_string();
        }

        let mut resumed = engine_from_sources(sources);
        let error = resumed
            .resume(snapshot)
            .expect_err("pending choice node mismatch");
        assert_eq!(error.code, "SNAPSHOT_PENDING_BOUNDARY");
    }

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
    fn public_state_accessors_and_empty_registry_are_covered() {
        let registry = EmptyHostFunctionRegistry::default();
        assert!(registry.names().is_empty());
        let call_error = registry
            .call("noop", &[])
            .expect_err("empty registry call should fail");
        assert_eq!(call_error.code, "ENGINE_HOST_FUNCTION_MISSING");

        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
<script name="main">
  <choice text="Pick">
    <option text="A"><text>A</text></option>
  </choice>
</script>
"#,
        )]));
        assert_eq!(engine.compiler_version(), DEFAULT_COMPILER_VERSION);
        assert!(!engine.waiting_choice());
        engine.start("main", None).expect("start");
        let next = engine.next_output().expect("next");
        assert!(matches!(next, EngineOutput::Choices { .. }));
        assert!(engine.waiting_choice());
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
    fn runtime_errors_cover_break_continue_and_return_args() {
        let mut source = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
<script name="main">
  <choice text="Pick">
    <option text="A"><text>A</text></option>
  </choice>
</script>
"#,
        )]));
        source.start("main", None).expect("start");
        let _ = source.next_output().expect("choice");
        let mut snapshot = source.snapshot().expect("snapshot");
        if let Some(frame) = snapshot.runtime_frames.last_mut() {
            frame.group_id = "missing-group".to_string();
        }
        let mut resumed = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
<script name="main">
  <choice text="Pick">
    <option text="A"><text>A</text></option>
  </choice>
</script>
"#,
        )]));
        let error = resumed
            .resume(snapshot)
            .expect_err("missing group in snapshot should fail");
        assert_eq!(error.code, "ENGINE_GROUP_NOT_FOUND");

        let mut return_arg_unknown = engine_from_sources(map(&[
            (
                "main.script.xml",
                r#"<script name="main"><return script="next" args="1,2"/></script>"#,
            ),
            (
                "next.script.xml",
                r#"<script name="next" args="int:x"><text>${x}</text></script>"#,
            ),
        ]));
        return_arg_unknown.start("main", None).expect("start");
        let error = return_arg_unknown
            .next_output()
            .expect_err("extra return arg should fail");
        assert_eq!(error.code, "ENGINE_RETURN_ARG_UNKNOWN");
    }

    #[test]
    fn runtime_errors_cover_snapshot_shape_mismatches() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
<script name="main">
  <choice text="Pick">
    <option text="A"><text>A</text></option>
  </choice>
</script>
"#,
        )]));
        engine.start("main", None).expect("start");
        let _ = engine.next_output().expect("choice");
        let mut snapshot = engine.snapshot().expect("snapshot");

        snapshot.runtime_frames.clear();
        let error = engine
            .resume(snapshot.clone())
            .expect_err("empty runtime frames should fail");
        assert_eq!(error.code, "SNAPSHOT_EMPTY");

        let mut fresh = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
<script name="main">
  <choice text="Pick">
    <option text="A"><text>A</text></option>
  </choice>
</script>
"#,
        )]));
        fresh.start("main", None).expect("start fresh");
        let _ = fresh.next_output().expect("choice fresh");
        let mut bad_index = fresh.snapshot().expect("snapshot again");
        if let Some(frame) = bad_index.runtime_frames.last_mut() {
            frame.node_index = 9999;
        }
        let error = fresh
            .resume(bad_index)
            .expect_err("invalid pending node index should fail");
        assert_eq!(error.code, "SNAPSHOT_PENDING_BOUNDARY");
    }

    #[test]
    fn internal_state_error_paths_are_covered() {
        let mut engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
<script name="main">
  <choice text="Pick">
    <option text="A"><text>A</text></option>
  </choice>
</script>
"#,
        )]));

        engine.start("main", None).expect("start");
        let first = engine.next_output().expect("choice");
        assert!(matches!(first, EngineOutput::Choices { .. }));
        let mut items = Vec::new();
        let mut prompt_text = None;
        if let Some(PendingBoundary::Choice {
            options,
            prompt_text: choice_prompt,
            ..
        }) = engine.pending_boundary.clone()
        {
            items = options;
            prompt_text = choice_prompt;
        }
        assert!(!items.is_empty());
        let frame_id = engine.frames.last().expect("frame").frame_id;
        engine.pending_boundary = Some(PendingBoundary::Choice {
            frame_id,
            node_id: "x".to_string(),
            options: items.clone(),
            prompt_text: prompt_text.clone(),
        });
        let again = engine.next_output().expect("pending boundary should echo");
        assert!(matches!(again, EngineOutput::Choices { .. }));

        engine.pending_boundary = None;
        engine.ended = true;
        let end = engine.next_output().expect("ended should return end");
        assert!(matches!(end, EngineOutput::End));

        engine.pending_boundary = Some(PendingBoundary::Choice {
            frame_id: 999_999,
            node_id: "x".to_string(),
            options: items.clone(),
            prompt_text,
        });
        let error = engine.choose(0).expect_err("missing frame should fail");
        assert_eq!(error.code, "ENGINE_CHOICE_FRAME_MISSING");

        engine.pending_boundary = Some(PendingBoundary::Input {
            frame_id: 999_999,
            node_id: "x".to_string(),
            target_var: "name".to_string(),
            prompt_text: "p".to_string(),
            default_text: "d".to_string(),
        });
        let error = engine
            .submit_input("abc")
            .expect_err("missing input frame should fail");
        assert_eq!(error.code, "ENGINE_INPUT_FRAME_MISSING");

        let mut helper_engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"<script name="main"><text>Hello</text></script>"#,
        )]));
        let err = helper_engine.top_frame_id().expect_err("no frame");
        assert_eq!(err.code, "ENGINE_NO_FRAME");
        let err = helper_engine
            .bump_top_node_index(1)
            .expect_err("no frame for bump");
        assert_eq!(err.code, "ENGINE_NO_FRAME");

        let err = helper_engine
            .push_group_frame("missing-group", CompletionKind::ResumeAfterChild)
            .expect_err("missing group");
        assert_eq!(err.code, "ENGINE_GROUP_NOT_FOUND");

        assert!(helper_engine.finish_frame(123).is_ok());
        let err = helper_engine
            .find_current_root_frame_index()
            .expect_err("no root frame");
        assert_eq!(err.code, "ENGINE_ROOT_FRAME");
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
    fn resume_and_boundary_shape_paths_are_covered() {
        let mut input_engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
<script name="main">
  <var name="name" type="string">&quot;X&quot;</var>
  <input var="name" text="name?"/>
</script>
"#,
        )]));
        input_engine.start("main", None).expect("start");
        let input = input_engine.next_output().expect("input boundary");
        assert!(matches!(input, EngineOutput::Input { .. }));
        let input_snapshot = input_engine.snapshot().expect("snapshot");

        let mut choice_on_input = input_snapshot.clone();
        if let PendingBoundaryV3::Input { node_id, .. } = &choice_on_input.pending_boundary {
            choice_on_input.pending_boundary = PendingBoundaryV3::Choice {
                node_id: node_id.clone(),
                items: Vec::new(),
                prompt_text: None,
            };
        }
        let mut resume_choice = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
<script name="main">
  <var name="name" type="string">&quot;X&quot;</var>
  <input var="name" text="name?"/>
</script>
"#,
        )]));
        let error = resume_choice
            .resume(choice_on_input)
            .expect_err("choice on input node should fail");
        assert_eq!(error.code, "SNAPSHOT_PENDING_BOUNDARY");

        let mut choice_engine = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
<script name="main">
  <choice text="Pick">
    <option text="A"><text>A</text></option>
  </choice>
</script>
"#,
        )]));
        choice_engine.start("main", None).expect("start");
        let _ = choice_engine.next_output().expect("choice");
        let choice_snapshot = choice_engine.snapshot().expect("snapshot");

        let mut input_on_choice = choice_snapshot.clone();
        if let PendingBoundaryV3::Choice { node_id, .. } = &input_on_choice.pending_boundary {
            input_on_choice.pending_boundary = PendingBoundaryV3::Input {
                node_id: node_id.clone(),
                target_var: "name".to_string(),
                prompt_text: "p".to_string(),
                default_text: "d".to_string(),
            };
        }
        let mut resume_input = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
<script name="main">
  <choice text="Pick">
    <option text="A"><text>A</text></option>
  </choice>
</script>
"#,
        )]));
        let error = resume_input
            .resume(input_on_choice)
            .expect_err("input on choice node should fail");
        assert_eq!(error.code, "SNAPSHOT_PENDING_BOUNDARY");

        let mut input_mismatch = input_snapshot.clone();
        if let PendingBoundaryV3::Input { node_id, .. } = &mut input_mismatch.pending_boundary {
            *node_id = "missing-input-node".to_string();
        }
        let mut resume_mismatch = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
<script name="main">
  <var name="name" type="string">&quot;X&quot;</var>
  <input var="name" text="name?"/>
</script>
"#,
        )]));
        let error = resume_mismatch
            .resume(input_mismatch)
            .expect_err("input node mismatch should fail");
        assert_eq!(error.code, "SNAPSHOT_PENDING_BOUNDARY");

        let pending = PendingBoundary::Input {
            frame_id: 1,
            node_id: "n".to_string(),
            target_var: "name".to_string(),
            prompt_text: "p".to_string(),
            default_text: "d".to_string(),
        };
        let output = resume_mismatch.boundary_output(&pending);
        assert!(matches!(output, EngineOutput::Input { .. }));

        let mut with_resume = choice_snapshot.clone();
        if let Some(frame) = with_resume.runtime_frames.last_mut() {
            frame.completion = SnapshotCompletion::ResumeAfterChild;
        }
        let mut resumed = engine_from_sources(map(&[(
            "main.script.xml",
            r#"
<script name="main">
  <choice text="Pick">
    <option text="A"><text>A</text></option>
  </choice>
</script>
"#,
        )]));
        resumed
            .resume(with_resume)
            .expect("resume after child completion should work");
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
