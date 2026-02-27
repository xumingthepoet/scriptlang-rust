use super::runtime_test_support::*;
use super::*;

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
        defs_global_declarations: compiled.defs_global_declarations,
        defs_global_init_order: compiled.defs_global_init_order,
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
        defs_global_declarations: compiled.defs_global_declarations,
        defs_global_init_order: compiled.defs_global_init_order,
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
        defs_global_declarations: compiled.defs_global_declarations,
        defs_global_init_order: compiled.defs_global_init_order,
        host_functions: None,
        random_seed: Some(1),
        compiler_version: Some(DEFAULT_COMPILER_VERSION.to_string()),
    });
    let error = result
        .err()
        .expect("normalized symbol conflict should fail");
    assert_eq!(error.code, "ENGINE_DEFS_FUNCTION_SYMBOL_CONFLICT");
}

#[test]
fn random_function_success_and_registry_call_path_are_covered() {
    let files = map(&[(
        "main.script.xml",
        r#"
<script name="main">
  <var name="n" type="int">random(5)</var>
  <text>${n}</text>
</script>
"#,
    )]);
    let mut engine = engine_from_sources(files);
    engine.start("main", None).expect("start");
    let output = engine.next_output().expect("next");
    assert!(matches!(output, EngineOutput::Text { .. }));

    let registry = TestRegistry {
        names: vec!["ok".to_string()],
    };
    let value = registry.call("ok", &[]).expect("call should succeed");
    assert_eq!(value, SlValue::Bool(true));
}

#[test]
fn new_success_path_initializes_defs_and_function_symbols() {
    let files = map(&[
        (
            "main.script.xml",
            r#"
<!-- include: shared.defs.xml -->
<script name="main"><text>ok</text></script>
"#,
        ),
        (
            "shared.defs.xml",
            r#"
<defs name="shared">
  <var name="hp" type="int">1</var>
  <function name="addWithGameBonus" args="int:a1,int:a2" return="int:out">
out = a1 + a2;
  </function>
</defs>
"#,
        ),
    ]);
    let compiled = compile_project_bundle_from_xml_map(&files).expect("compile should pass");
    let mut engine = ScriptLangEngine::new(ScriptLangEngineOptions {
        scripts: compiled.scripts,
        global_json: compiled.global_json,
        defs_global_declarations: compiled.defs_global_declarations,
        defs_global_init_order: compiled.defs_global_init_order,
        host_functions: None,
        random_seed: Some(7),
        compiler_version: None,
    })
    .expect("new should succeed");

    assert_eq!(engine.compiler_version(), DEFAULT_COMPILER_VERSION);
    assert!(!engine.waiting_choice());
    assert!(!engine.ended);
    assert!(
        engine.defs_globals_type.contains_key("shared.hp"),
        "defs global type should be initialized"
    );
    assert_eq!(
        engine
            .visible_function_symbols_by_script
            .get("main")
            .and_then(|m| m.get("addWithGameBonus"))
            .map(String::as_str),
        Some("addWithGameBonus")
    );

    engine.start("main", None).expect("start");
    assert_eq!(
        engine.defs_globals_value.get("shared.hp"),
        Some(&SlValue::Number(1.0))
    );
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
fn start_rejects_defs_global_initializer_type_mismatch() {
    let mut engine = engine_from_sources(map(&[
        (
            "shared.defs.xml",
            r#"
<defs name="shared">
  <var name="hp" type="int">"bad"</var>
</defs>
"#,
        ),
        (
            "main.script.xml",
            r#"
<!-- include: shared.defs.xml -->
<script name="main"><text>ok</text></script>
"#,
        ),
    ]));

    let error = engine
        .start("main", None)
        .expect_err("type mismatch should fail");
    assert_eq!(error.code, "ENGINE_TYPE_MISMATCH");
}

#[test]
fn start_rejects_missing_defs_global_decl_in_init_order() {
    let mut engine = engine_from_sources(map(&[(
        "main.script.xml",
        r#"<script name="main"><text>ok</text></script>"#,
    )]));
    engine.defs_global_init_order = vec!["shared.hp".to_string()];
    let error = engine
        .start("main", None)
        .expect_err("missing decl in init order should fail");
    assert_eq!(error.code, "ENGINE_DEFS_GLOBAL_DECL_MISSING");
}

#[test]
fn start_fills_default_for_defs_global_not_present_in_init_order() {
    let mut engine = engine_from_sources(map(&[(
        "main.script.xml",
        r#"<script name="main"><text>ok</text></script>"#,
    )]));
    engine.defs_global_declarations.insert(
        "shared.hp".to_string(),
        DefsGlobalVarDecl {
            namespace: "shared".to_string(),
            name: "hp".to_string(),
            qualified_name: "shared.hp".to_string(),
            r#type: ScriptType::Primitive {
                name: "int".to_string(),
            },
            initial_value_expr: None,
            location: sl_core::SourceSpan::synthetic(),
        },
    );
    engine.defs_global_init_order.clear();

    engine.start("main", None).expect("start");
    assert_eq!(
        engine.defs_globals_value.get("shared.hp"),
        Some(&SlValue::Number(0.0))
    );
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
fn random_function_error_path_is_covered() {
    // Test that random(n) with n <= 0 returns error (covers lifecycle.rs lines 218, 220-222)
    let mut engine = engine_from_sources(map(&[(
        "main.script.xml",
        r#"
<script name="main">
  <code>let x = random(0);</code>
  <text>done</text>
</script>
"#,
    )]));
    engine.start("main", None).expect("start");
    let error = engine.next_output().expect_err("random(0) should fail");
    assert!(error.code == "ENGINE_EVAL_ERROR" || error.code == "ENGINE_RANDOM_ERROR");
}
