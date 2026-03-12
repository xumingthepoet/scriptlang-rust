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
    let mut all_script_access = BTreeMap::new();
    for scripts in module_scripts_by_path.values() {
        for script in scripts {
            all_script_access.insert(script.qualified_script_name.clone(), script.access);
        }
    }
    let module_by_path = parse_module_files(&sources)
        .expect("module parsing should match previously validated module parsing");
    let module_alias_directives_by_namespace =
        collect_module_alias_directives_by_namespace(&sources);
    let global_data = BTreeMap::new();
    let invoke_all_functions = collect_functions_for_bundle_with_aliases(
        &module_by_path,
        &module_alias_directives_by_namespace,
    )?;
    let (module_var_declarations, module_var_init_order) =
        collect_module_vars_for_bundle(&module_by_path, &invoke_all_functions)?;
    let (module_const_declarations, module_const_init_order) = collect_module_consts_for_bundle(
        &module_by_path,
        &module_var_declarations,
        &invoke_all_functions,
    )?;

    let mut scripts = BTreeMap::new();
    let mut reachable_cache = HashMap::new();

    for (file_path, source) in &sources {
        let reachable = reachable_cache
            .entry(file_path.clone())
            .or_insert_with(|| collect_reachable_imports(file_path, &sources));
        let script_roots = collect_source_scripts(source, file_path, &module_scripts_by_path);
        for script_decl in script_roots {
            let (visible_types, visible_functions, visible_module_vars, visible_module_consts) =
                resolve_visible_module_symbols_with_aliases_and_module_scoped_type_aliases(
                    reachable,
                    &module_by_path,
                    script_decl.module_name.as_deref(),
                    &source.alias_directives,
                    &module_alias_directives_by_namespace,
                )
                .map_err(|error| with_file_context(error, file_path))?;
            let ir = compile_script(CompileScriptOptions {
                script_path: file_path,
                root: &script_decl.root,
                script_access: script_decl.script_access,
                qualified_script_name: script_decl.qualified_script_name.as_deref(),
                module_name: script_decl.module_name.as_deref(),
                visible_types: &visible_types,
                visible_functions: &visible_functions,
                visible_module_vars: &visible_module_vars,
                visible_module_consts: &visible_module_consts,
                all_script_access: &all_script_access,
                invoke_all_functions: &invoke_all_functions,
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
    validate_static_literal_script_target_rules(&scripts)?;

    Ok(CompileProjectBundleResult {
        scripts,
        global_data,
        module_var_declarations,
        module_var_init_order,
        module_const_declarations,
        module_const_init_order,
    })
}

fn collect_module_alias_directives_by_namespace(
    sources: &BTreeMap<String, SourceFile>,
) -> BTreeMap<String, Vec<AliasDirective>> {
    let mut directives_by_namespace = BTreeMap::new();
    for source in sources.values() {
        // xml_root is always Some - parse_sources always sets it
        let root = source
            .xml_root
            .as_ref()
            .expect("xml_root should be present");
        // root.name is always "module" - extract_module_name rejects non-module roots at parse time
        // name attribute is always present - extract_module_name validates this at parse time
        let namespace = root
            .attributes
            .get("name")
            .expect("module name should be present");
        if source.alias_directives.is_empty() {
            continue;
        }
        directives_by_namespace.insert(namespace.clone(), source.alias_directives.clone());
    }
    directives_by_namespace
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

fn validate_static_literal_script_target_rules(
    scripts: &BTreeMap<String, ScriptIr>,
) -> Result<(), ScriptLangError> {
    for script in scripts.values() {
        for group in script.groups.values() {
            for node in &group.nodes {
                match node {
                    ScriptNode::Call {
                        target_script: ScriptTarget::Literal { script_name },
                        args,
                        location,
                        ..
                    } => {
                        validate_single_literal_call_arg_count(
                            scripts,
                            script_name,
                            args.len(),
                            location,
                            "SCRIPT_CALL_ARGS_COUNT_MISMATCH",
                            "call",
                        )?;
                        validate_single_literal_target_script_kind(
                            scripts,
                            script_name,
                            ScriptKind::Call,
                            location,
                            "SCRIPT_CALL_TARGET_KIND_MISMATCH",
                            "call",
                        )?;
                    }
                    ScriptNode::Goto {
                        target_script: ScriptTarget::Literal { script_name },
                        args,
                        location,
                        ..
                    } => {
                        validate_single_literal_call_arg_count(
                            scripts,
                            script_name,
                            args.len(),
                            location,
                            "SCRIPT_GOTO_ARGS_COUNT_MISMATCH",
                            "goto",
                        )?;
                        validate_single_literal_target_script_kind(
                            scripts,
                            script_name,
                            ScriptKind::Goto,
                            location,
                            "SCRIPT_GOTO_TARGET_KIND_MISMATCH",
                            "goto",
                        )?;
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

fn validate_single_literal_target_script_kind(
    scripts: &BTreeMap<String, ScriptIr>,
    target_script_name: &str,
    expected_kind: ScriptKind,
    location: &SourceSpan,
    error_code: &str,
    label: &str,
) -> Result<(), ScriptLangError> {
    let target = scripts
        .get(target_script_name)
        .expect("target script must exist");
    if target.kind != expected_kind {
        let expected_kind_label = match expected_kind {
            ScriptKind::Call => "call",
            ScriptKind::Goto => "goto",
        };
        return Err(ScriptLangError::with_span(
            error_code,
            format!(
                "{} target script \"{}\" must be {} kind.",
                label, target_script_name, expected_kind_label
            ),
            location.clone(),
        ));
    }
    Ok(())
}

fn validate_single_literal_call_arg_count(
    scripts: &BTreeMap<String, ScriptIr>,
    target_script_name: &str,
    arg_count: usize,
    location: &SourceSpan,
    error_code: &str,
    label: &str,
) -> Result<(), ScriptLangError> {
    let target = scripts
        .get(target_script_name)
        .expect("target script must exist");
    let expected = target.params.len();
    if arg_count != expected {
        return Err(ScriptLangError::with_span(
            error_code,
            format!(
                "{} target script \"{}\" expects {} args, got {}.",
                label, target_script_name, expected, arg_count
            ),
            location.clone(),
        ));
    }
    Ok(())
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
    <module name="main" export="script:main">
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
<module name="shared" export="var:hp">
  <var name="hp" type="int">100</var>
</module>
"#,
            ),
            (
                "battle.xml",
                r#"
<!-- import shared from shared.xml -->
<module name="battle" export="script:battle">
<script name="battle" kind="call">
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
<module name="main" export="script:main">
<script name="main">
  <text>main.hp=${shared.hp}</text>
  <call script="@battle.battle"/>
</script>
</module>
"#,
            ),
        ]);

        let bundle = compile_project_bundle_from_xml_map(&files).expect("compile should pass");
        assert!(bundle.scripts.contains_key("main.main"));
        assert!(bundle.scripts.contains_key("battle.battle"));
        assert!(bundle.global_data.is_empty());
    }

    #[test]
    fn compile_bundle_supports_module_files_and_qualified_script_names() {
        let files = map(&[(
            "battle.xml",
            r#"
<module name="battle" export="script:main,next;function:boost;type:Combatant;var:baseHp">
  <type name="Combatant">
    <field name="hp" type="int"/>
  </type>
  <function name="boost" args="int:x" return_type="int">return x + 1;</function>
  <var name="baseHp" type="int">40</var>
  <script name="main">
    <temp name="hero" type="Combatant">#{hp: baseHp}</temp>
    <call script="@next"/>
  </script>
  <script name="next" kind="call">
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
                ScriptNode::Call {
                    target_script: ScriptTarget::Literal { script_name },
                    ..
                } if script_name == "battle.next"
            )
        });
        assert!(
            has_qualified_call,
            "local module call should be qualified at compile time"
        );
        assert!(main.visible_functions.contains_key("boost"));
        assert!(main.visible_module_vars.contains_key("baseHp"));
    }

    #[test]
    fn compile_bundle_rewrites_explicit_module_alias_in_invoke_functions() {
        let files = map(&[
            (
                "game.xml",
                r#"
<module name="game" export="type:WorldState;var:world_state">
  <type name="WorldState">
    <field name="day_count" type="int"/>
  </type>
  <var name="world_state" type="WorldState">#{day_count: 1}</var>
</module>
"#,
            ),
            (
                "evt.xml",
                r#"
<!-- import game from game.xml -->
<!-- alias game.world_state as world_state -->
<module name="evt" export="script:main;function:can">
  <function name="can" return_type="boolean">
    return world_state.day_count > 0;
  </function>
  <script name="main"><text>ok</text></script>
</module>
"#,
            ),
            (
                "app.xml",
                r#"
<!-- import evt from evt.xml -->
<module name="app" export="script:main">
  <script name="main"><text>run</text></script>
</module>
"#,
            ),
        ]);

        let bundle = compile_project_bundle_from_xml_map(&files).expect("compile should pass");
        let app = bundle.scripts.get("app.main").expect("app script");
        let can = app
            .invoke_all_functions
            .get("evt.can")
            .expect("evt.can should be present in invoke function map");
        assert!(
            can.code.contains("game.world_state.day_count"),
            "explicit alias in module function should be rewritten to qualified access"
        );
    }

    #[test]
    fn compile_bundle_rejects_literal_call_when_arg_count_mismatch() {
        let files = map(&[
            (
                "callee.xml",
                r#"<module name="callee" export="script:callee"><script name="callee" kind="call" args="int:x"><return/></script></module>"#,
            ),
            (
                "main.xml",
                r#"<module name="main" export="script:main"><script name="main"><call script="@callee.callee"/></script></module>"#,
            ),
        ]);

        let error =
            compile_project_bundle_from_xml_map(&files).expect_err("missing arg should fail");
        assert_eq!(error.code, "SCRIPT_CALL_ARGS_COUNT_MISMATCH");
    }

    #[test]
    fn compile_bundle_rejects_literal_goto_when_arg_count_mismatch() {
        let files = map(&[
            (
                "next.xml",
                r#"<module name="next" export="script:next"><script name="next" args="int:x"><text>${x}</text></script></module>"#,
            ),
            (
                "main.xml",
                r#"<module name="main" export="script:main"><script name="main"><goto script="@next.next"/></script></module>"#,
            ),
        ]);

        let error =
            compile_project_bundle_from_xml_map(&files).expect_err("goto missing arg should fail");
        assert_eq!(error.code, "SCRIPT_GOTO_ARGS_COUNT_MISMATCH");
    }

    #[test]
    fn compile_bundle_rejects_literal_call_when_target_is_goto_script() {
        let files = map(&[
            (
                "callee.xml",
                r#"<module name="callee" export="script:callee"><script name="callee"><text>x</text></script></module>"#,
            ),
            (
                "main.xml",
                r#"<module name="main" export="script:main"><script name="main"><call script="@callee.callee"/></script></module>"#,
            ),
        ]);
        let error =
            compile_project_bundle_from_xml_map(&files).expect_err("call target kind should fail");
        assert_eq!(error.code, "SCRIPT_CALL_TARGET_KIND_MISMATCH");
    }

    #[test]
    fn compile_bundle_rejects_literal_goto_when_target_is_call_script() {
        let files = map(&[
            (
                "next.xml",
                r#"<module name="next" export="script:next"><script name="next" kind="call"><return/></script></module>"#,
            ),
            (
                "main.xml",
                r#"<module name="main" export="script:main"><script name="main"><goto script="@next.next"/></script></module>"#,
            ),
        ]);
        let error =
            compile_project_bundle_from_xml_map(&files).expect_err("goto target kind should fail");
        assert_eq!(error.code, "SCRIPT_GOTO_TARGET_KIND_MISMATCH");
    }

    #[test]
    fn compile_bundle_allows_same_local_script_name_across_modules() {
        let files = map(&[
            (
                "a.xml",
                r#"<module name="a" export="script:main"><script name="main"><text>A</text></script></module>"#,
            ),
            (
                "b.xml",
                r#"<module name="b" export="script:main"><script name="main"><text>B</text></script></module>"#,
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

        let invalid_child = map(&[("bad.xml", r#"<module name="bad"><unknown/></module>"#)]);
        let invalid_child_error =
            compile_project_bundle_from_xml_map(&invalid_child).expect_err("invalid child");
        assert_eq!(invalid_child_error.code, "XML_MODULE_CHILD_INVALID");

        let duplicate = map(&[(
            "bad.xml",
            r#"<module name="bad" export="script:main"><script name="main"/><script name="main"/></module>"#,
        )]);
        let duplicate_error =
            compile_project_bundle_from_xml_map(&duplicate).expect_err("duplicate qualified name");
        assert_eq!(duplicate_error.code, "SCRIPT_NAME_DUPLICATE");

        let invalid_export_format = map(&[(
            "bad.xml",
            r#"<module name="bad" export="script"><script name="main"><text>x</text></script></module>"#,
        )]);
        let invalid_export_format_error =
            compile_project_bundle_from_xml_map(&invalid_export_format)
                .expect_err("invalid export format should fail");
        assert_eq!(invalid_export_format_error.code, "XML_EXPORT_INVALID");

        // Test empty export attribute (line 214)
        let empty_export = map(&[(
            "bad.xml",
            r#"<module name="bad" export=""><script name="main"><text>x</text></script></module>"#,
        )]);
        let empty_export_result = compile_project_bundle_from_xml_map(&empty_export);
        assert!(
            empty_export_result.is_ok(),
            "empty export should return default"
        );

        // Test empty group error (line 220): export=";;"
        let empty_group = map(&[(
            "bad.xml",
            r#"<module name="bad" export=";;"><script name="main"><text>x</text></script></module>"#,
        )]);
        let empty_group_error =
            compile_project_bundle_from_xml_map(&empty_group).expect_err("empty group should fail");
        assert_eq!(empty_group_error.code, "XML_EXPORT_INVALID");

        // Test empty names error (line 240): export="script:"
        let empty_names = map(&[(
            "bad.xml",
            r#"<module name="bad" export="script:"><script name="main"><text>x</text></script></module>"#,
        )]);
        let empty_names_error =
            compile_project_bundle_from_xml_map(&empty_names).expect_err("empty names should fail");
        assert_eq!(empty_names_error.code, "XML_EXPORT_INVALID");

        // Test empty name in list error (line 252): export="script:main,"
        let empty_name_in_list = map(&[(
            "bad.xml",
            r#"<module name="bad" export="script:main,"><script name="main"><text>x</text></script></module>"#,
        )]);
        let empty_name_in_list_error = compile_project_bundle_from_xml_map(&empty_name_in_list)
            .expect_err("empty name in list should fail");
        assert_eq!(empty_name_in_list_error.code, "XML_EXPORT_INVALID");

        let invalid_export_kind = map(&[(
            "bad.xml",
            r#"<module name="bad" export="invalid:main"><script name="main"><text>x</text></script></module>"#,
        )]);
        let invalid_export_kind_error = compile_project_bundle_from_xml_map(&invalid_export_kind)
            .expect_err("invalid export kind should fail");
        assert_eq!(invalid_export_kind_error.code, "XML_EXPORT_KIND_INVALID");

        let duplicate_export_target = map(&[(
            "bad.xml",
            r#"<module name="bad" export="script:main;script:main"><script name="main"><text>x</text></script></module>"#,
        )]);
        let duplicate_export_target_error =
            compile_project_bundle_from_xml_map(&duplicate_export_target)
                .expect_err("duplicate export target should fail");
        assert_eq!(duplicate_export_target_error.code, "XML_EXPORT_DUPLICATE");

        let export_target_not_found = map(&[(
            "bad.xml",
            r#"<module name="bad" export="script:missing"><script name="main"><text>x</text></script></module>"#,
        )]);
        let export_target_not_found_error =
            compile_project_bundle_from_xml_map(&export_target_not_found)
                .expect_err("export target must exist");
        assert_eq!(
            export_target_not_found_error.code,
            "XML_EXPORT_TARGET_NOT_FOUND"
        );
    }

    #[test]
    fn compile_bundle_supports_directory_imports() {
        let files = map(&[
            (
                "shared/support/types.xml",
                r#"
<module name="shared" export="function:boost">
  <function name="boost" args="int:x" return_type="int">
    return x + 1;
  </function>
</module>
"#,
            ),
            (
                "shared/nested/battle.xml",
                r#"
<!-- import {shared} from ../support/ -->
<module name="battle" export="script:battle">
<script name="battle" kind="call">
  <text>battle=${shared.boost(3)}</text>
</script>
</module>
"#,
            ),
            (
                "main.xml",
                r#"
<!-- import {battle, shared} from shared/ -->
<module name="main" export="script:main">
<script name="main">
  <text>main=${shared.boost(3)}</text>
  <call script="@battle.battle"/>
</script>
</module>
"#,
            ),
        ]);

        let bundle = compile_project_bundle_from_xml_map(&files).expect("compile should pass");
        assert!(bundle.scripts.contains_key("main.main"));
        assert!(bundle.scripts.contains_key("battle.battle"));
        assert!(bundle.global_data.is_empty());
    }

    #[test]
    fn compile_scripts_from_xml_map_returns_script_only_bundle() {
        let files = map(&[
            (
                "main.xml",
                r#"<module name="main" export="script:main">
<script name="main"><text>Main</text></script>
</module>"#,
            ),
            (
                "alt.xml",
                r#"<module name="alt" export="script:alt">
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
    <module name="main" export="script:main">
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
    <module name="main" export="script:main">
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
    <module name="a" export="script:a">
<script name="a"></script>
</module>
    "#,
            ),
            (
                "b.xml",
                r#"
    <!-- import a from a.xml -->
    <module name="b" export="script:b">
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
    <module name="main" export="script:main">
<script name="main"></script>
</module>
    "#,
            ),
            (
                "shared/loop.xml",
                r#"
    <!-- import main from ../main.xml -->
    <module name="loop" export="script:loop">
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
            "<script name=\"x\"></script>".to_string(),
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
            alias_directives: Vec::new(),
            xml_root: None,
            json_value: Some(SlValue::Bool(true)),
        };
        let collected = collect_source_scripts(&json_source, "shared.json", &BTreeMap::new());
        assert!(collected.is_empty());

        let global_data =
            collect_global_data(&BTreeMap::from([("shared.json".to_string(), json_source)]))
                .expect("internal json helper should still collect manual sources");
        assert_eq!(global_data.get("shared"), Some(&SlValue::Bool(true)));

        let bad_module = map(&[(
            "bad.xml",
            r#"<module name="bad"><script><text>x</text></script></module>"#,
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
            r#"<module name="main" export="script:main">
<script name="main"><break/></script>
</module>"#,
        )]);
        let compile_error = compile_project_bundle_from_xml_map(&compile_error_case)
            .expect_err("break outside while should fail");
        assert!(compile_error.message.contains("broken.xml"));
    }

    #[test]
    fn compile_bundle_wraps_visible_symbol_resolution_errors_with_file_context() {
        let files = map(&[(
            "main.xml",
            r#"<module name="main" export="script:main;function:broken">
<function name="broken" args="Missing:x" return_type="int">return 1;</function>
<script name="main"><text>Main</text></script>
</module>"#,
        )]);
        let error = compile_project_bundle_from_xml_map(&files)
            .expect_err("unknown visible type should fail");
        assert_eq!(error.code, "TYPE_UNKNOWN");
    }

    #[test]
    fn compile_bundle_exposes_module_vars_with_short_alias_rules() {
        let unique = map(&[
            (
                "shared.xml",
                r#"<module name="shared" export="var:hp"><var name="hp" type="int">1</var></module>"#,
            ),
            (
                "main.xml",
                r#"
<!-- import shared from shared.xml -->
<module name="main" export="script:main">
<script name="main"><text>${hp + shared.hp}</text></script>
</module>
"#,
            ),
        ]);
        let unique_bundle = compile_project_bundle_from_xml_map(&unique).expect("compile");
        let unique_main = unique_bundle.scripts.get("main.main").expect("main");
        assert!(unique_main.visible_module_vars.contains_key("shared.hp"));
        assert!(!unique_main.visible_module_vars.contains_key("hp"));

        let conflict = map(&[
            (
                "a.xml",
                r#"<module name="a" export="var:hp"><var name="hp" type="int">1</var></module>"#,
            ),
            (
                "b.xml",
                r#"<module name="b" export="var:hp"><var name="hp" type="int">2</var></module>"#,
            ),
            (
                "main.xml",
                r#"
<!-- import a from a.xml -->
<!-- import b from b.xml -->
<module name="main" export="script:main">
<script name="main"><text>${a.hp + b.hp}</text></script>
</module>
"#,
            ),
        ]);
        let conflict_bundle = compile_project_bundle_from_xml_map(&conflict).expect("compile");
        let conflict_main = conflict_bundle.scripts.get("main.main").expect("main");
        assert!(conflict_main.visible_module_vars.contains_key("a.hp"));
        assert!(conflict_main.visible_module_vars.contains_key("b.hp"));
        assert!(!conflict_main.visible_module_vars.contains_key("hp"));
    }

    #[test]
    fn compile_bundle_supports_explicit_alias_directives() {
        let files = map(&[
            (
                "shared.xml",
                r#"
<module name="shared" export="type:Unit;var:hp;const:BASE">
  <type name="Unit">
    <field name="hp" type="int"/>
  </type>
  <var name="hp" type="int">10</var>
  <const name="BASE" type="int">2</const>
</module>
"#,
            ),
            (
                "main.xml",
                r#"
<!-- import shared from shared.xml -->
<!-- alias shared.Unit as Hero -->
<!-- alias shared.hp as health -->
<!-- alias shared.BASE as base -->
<module name="main" export="script:main">
  <script name="main">
    <temp name="hero" type="Hero">#{hp: health + base}</temp>
    <text>${hero.hp}</text>
  </script>
</module>
"#,
            ),
        ]);

        let bundle = compile_project_bundle_from_xml_map(&files).expect("compile should pass");
        let main = bundle
            .scripts
            .get("main.main")
            .expect("main script should exist");
        assert_eq!(
            main.visible_module_vars
                .get("health")
                .expect("module var alias should exist")
                .qualified_name,
            "shared.hp"
        );
        assert_eq!(
            main.visible_module_consts
                .get("base")
                .expect("module const alias should exist")
                .qualified_name,
            "shared.BASE"
        );
    }

    #[test]
    fn compile_bundle_handles_mixed_script_and_module_sources_for_alias_directives() {
        // Test that collect_module_alias_directives_by_namespace handles:
        // - Line 105: script root (not module) is skipped
        // - Line 108: module without name attribute is skipped
        let files = map(&[
            // This is a script, not a module - should be skipped (line 105)
            (
                "standalone.xml",
                r#"<script name="standalone"><text>alone</text></script>"#,
            ),
            (
                "main.xml",
                r#"<module name="main" export="script:main"><script name="main"><text>x</text></script></module>"#,
            ),
        ]);
        // Should compile without error - script sources are skipped
        let result = compile_project_bundle_from_xml_map(&files);
        assert!(result.is_ok(), "mixed sources should compile");
    }

    #[test]
    fn compile_bundle_supports_explicit_type_aliases_in_type_positions() {
        let files = map(&[
            (
                "ids.xml",
                r#"
<module name="ids" export="enum:LocationId,MessageKey">
  <enum name="LocationId">
    <member name="Home"/>
  </enum>
  <enum name="MessageKey">
    <member name="Ping"/>
  </enum>
</module>
"#,
            ),
            (
                "main.xml",
                r#"
<!-- import ids from ids.xml -->
<!-- alias ids.LocationId -->
<!-- alias ids.MessageKey -->
<module name="main" export="script:main;function:check;type:Pair">
  <type name="Pair">
    <field name="loc" type="LocationId"/>
    <field name="msg" type="MessageKey"/>
  </type>
  <function name="check" args="MessageKey:message_key,LocationId:location_id" return_type="boolean">
    return message_key == MessageKey.Ping AND location_id == LocationId.Home;
  </function>
  <script name="main">
    <text>ok</text>
  </script>
</module>
"#,
            ),
        ]);

        let bundle = compile_project_bundle_from_xml_map(&files).expect("compile should pass");
        assert!(bundle.scripts.contains_key("main.main"));
    }

    #[test]
    fn compile_bundle_keeps_alias_file_local_for_imported_module_type_resolution() {
        let files = map(&[
            (
                "ids.xml",
                r#"
<module name="ids" export="enum:LocationId">
  <enum name="LocationId">
    <member name="Home"/>
  </enum>
</module>
"#,
            ),
            (
                "map_data.xml",
                r#"
<!-- import ids from ids.xml -->
<!-- alias ids.LocationId -->
<module name="map_data" export="type:Node">
  <type name="Node">
    <field name="location_id" type="LocationId"/>
  </type>
</module>
"#,
            ),
            (
                "actions.xml",
                r#"
<!-- import ids from ids.xml -->
<!-- import map_data from map_data.xml -->
<module name="actions" export="script:main">
  <script name="main" args="ids.LocationId:from_id">
    <text>${from_id}</text>
  </script>
</module>
"#,
            ),
        ]);

        let bundle = compile_project_bundle_from_xml_map(&files).expect("compile should pass");
        assert!(bundle.scripts.contains_key("actions.main"));
    }

    #[test]
    fn compile_bundle_resolves_imported_public_const_with_local_short_array_type() {
        let files = map(&[
            (
                "ids.xml",
                r#"
<module name="ids" export="enum:LocationId">
  <enum name="LocationId">
    <member name="Home"/>
  </enum>
</module>
"#,
            ),
            (
                "map_data.xml",
                r#"
<!-- import ids from ids.xml -->
<!-- alias ids.LocationId -->
<module name="map_data" export="type:Node;const:nodes">
  <type name="Node">
    <field name="location_id" type="LocationId"/>
  </type>
  <const name="nodes" type="Node[]">[
    #{location_id: LocationId.Home}
  ]</const>
</module>
"#,
            ),
            (
                "app.xml",
                r#"
<!-- import map_data from map_data.xml -->
<module name="app" export="script:main">
  <script name="main">
    <text>ok</text>
  </script>
</module>
"#,
            ),
        ]);

        let bundle = compile_project_bundle_from_xml_map(&files).expect("compile should pass");
        assert!(bundle.scripts.contains_key("app.main"));
        assert!(bundle
            .module_const_declarations
            .contains_key("map_data.nodes"));
    }

    #[test]
    fn compile_bundle_propagates_alias_resolution_errors_without_panicking() {
        let files = map(&[
            (
                "shared.xml",
                r#"
<module name="shared" export="script:main">
  <script name="main"><text>ok</text></script>
</module>
"#,
            ),
            (
                "main.xml",
                r#"
<!-- import shared from shared.xml -->
<!-- alias shared.Missing -->
<module name="main" export="script:main">
  <script name="main"><text>ok</text></script>
</module>
"#,
            ),
        ]);

        let error = compile_project_bundle_from_xml_map(&files)
            .expect_err("unknown alias target should return a structured error");
        assert_eq!(error.code, "ALIAS_TARGET_NOT_FOUND");
        assert!(error.message.contains("main.xml"));
    }

    #[test]
    fn compile_bundle_requires_temp_and_enforces_access_visibility() {
        let removed_var = map(&[(
            "main.xml",
            r#"<module name="main" export="script:main"><script name="main"><var name="x" type="int">1</var></script></module>"#,
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
<module name="main" export="script:main">
<script name="main"><text>${shared.hp}</text></script>
</module>
"#,
            ),
        ]);
        let private_bundle = compile_project_bundle_from_xml_map(&private_import).expect("compile");
        let private_main = private_bundle.scripts.get("main.main").expect("main");
        assert!(!private_main.visible_module_vars.contains_key("shared.hp"));

        let public_import = map(&[
            (
                "shared.xml",
                r#"<module name="shared" export="var:hp"><var name="hp" type="int">1</var></module>"#,
            ),
            (
                "main.xml",
                r#"
<!-- import shared from shared.xml -->
<module name="main" export="script:main">
<script name="main"><text>${shared.hp}</text></script>
</module>
"#,
            ),
        ]);
        let public_bundle = compile_project_bundle_from_xml_map(&public_import).expect("compile");
        let public_main = public_bundle.scripts.get("main.main").expect("main");
        assert!(public_main.visible_module_vars.contains_key("shared.hp"));
    }

    #[test]
    fn compile_bundle_tracks_script_access_metadata() {
        let files = map(&[(
            "main.xml",
            r#"<module name="main" export="script:main">
<script name="main"><text>pub</text></script>
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
    fn pipeline_propagates_resolve_visible_module_symbols_errors() {
        // Test duplicate type declaration error through resolve_visible_module_symbols
        // Two different modules define types with the same qualified name
        // When main imports both, resolve_visible_module_symbols sees the conflict
        let conflict = map(&[
            (
                "a.xml",
                r#"<module name="shared" export="type:Obj">
<type name="Obj"><field name="x" type="int"/></type>
</module>"#,
            ),
            (
                "b.xml",
                r#"<module name="shared" export="type:Obj">
<type name="Obj"><field name="y" type="int"/></type>
</module>"#,
            ),
            (
                "main.xml",
                r#"
<!-- import shared from a.xml -->
<!-- import shared from b.xml -->
<module name="main" export="script:main">
<script name="main"><text>Hello</text></script>
</module>
"#,
            ),
        ]);
        let error = compile_project_bundle_from_xml_map(&conflict)
            .expect_err("duplicate type across imported modules should fail");
        assert_eq!(error.code, "TYPE_DECL_DUPLICATE");
    }

    #[test]
    fn pipeline_resolve_visible_module_symbols_error_propagates_to_script() {
        // Test that resolve_visible_module_symbols error propagates through .map_err at line 47
        // This creates a module with duplicate type - triggers TYPE_DECL_DUPLICATE at module_resolver.rs line 420
        let files = map(&[
            (
                "a.xml",
                r#"<module name="shared" export="type:Obj">
<type name="Obj"><field name="x" type="int"/></type>
</module>"#,
            ),
            (
                "b.xml",
                r#"<module name="shared" export="type:Obj">
<type name="Obj"><field name="y" type="int"/></type>
</module>"#,
            ),
            (
                "main.xml",
                r#"
<!-- import shared from a.xml -->
<!-- import shared from b.xml -->
<module name="main" export="script:main">
<script name="main"><text>Hello</text></script>
</module>
"#,
            ),
        ]);
        let error = compile_project_bundle_from_xml_map(&files)
            .expect_err("duplicate type across imported modules should fail");
        // The error should propagate through resolve_visible_module_symbols .map_err
        assert_eq!(error.code, "TYPE_DECL_DUPLICATE");
    }

    #[test]
    fn pipeline_propagates_function_decl_duplicate_error() {
        // Test duplicate function declaration error through resolve_visible_module_symbols
        // Two different modules define functions with the same qualified name "shared.foo"
        let files = map(&[
            (
                "a.xml",
                r#"<module name="shared" export="function:foo">
<function name="foo" args="int:x" return_type="int">return x + 1;</function>
</module>"#,
            ),
            (
                "b.xml",
                r#"<module name="shared" export="function:foo">
<function name="foo" args="int:y" return_type="int">return y + 2;</function>
</module>"#,
            ),
            (
                "main.xml",
                r#"
<module name="main" export="script:main">
<script name="main"><text>Hello</text></script>
</module>
"#,
            ),
        ]);
        let error = compile_project_bundle_from_xml_map(&files)
            .expect_err("duplicate function across imported modules should fail");
        assert_eq!(error.code, "FUNCTION_DECL_DUPLICATE");
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
                alias_directives: Vec::new(),
            },
        )]);
        let error = parse_module_scripts(&bad_sources).expect_err("module parse should fail");
        assert_eq!(error.code, "XML_MISSING_ATTR");

        let json_sources = BTreeMap::from([(
            "game.json".to_string(),
            SourceFile {
                kind: SourceKind::Json,
                imports: Vec::new(),
                alias_directives: Vec::new(),
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
