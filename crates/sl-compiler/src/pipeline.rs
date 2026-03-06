use crate::*;

pub fn compile_project_scripts_from_xml_map(
    xml_by_path: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, ScriptIr>, ScriptLangError> {
    Ok(compile_project_bundle_from_xml_map(xml_by_path)?.scripts)
}

pub fn compile_project_bundle_from_xml_map(
    xml_by_path: &BTreeMap<String, String>,
) -> Result<CompileProjectBundleResult, ScriptLangError> {
    let sources = parse_sources(xml_by_path)?;
    validate_include_graph(&sources)?;

    let defs_by_path = parse_defs_files(&sources)?;
    let global_json = collect_global_json(&sources)?;
    let all_json_symbols = global_json.keys().cloned().collect::<BTreeSet<_>>();
    let (defs_global_declarations, defs_global_init_order) =
        collect_defs_globals_for_bundle(&defs_by_path)?;

    let mut scripts = BTreeMap::new();
    let mut reachable_cache = HashMap::new();

    for (file_path, source) in &sources {
        if !matches!(source.kind, SourceKind::ScriptXml) {
            continue;
        }

        let script_root = source
            .xml_root
            .as_ref()
            .expect("script/defs sources should always carry parsed xml root");

        if script_root.name != "script" {
            return Err(ScriptLangError::with_span(
                "XML_ROOT_INVALID",
                format!(
                    "Expected <script> root in file \"{}\", got <{}>.",
                    file_path, script_root.name
                ),
                script_root.location.clone(),
            ));
        }

        let reachable = reachable_cache
            .entry(file_path.clone())
            .or_insert_with(|| collect_reachable_files(file_path, &sources));
        let (visible_types, visible_functions, visible_defs_globals) =
            resolve_visible_defs(reachable, &defs_by_path)
                .map_err(|error| with_file_context(error, file_path))?;
        let visible_json_symbols = collect_visible_json_symbols(reachable, &sources)
            .expect("collect_visible_json_symbols should be infallible after collect_global_json");
        let ir = compile_script(
            file_path,
            script_root,
            &visible_types,
            &visible_functions,
            &visible_defs_globals,
            &visible_json_symbols,
            &all_json_symbols,
        )
        .map_err(|error| with_file_context(error, file_path))?;
        if scripts.contains_key(&ir.script_name) {
            return Err(ScriptLangError::with_span(
                "SCRIPT_NAME_DUPLICATE",
                format!("Duplicate script name \"{}\".", ir.script_name),
                script_root.location.clone(),
            ));
        }

        scripts.insert(ir.script_name.clone(), ir);
    }

    Ok(CompileProjectBundleResult {
        scripts,
        global_json,
        defs_global_declarations,
        defs_global_init_order,
    })
}

pub(crate) fn with_file_context(error: ScriptLangError, file_path: &str) -> ScriptLangError {
    let code = error.code;
    let message = format!("In file \"{}\": {}", file_path, error.message);
    let span = error.span.unwrap_or(SourceSpan::synthetic());
    ScriptLangError::with_span(code, message, span)
}

#[cfg(test)]
mod pipeline_tests {
    use super::*;
    use crate::compiler_test_support::*;
    use sl_core::SourceLocation;

    #[test]
    fn compile_basic_script_project() {
        let files = map(&[(
            "main.script.xml",
            r#"
    <script name="main">
      <text>Hello</text>
      <choice text="Pick">
        <option text="A"><text>A1</text></option>
      </choice>
    </script>
    "#,
        )]);

        let result = compile_project_bundle_from_xml_map(&files).expect("project should compile");
        assert!(result.scripts.contains_key("main"));
        let main = result.scripts.get("main").expect("main script");
        assert!(!main.groups.is_empty());
    }

    #[test]
    fn compile_bundle_supports_mixed_sources_without_filesystem_examples() {
        let files = map(&[
            (
                "shared.defs.xml",
                r#"
<defs name="shared">
  <var name="hp" type="int">100</var>
</defs>
"#,
            ),
            ("config.json", r#"{"base": 3}"#),
            (
                "battle.script.xml",
                r#"
<!-- include: shared.defs.xml -->
<script name="battle">
  <text>battle.hp=${shared.hp}</text>
</script>
"#,
            ),
            (
                "main.script.xml",
                r#"
<!-- include: shared.defs.xml -->
<!-- include: battle.script.xml -->
<!-- include: config.json -->
<script name="main">
  <text>main.base=${config.base}</text>
  <call script="battle"/>
</script>
"#,
            ),
        ]);

        let bundle = compile_project_bundle_from_xml_map(&files).expect("compile should pass");
        assert!(bundle.scripts.contains_key("main"));
        assert!(bundle.scripts.contains_key("battle"));
        assert!(bundle.global_json.contains_key("config"));
    }

    #[test]
    fn compile_bundle_supports_directory_includes() {
        let files = map(&[
            (
                "shared/support/types.defs.xml",
                r#"
<defs name="shared">
  <function name="boost" args="int:x" return="int:out">
    out = x + 1;
  </function>
</defs>
"#,
            ),
            ("shared/support/config.json", r#"{"base": 3}"#),
            (
                "shared/nested/battle.script.xml",
                r#"
<!-- include: ../support/ -->
<script name="battle">
  <text>battle=${shared.boost(config.base)}</text>
</script>
"#,
            ),
            (
                "main.script.xml",
                r#"
<!-- include: shared/ -->
<script name="main">
  <text>main=${shared.boost(config.base)}</text>
  <call script="battle"/>
</script>
"#,
            ),
        ]);

        let bundle = compile_project_bundle_from_xml_map(&files).expect("compile should pass");
        assert!(bundle.scripts.contains_key("main"));
        assert!(bundle.scripts.contains_key("battle"));
        assert!(bundle.global_json.contains_key("config"));
    }

    #[test]
    fn compile_scripts_from_xml_map_returns_script_only_bundle() {
        let files = map(&[
            (
                "main.script.xml",
                r#"<script name="main"><text>Main</text></script>"#,
            ),
            (
                "alt.script.xml",
                r#"<script name="alt"><text>Alt</text></script>"#,
            ),
        ]);
        let scripts = compile_project_scripts_from_xml_map(&files).expect("compile should pass");
        assert!(scripts.contains_key("main"));
        assert!(scripts.contains_key("alt"));
    }

    #[test]
    fn compile_scripts_from_xml_map_propagates_errors() {
        let files = map(&[("bad.script.xml", "<script>")]);
        let error =
            compile_project_scripts_from_xml_map(&files).expect_err("invalid xml should fail");
        assert_eq!(error.code, "XML_PARSE_ERROR");
    }

    #[test]
    fn compile_bundle_rejects_unsupported_source_extension() {
        let files = BTreeMap::from([("x.txt".to_string(), "bad".to_string())]);
        let error = compile_project_bundle_from_xml_map(&files)
            .expect_err("unsupported extension should fail");
        assert_eq!(error.code, "SOURCE_KIND_UNSUPPORTED");
    }

    #[test]
    fn compile_bundle_rejects_missing_include_and_cycle() {
        let missing_include = map(&[(
            "main.script.xml",
            r#"
    <!-- include: missing.script.xml -->
    <script name="main"></script>
    "#,
        )]);
        let missing = compile_project_bundle_from_xml_map(&missing_include)
            .expect_err("missing include should fail");
        assert_eq!(missing.code, "INCLUDE_NOT_FOUND");

        let empty_directory_include = map(&[(
            "main.script.xml",
            r#"
    <!-- include: missing/ -->
    <script name="main"></script>
    "#,
        )]);
        let empty_directory = compile_project_bundle_from_xml_map(&empty_directory_include)
            .expect_err("empty directory include should fail");
        assert_eq!(empty_directory.code, "INCLUDE_DIR_EMPTY");
        assert!(empty_directory.message.contains("main.script.xml"));

        let cycle = map(&[
            (
                "a.script.xml",
                r#"
    <!-- include: b.script.xml -->
    <script name="a"></script>
    "#,
            ),
            (
                "b.script.xml",
                r#"
    <!-- include: a.script.xml -->
    <script name="b"></script>
    "#,
            ),
        ]);
        let cycle_error =
            compile_project_bundle_from_xml_map(&cycle).expect_err("include cycle should fail");
        assert_eq!(cycle_error.code, "INCLUDE_CYCLE");

        let directory_cycle = map(&[
            (
                "main.script.xml",
                r#"
    <!-- include: shared/ -->
    <script name="main"></script>
    "#,
            ),
            (
                "shared/loop.script.xml",
                r#"
    <!-- include: ../main.script.xml -->
    <script name="loop"></script>
    "#,
            ),
        ]);
        let directory_cycle_error = compile_project_bundle_from_xml_map(&directory_cycle)
            .expect_err("directory include cycle should fail");
        assert_eq!(directory_cycle_error.code, "INCLUDE_CYCLE");
    }

    #[test]
    fn compile_bundle_rejects_invalid_root_and_duplicate_script_names() {
        let invalid_root = map(&[("main.script.xml", "<defs name=\"x\"></defs>")]);
        let root_error =
            compile_project_bundle_from_xml_map(&invalid_root).expect_err("invalid root");
        assert_eq!(root_error.code, "XML_ROOT_INVALID");

        let duplicate_script_name = map(&[
            ("a.script.xml", "<script name=\"main\"></script>"),
            ("b.script.xml", "<script name=\"main\"></script>"),
        ]);
        let duplicate_error = compile_project_bundle_from_xml_map(&duplicate_script_name)
            .expect_err("duplicate script names should fail");
        assert_eq!(duplicate_error.code, "SCRIPT_NAME_DUPLICATE");
    }

    #[test]
    fn compile_bundle_errors_include_file_context() {
        let xml_parse = map(&[("bad.script.xml", "<script>")]);
        let parse_error =
            compile_project_bundle_from_xml_map(&xml_parse).expect_err("xml parse should fail");
        assert!(parse_error.message.contains("bad.script.xml"));

        let compile_error_case = map(&[(
            "broken.script.xml",
            r#"<script name="main"><break/></script>"#,
        )]);
        let compile_error = compile_project_bundle_from_xml_map(&compile_error_case)
            .expect_err("break outside while should fail");
        assert!(compile_error.message.contains("broken.script.xml"));
    }

    #[test]
    fn compile_bundle_exposes_defs_globals_with_short_alias_rules() {
        let unique = map(&[
            (
                "shared.defs.xml",
                r#"<defs name="shared"><var name="hp" type="int">1</var></defs>"#,
            ),
            (
                "main.script.xml",
                r#"
<!-- include: shared.defs.xml -->
<script name="main"><text>${hp + shared.hp}</text></script>
"#,
            ),
        ]);
        let unique_bundle = compile_project_bundle_from_xml_map(&unique).expect("compile");
        let unique_main = unique_bundle.scripts.get("main").expect("main");
        assert!(unique_main.visible_defs_globals.contains_key("shared.hp"));
        assert!(unique_main.visible_defs_globals.contains_key("hp"));

        let conflict = map(&[
            (
                "a.defs.xml",
                r#"<defs name="a"><var name="hp" type="int">1</var></defs>"#,
            ),
            (
                "b.defs.xml",
                r#"<defs name="b"><var name="hp" type="int">2</var></defs>"#,
            ),
            (
                "main.script.xml",
                r#"
<!-- include: a.defs.xml -->
<!-- include: b.defs.xml -->
<script name="main"><text>${a.hp + b.hp}</text></script>
"#,
            ),
        ]);
        let conflict_bundle = compile_project_bundle_from_xml_map(&conflict).expect("compile");
        let conflict_main = conflict_bundle.scripts.get("main").expect("main");
        assert!(conflict_main.visible_defs_globals.contains_key("a.hp"));
        assert!(conflict_main.visible_defs_globals.contains_key("b.hp"));
        assert!(!conflict_main.visible_defs_globals.contains_key("hp"));
    }

    #[test]
    fn with_file_context_adds_file_name_and_preserves_or_synthesizes_span() {
        let with_span = ScriptLangError::with_span(
            "SOME_CODE",
            "boom",
            SourceSpan {
                start: SourceLocation { line: 7, column: 9 },
                end: SourceLocation { line: 7, column: 9 },
            },
        );
        let wrapped_with_span = with_file_context(with_span, "main.script.xml");
        assert!(wrapped_with_span
            .message
            .contains("In file \"main.script.xml\": boom"));
        let span = wrapped_with_span.span.expect("span should be preserved");
        assert_eq!(span.start.line, 7);
        assert_eq!(span.start.column, 9);
        assert_eq!(span.end.line, 7);
        assert_eq!(span.end.column, 9);

        let without_span = ScriptLangError::new("SOME_CODE", "no-span");
        let wrapped_without_span = with_file_context(without_span, "other.script.xml");
        assert!(wrapped_without_span
            .message
            .contains("In file \"other.script.xml\": no-span"));
        let synthetic = wrapped_without_span
            .span
            .expect("missing span should become synthetic");
        assert_eq!(synthetic.start.line, 1);
        assert_eq!(synthetic.start.column, 1);
        assert_eq!(synthetic.end.line, 1);
        assert_eq!(synthetic.end.column, 1);
    }
}
