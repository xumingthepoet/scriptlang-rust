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
    validate_import_graph(&sources)?;

    let module_scripts_by_path = parse_module_scripts(&sources)?;
    let defs_by_path = parse_defs_files(&sources)
        .expect("defs parsing should match previously validated module parsing");
    let global_json = BTreeMap::new();
    let (defs_global_declarations, defs_global_init_order) =
        collect_defs_globals_for_bundle(&defs_by_path)?;
    let (defs_global_const_declarations, defs_global_const_init_order) =
        collect_defs_consts_for_bundle(&defs_by_path, &defs_global_declarations)?;

    let mut scripts = BTreeMap::new();
    let mut reachable_cache = HashMap::new();

    for (file_path, source) in &sources {
        let reachable = reachable_cache
            .entry(file_path.clone())
            .or_insert_with(|| collect_reachable_imports(file_path, &sources));
        let script_roots = collect_source_scripts(source, file_path, &module_scripts_by_path);
        for script_decl in script_roots {
            let (visible_types, visible_functions, visible_defs_globals, visible_defs_consts) =
                resolve_visible_defs(reachable, &defs_by_path, script_decl.module_name.as_deref())
                    .map_err(|error| with_file_context(error, file_path))?;
            let ir = compile_script(CompileScriptOptions {
                script_path: file_path,
                root: &script_decl.root,
                script_access: script_decl.script_access,
                qualified_script_name: script_decl.qualified_script_name.as_deref(),
                module_name: script_decl.module_name.as_deref(),
                visible_types: &visible_types,
                visible_functions: &visible_functions,
                visible_defs_globals: &visible_defs_globals,
                visible_defs_consts: &visible_defs_consts,
            })
            .map_err(|error| with_file_context(error, file_path))?;
            if scripts.contains_key(&ir.script_name) {
                return Err(ScriptLangError::with_span(
                    "SCRIPT_NAME_DUPLICATE",
                    format!("Duplicate script name \"{}\".", ir.script_name),
                    script_decl.root.location.clone(),
                ));
            }

            scripts.insert(ir.script_name.clone(), ir);
        }
    }

    Ok(CompileProjectBundleResult {
        scripts,
        global_json,
        defs_global_declarations,
        defs_global_init_order,
        defs_global_const_declarations,
        defs_global_const_init_order,
    })
}

#[derive(Clone)]
struct SourceScriptToCompile {
    root: XmlElementNode,
    script_access: AccessLevel,
    qualified_script_name: Option<String>,
    module_name: Option<String>,
}

fn collect_source_scripts(
    source: &SourceFile,
    file_path: &str,
    module_scripts_by_path: &BTreeMap<String, Vec<ParsedModuleScript>>,
) -> Vec<SourceScriptToCompile> {
    match source.kind {
        SourceKind::ModuleXml => module_scripts_by_path
            .get(file_path)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|script| {
                let module_name = script
                    .qualified_script_name
                    .split('.')
                    .next()
                    .unwrap_or_default()
                    .to_string();
                SourceScriptToCompile {
                    root: script.root,
                    script_access: script.access,
                    qualified_script_name: Some(script.qualified_script_name),
                    module_name: Some(module_name),
                }
            })
            .collect(),
        _ => Vec::new(),
    }
}

pub(crate) fn with_file_context(error: ScriptLangError, file_path: &str) -> ScriptLangError {
    crate::with_file_context_shared(error, file_path)
}

#[cfg(test)]
mod pipeline_tests {
    use super::*;
    use crate::compiler_test_support::*;
    use sl_core::SourceLocation;

    #[test]
    fn compile_basic_script_project() {
        let files = map(&[(
            "main.xml",
            r#"
    <module name="main" default_access="public">
    <script name="main">
      <text>Hello</text>
      <choice text="Pick">
        <option text="A"><text>A1</text></option>
      </choice>
    </script>
    </module>
    "#,
        )]);

        let result = compile_project_bundle_from_xml_map(&files).expect("project should compile");
        assert!(result.scripts.contains_key("main.main"));
        let main = result.scripts.get("main.main").expect("main script");
        assert!(!main.groups.is_empty());
    }

