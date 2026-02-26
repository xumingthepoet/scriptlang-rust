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
            resolve_visible_defs(reachable, &defs_by_path)?;
        let visible_json_symbols = collect_visible_json_symbols(reachable, &sources)?;

        let ir = compile_script(
            file_path,
            script_root,
            &visible_types,
            &visible_functions,
            &visible_defs_globals,
            &visible_json_symbols,
            &all_json_symbols,
        )?;

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

#[cfg(test)]
mod pipeline_tests {
    use super::*;
    use crate::compiler_test_support::*;
    use std::fs;

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
    fn compile_bundle_supports_all_example_scenarios() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let examples_root = manifest_dir
            .join("..")
            .join("..")
            .join("examples");
    
        let mut dirs = fs::read_dir(&examples_root)
            .expect("examples root should exist")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect::<Vec<_>>();
        dirs.sort();
    
        assert!(!dirs.is_empty(), "examples should not be empty");
    
        for directory in dirs {
            let mut files = BTreeMap::new();
            read_sources_recursive(&directory, &directory, &mut files)
                .expect("read sources should pass");
            let compiled =
                compile_project_bundle_from_xml_map(&files).expect("example should compile");
            assert!(!compiled.scripts.is_empty());
        }
    }

    #[test]
    fn compile_scripts_from_xml_map_returns_script_only_bundle() {
        let files = sources_from_example_dir("15-entry-override-recursive");
        let scripts = compile_project_scripts_from_xml_map(&files).expect("compile should pass");
        assert!(scripts.contains_key("main"));
        assert!(scripts.contains_key("alt"));
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

}
