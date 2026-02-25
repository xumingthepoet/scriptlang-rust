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

    let mut scripts = BTreeMap::new();

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

        let reachable = collect_reachable_files(file_path, &sources);
        let (visible_types, visible_functions) = resolve_visible_defs(&reachable, &defs_by_path)?;
        let visible_json_symbols = collect_visible_json_symbols(&reachable, &sources)?;

        let ir = compile_script(
            file_path,
            script_root,
            &visible_types,
            &visible_functions,
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
    })
}