    #[test]
    fn compile_bundle_supports_mixed_sources_without_filesystem_examples() {
        let files = map(&[
            (
                "shared.xml",
                r#"
<module name="shared" default_access="public">
  <var name="hp" type="int">100</var>
</module>
"#,
            ),
            (
                "battle.xml",
                r#"
<!-- import shared from shared.xml -->
<module name="battle" default_access="public">
<script name="battle">
  <text>battle.hp=${shared.hp}</text>
</script>
</module>
"#,
            ),
            (
                "main.xml",
                r#"
<!-- import shared from shared.xml -->
<!-- import battle from battle.xml -->
<module name="main" default_access="public">
<script name="main">
  <text>main.hp=${shared.hp}</text>
  <call script="battle"/>
</script>
</module>
"#,
            ),
        ]);

        let bundle = compile_project_bundle_from_xml_map(&files).expect("compile should pass");
        assert!(bundle.scripts.contains_key("main.main"));
        assert!(bundle.scripts.contains_key("battle.battle"));
        assert!(bundle.global_json.is_empty());
    }

    #[test]
    fn compile_bundle_supports_module_files_and_qualified_script_names() {
        let files = map(&[(
            "battle.xml",
            r#"
<module name="battle" default_access="public">
  <type name="Combatant">
    <field name="hp" type="int"/>
  </type>
  <function name="boost" args="int:x" return="int:out">out = x + 1;</function>
  <var name="baseHp" type="int">40</var>
  <script name="main">
    <temp name="hero" type="Combatant">#{hp: baseHp}</temp>
    <call script="next"/>
  </script>
  <script name="next">
    <text>${boost(hero.hp)}</text>
  </script>
</module>
"#,
        )]);

        let bundle = compile_project_bundle_from_xml_map(&files).expect("module compile");
        assert!(bundle.scripts.contains_key("battle.main"));
        assert!(bundle.scripts.contains_key("battle.next"));
        let main = bundle.scripts.get("battle.main").expect("qualified main");
        assert_eq!(main.module_name.as_deref(), Some("battle"));
        assert_eq!(main.local_script_name.as_deref(), Some("main"));

        let root_group = main.groups.get(&main.root_group_id).expect("root group");
        let has_qualified_call = root_group.nodes.iter().any(|node| {
            matches!(
                node,
                ScriptNode::Call { target_script, .. } if target_script == "battle.next"
            )
        });
        assert!(
            has_qualified_call,
            "local module call should be qualified at compile time"
        );
        assert!(main.visible_functions.contains_key("boost"));
        assert!(main.visible_defs_globals.contains_key("baseHp"));
    }

    #[test]
    fn compile_bundle_allows_same_local_script_name_across_modules() {
        let files = map(&[
            (
                "a.xml",
                r#"<module name="a" default_access="public"><script name="main"><text>A</text></script></module>"#,
            ),
            (
                "b.xml",
                r#"<module name="b" default_access="public"><script name="main"><text>B</text></script></module>"#,
            ),
        ]);

        let bundle = compile_project_bundle_from_xml_map(&files)
            .expect("duplicate local names across modules should pass");
        assert!(bundle.scripts.contains_key("a.main"));
        assert!(bundle.scripts.contains_key("b.main"));
    }

    #[test]
    fn compile_bundle_rejects_invalid_module_shapes() {
        let missing_name = map(&[(
            "bad.xml",
            r#"<module><script name="main"><text>x</text></script></module>"#,
        )]);
        let missing_name_error =
            compile_project_bundle_from_xml_map(&missing_name).expect_err("module name required");
        assert_eq!(missing_name_error.code, "XML_MODULE_NAME_MISSING");

        let invalid_child = map(&[(
            "bad.xml",
            r#"<module name="bad" default_access="public"><unknown/></module>"#,
        )]);
        let invalid_child_error =
            compile_project_bundle_from_xml_map(&invalid_child).expect_err("invalid child");
        assert_eq!(invalid_child_error.code, "XML_MODULE_CHILD_INVALID");

        let duplicate = map(&[(
            "bad.xml",
            r#"<module name="bad" default_access="public"><script name="main"/><script name="main"/></module>"#,
        )]);
        let duplicate_error =
            compile_project_bundle_from_xml_map(&duplicate).expect_err("duplicate qualified name");
        assert_eq!(duplicate_error.code, "SCRIPT_NAME_DUPLICATE");

        let invalid_default_access = map(&[(
            "bad.xml",
            r#"<module name="bad" default_access="open"><script name="main"><text>x</text></script></module>"#,
        )]);
        let invalid_default_access_error =
            compile_project_bundle_from_xml_map(&invalid_default_access)
                .expect_err("invalid default_access should fail");
        assert_eq!(invalid_default_access_error.code, "XML_ACCESS_INVALID");

        let invalid_access = map(&[(
            "bad.xml",
            r#"<module name="bad" default_access="public"><script name="main" access="open"><text>x</text></script></module>"#,
        )]);
        let invalid_access_error = compile_project_bundle_from_xml_map(&invalid_access)
            .expect_err("invalid access should fail");
        assert_eq!(invalid_access_error.code, "XML_ACCESS_INVALID");

        let typo_default_access = map(&[(
            "bad.xml",
            r#"<module name="bad" defaul_access="public"><script name="main"><text>x</text></script></module>"#,
        )]);
        let typo_default_access_error = compile_project_bundle_from_xml_map(&typo_default_access)
            .expect_err("defaul_access typo should fail");
        assert_eq!(typo_default_access_error.code, "XML_ATTR_NOT_ALLOWED");
    }

    #[test]
    fn compile_bundle_supports_directory_imports() {
        let files = map(&[
            (
                "shared/support/types.xml",
                r#"
<module name="shared" default_access="public">
  <function name="boost" args="int:x" return="int:out">
    out = x + 1;
  </function>
</module>
"#,
            ),
            (
                "shared/nested/battle.xml",
                r#"
<!-- import {shared} from ../support/ -->
<module name="battle" default_access="public">
<script name="battle">
  <text>battle=${shared.boost(3)}</text>
</script>
</module>
"#,
            ),
            (
                "main.xml",
                r#"
<!-- import {battle, shared} from shared/ -->
<module name="main" default_access="public">
<script name="main">
  <text>main=${shared.boost(3)}</text>
  <call script="battle"/>
</script>
</module>
"#,
            ),
        ]);

        let bundle = compile_project_bundle_from_xml_map(&files).expect("compile should pass");
        assert!(bundle.scripts.contains_key("main.main"));
        assert!(bundle.scripts.contains_key("battle.battle"));
        assert!(bundle.global_json.is_empty());
    }

    #[test]
    fn compile_scripts_from_xml_map_returns_script_only_bundle() {
        let files = map(&[
            (
                "main.xml",
                r#"<module name="main" default_access="public">
<script name="main"><text>Main</text></script>
</module>"#,
            ),
            (
                "alt.xml",
                r#"<module name="alt" default_access="public">
<script name="alt"><text>Alt</text></script>
</module>"#,
            ),
        ]);
        let scripts = compile_project_scripts_from_xml_map(&files).expect("compile should pass");
        assert!(scripts.contains_key("main.main"));
        assert!(scripts.contains_key("alt.alt"));
    }

    #[test]
    fn compile_scripts_from_xml_map_propagates_errors() {
        let files = map(&[("bad.xml", "<script>")]);
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
    fn compile_bundle_rejects_missing_import_and_cycle() {
        let missing_import = map(&[(
            "main.xml",
            r#"
    <!-- import missing from missing.xml -->
    <module name="main" default_access="public">
<script name="main"></script>
</module>
    "#,
        )]);
        let missing = compile_project_bundle_from_xml_map(&missing_import)
            .expect_err("missing import should fail");
        assert_eq!(missing.code, "IMPORT_FILE_NOT_FOUND");

        let empty_directory_import = map(&[(
            "main.xml",
            r#"
    <!-- import {missing} from missing/ -->
    <module name="main" default_access="public">
<script name="main"></script>
</module>
    "#,
        )]);
        let empty_directory = compile_project_bundle_from_xml_map(&empty_directory_import)
            .expect_err("empty directory import should fail");
        assert_eq!(empty_directory.code, "IMPORT_DIR_EMPTY");
        assert!(empty_directory.message.contains("main.xml"));

        let cycle = map(&[
            (
                "a.xml",
                r#"
    <!-- import b from b.xml -->
    <module name="a" default_access="public">
<script name="a"></script>
</module>
    "#,
            ),
            (
                "b.xml",
                r#"
    <!-- import a from a.xml -->
    <module name="b" default_access="public">
<script name="b"></script>
</module>
    "#,
            ),
        ]);
        let cycle_error =
            compile_project_bundle_from_xml_map(&cycle).expect_err("import cycle should fail");
        assert_eq!(cycle_error.code, "IMPORT_CYCLE");

        let directory_cycle = map(&[
            (
                "main.xml",
                r#"
    <!-- import {loop} from shared/ -->
    <module name="main" default_access="public">
<script name="main"></script>
</module>
    "#,
            ),
            (
                "shared/loop.xml",
                r#"
    <!-- import main from ../main.xml -->
    <module name="loop" default_access="public">
<script name="loop"></script>
</module>
    "#,
            ),
        ]);
        let directory_cycle_error = compile_project_bundle_from_xml_map(&directory_cycle)
            .expect_err("directory import cycle should fail");
        assert_eq!(directory_cycle_error.code, "IMPORT_CYCLE");
    }

    #[test]
    fn compile_bundle_rejects_invalid_root_and_duplicate_script_names() {
        let invalid_root = BTreeMap::from([(
            "main.xml".to_string(),
            "<defs name=\"x\"></defs>".to_string(),
        )]);
        let root_error =
            compile_project_bundle_from_xml_map(&invalid_root).expect_err("invalid root");
        assert_eq!(root_error.code, "XML_ROOT_INVALID");

        let duplicate_script_name = map(&[
            ("a.xml", "<script name=\"main\"></script>"),
            ("b.xml", "<script name=\"main\"></script>"),
        ]);
        let duplicate_error = compile_project_bundle_from_xml_map(&duplicate_script_name)
            .expect_err("duplicate script names should fail");
        assert_eq!(duplicate_error.code, "SCRIPT_NAME_DUPLICATE");
    }

    #[test]
    fn collect_source_scripts_and_module_parse_helpers_cover_non_happy_paths() {
        let json_source = SourceFile {
            kind: SourceKind::Json,
            imports: Vec::new(),
            xml_root: None,
            json_value: Some(SlValue::Bool(true)),
        };
        let collected = collect_source_scripts(&json_source, "shared.json", &BTreeMap::new());
        assert!(collected.is_empty());

        let global_json =
            collect_global_json(&BTreeMap::from([("shared.json".to_string(), json_source)]))
                .expect("internal json helper should still collect manual sources");
        assert_eq!(global_json.get("shared"), Some(&SlValue::Bool(true)));

        let bad_module = map(&[(
            "bad.xml",
            r#"<module name="bad" default_access="public"><script><text>x</text></script></module>"#,
        )]);
        let error = compile_project_bundle_from_xml_map(&bad_module)
            .expect_err("module script validation should propagate through pipeline");
        assert_eq!(error.code, "XML_MISSING_ATTR");
    }

    #[test]
    fn compile_bundle_errors_import_file_context() {
        let xml_parse = map(&[("bad.xml", "<script>")]);
        let parse_error =
            compile_project_bundle_from_xml_map(&xml_parse).expect_err("xml parse should fail");
        assert!(parse_error.message.contains("bad.xml"));

        let compile_error_case = map(&[(
            "broken.xml",
            r#"<module name="main" default_access="public">
<script name="main"><break/></script>
</module>"#,
        )]);
        let compile_error = compile_project_bundle_from_xml_map(&compile_error_case)
            .expect_err("break outside while should fail");
        assert!(compile_error.message.contains("broken.xml"));
    }

    #[test]
    fn compile_bundle_exposes_defs_globals_with_short_alias_rules() {
        let unique = map(&[
            (
                "shared.xml",
                r#"<module name="shared" default_access="public"><var name="hp" type="int">1</var></module>"#,
            ),
            (
                "main.xml",
                r#"
<!-- import shared from shared.xml -->
<module name="main" default_access="public">
<script name="main"><text>${hp + shared.hp}</text></script>
</module>
"#,
            ),
        ]);
        let unique_bundle = compile_project_bundle_from_xml_map(&unique).expect("compile");
        let unique_main = unique_bundle.scripts.get("main.main").expect("main");
        assert!(unique_main.visible_defs_globals.contains_key("shared.hp"));
        assert!(!unique_main.visible_defs_globals.contains_key("hp"));

        let conflict = map(&[
            (
                "a.xml",
                r#"<module name="a" default_access="public"><var name="hp" type="int">1</var></module>"#,
            ),
            (
                "b.xml",
                r#"<module name="b" default_access="public"><var name="hp" type="int">2</var></module>"#,
            ),
            (
                "main.xml",
                r#"
<!-- import a from a.xml -->
<!-- import b from b.xml -->
<module name="main" default_access="public">
<script name="main"><text>${a.hp + b.hp}</text></script>
</module>
"#,
            ),
        ]);
        let conflict_bundle = compile_project_bundle_from_xml_map(&conflict).expect("compile");
        let conflict_main = conflict_bundle.scripts.get("main.main").expect("main");
        assert!(conflict_main.visible_defs_globals.contains_key("a.hp"));
        assert!(conflict_main.visible_defs_globals.contains_key("b.hp"));
        assert!(!conflict_main.visible_defs_globals.contains_key("hp"));
    }

    #[test]
    fn compile_bundle_requires_temp_and_enforces_access_visibility() {
        let removed_var = map(&[(
            "main.xml",
            r#"<module name="main" default_access="public"><script name="main"><var name="x" type="int">1</var></script></module>"#,
        )]);
        let removed_var_error = compile_project_bundle_from_xml_map(&removed_var)
            .expect_err("script <var> should fail");
        assert_eq!(removed_var_error.code, "XML_REMOVED_NODE");

        let private_import = map(&[
            (
                "shared.xml",
                r#"<module name="shared"><var name="hp" type="int">1</var></module>"#,
            ),
            (
                "main.xml",
                r#"
<!-- import shared from shared.xml -->
<module name="main" default_access="public">
<script name="main"><text>${shared.hp}</text></script>
</module>
"#,
            ),
        ]);
        let private_bundle = compile_project_bundle_from_xml_map(&private_import).expect("compile");
        let private_main = private_bundle.scripts.get("main.main").expect("main");
        assert!(!private_main.visible_defs_globals.contains_key("shared.hp"));

        let public_import = map(&[
            (
                "shared.xml",
                r#"<module name="shared" default_access="public"><var name="hp" type="int">1</var></module>"#,
            ),
            (
                "main.xml",
                r#"
<!-- import shared from shared.xml -->
<module name="main" default_access="public">
<script name="main"><text>${shared.hp}</text></script>
</module>
"#,
            ),
        ]);
        let public_bundle = compile_project_bundle_from_xml_map(&public_import).expect("compile");
        let public_main = public_bundle.scripts.get("main.main").expect("main");
        assert!(public_main.visible_defs_globals.contains_key("shared.hp"));
    }

    #[test]
    fn compile_bundle_tracks_script_access_metadata() {
        let files = map(&[(
            "main.xml",
            r#"<module name="main" default_access="private">
<script name="main" access="public"><text>pub</text></script>
<script name="hidden"><text>pri</text></script>
</module>"#,
        )]);
        let bundle = compile_project_bundle_from_xml_map(&files).expect("compile");
        assert_eq!(
            bundle.scripts.get("main.main").expect("main").access,
            AccessLevel::Public
        );
        assert_eq!(
            bundle.scripts.get("main.hidden").expect("hidden").access,
            AccessLevel::Private
        );
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
        let wrapped_with_span = with_file_context(with_span, "main.xml");
        assert!(wrapped_with_span
            .message
            .contains("In file \"main.xml\": boom"));
        let span = wrapped_with_span.span.expect("span should be preserved");
        assert_eq!(span.start.line, 7);
        assert_eq!(span.start.column, 9);
        assert_eq!(span.end.line, 7);
        assert_eq!(span.end.column, 9);

        let without_span = ScriptLangError::new("SOME_CODE", "no-span");
        let wrapped_without_span = with_file_context(without_span, "other.xml");
        assert!(wrapped_without_span
            .message
            .contains("In file \"other.xml\": no-span"));
        let synthetic = wrapped_without_span
            .span
            .expect("missing span should become synthetic");
        assert_eq!(synthetic.start.line, 1);
        assert_eq!(synthetic.start.column, 1);
        assert_eq!(synthetic.end.line, 1);
        assert_eq!(synthetic.end.column, 1);
    }

    #[test]
    fn pipeline_helpers_propagate_module_parse_and_lookup_errors() {
        let bad_sources = BTreeMap::from([(
            "bad.xml".to_string(),
            SourceFile {
                kind: SourceKind::ModuleXml,
                xml_root: Some(
                    parse_xml_document("<module><script name=\"main\"/></module>")
                        .expect("xml")
                        .root,
                ),
                json_value: None,
                imports: Vec::new(),
            },
        )]);
        let error = parse_module_scripts(&bad_sources).expect_err("module parse should fail");
        assert_eq!(error.code, "XML_MISSING_ATTR");

        let json_sources = BTreeMap::from([(
            "game.json".to_string(),
            SourceFile {
                kind: SourceKind::Json,
                imports: Vec::new(),
                xml_root: None,
                json_value: Some(SlValue::Bool(true)),
            },
        )]);
        let parsed = parse_module_scripts(&json_sources).expect("json-only sources should parse");
        assert!(parsed.is_empty());
        let source = json_sources.get("game.json").expect("json source");
        let scripts = collect_source_scripts(source, "game.json", &BTreeMap::new());
        assert!(scripts.is_empty());
    }
}
