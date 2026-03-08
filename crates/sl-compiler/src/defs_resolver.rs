use crate::*;

struct ParsedModuleHeader {
    namespace: String,
    default_access: AccessLevel,
}

enum ParsedModuleChild {
    Type(ParsedTypeDecl),
    Function(ParsedFunctionDecl),
    DefsGlobalVar(ParsedDefsGlobalVarDecl),
    DefsGlobalConst(ParsedDefsGlobalConstDecl),
    Script(ParsedModuleScript),
}

pub(crate) fn parse_defs_files(
    sources: &BTreeMap<String, SourceFile>,
) -> Result<BTreeMap<String, DefsDeclarations>, ScriptLangError> {
    let mut defs_by_path = BTreeMap::new();

    for (file_path, source) in sources {
        if !matches!(source.kind, SourceKind::ModuleXml) {
            continue;
        }

        let module = parse_module_source(source, file_path)?;
        defs_by_path.insert(file_path.clone(), module.defs);
    }

    Ok(defs_by_path)
}

pub(crate) fn parse_module_scripts(
    sources: &BTreeMap<String, SourceFile>,
) -> Result<BTreeMap<String, Vec<ParsedModuleScript>>, ScriptLangError> {
    let mut scripts_by_path = BTreeMap::new();

    for (file_path, source) in sources {
        if !matches!(source.kind, SourceKind::ModuleXml) {
            continue;
        }

        let module = parse_module_source(source, file_path)?;
        scripts_by_path.insert(file_path.clone(), module.scripts);
    }

    Ok(scripts_by_path)
}

fn parse_module_source(
    source: &SourceFile,
    file_path: &str,
) -> Result<ModuleDeclarations, ScriptLangError> {
    if !matches!(source.kind, SourceKind::ModuleXml) {
        return Err(ScriptLangError::new(
            "SOURCE_KIND_UNSUPPORTED",
            format!(
                "Unsupported source kind for module parsing in file \"{}\".",
                file_path
            ),
        ));
    }

    let root = source
        .xml_root
        .as_ref()
        .expect("module sources should always carry parsed xml root");

    if root.name != "module" {
        return Err(ScriptLangError::with_span(
            "XML_ROOT_INVALID",
            format!(
                "Expected <module> root in file \"{}\", got <{}>.",
                file_path, root.name
            ),
            root.location.clone(),
        ));
    }

    let ParsedModuleHeader {
        namespace,
        default_access,
    } = parse_module_header(root, file_path)?;

    let mut type_decls = Vec::new();
    let mut function_decls = Vec::new();
    let mut defs_global_var_decls = Vec::new();
    let mut defs_global_const_decls = Vec::new();
    let mut scripts = Vec::new();

    for child in element_children(root) {
        match parse_module_child(child, root, file_path, &namespace, default_access)? {
            ParsedModuleChild::Type(decl) => type_decls.push(decl),
            ParsedModuleChild::Function(decl) => function_decls.push(decl),
            ParsedModuleChild::DefsGlobalVar(decl) => defs_global_var_decls.push(decl),
            ParsedModuleChild::DefsGlobalConst(decl) => defs_global_const_decls.push(decl),
            ParsedModuleChild::Script(script) => scripts.push(script),
        }
    }

    Ok(ModuleDeclarations {
        defs: DefsDeclarations {
            type_decls,
            function_decls,
            defs_global_var_decls,
            defs_global_const_decls,
        },
        scripts,
    })
}

fn parse_module_header(
    root: &XmlElementNode,
    file_path: &str,
) -> Result<ParsedModuleHeader, ScriptLangError> {
    let namespace = get_required_non_empty_attr(root, "name")
        .map_err(|error| with_file_context(error, file_path))?;
    assert_name_not_reserved(&namespace, "module", root.location.clone())
        .map_err(|error| with_file_context(error, file_path))?;
    if has_attr(root, "defaul_access") {
        return Err(with_file_context(
            ScriptLangError::with_span(
                "XML_ATTR_NOT_ALLOWED",
                "Attribute \"defaul_access\" is invalid. Use \"default_access\".",
                root.location.clone(),
            ),
            file_path,
        ));
    }
    let default_access = parse_access_attr(root, "default_access", AccessLevel::Private)
        .map_err(|error| with_file_context(error, file_path))?;
    Ok(ParsedModuleHeader {
        namespace,
        default_access,
    })
}

fn parse_module_child(
    child: &XmlElementNode,
    root: &XmlElementNode,
    file_path: &str,
    namespace: &str,
    default_access: AccessLevel,
) -> Result<ParsedModuleChild, ScriptLangError> {
    match child.name.as_str() {
        "type" => parse_type_declaration_node_with_namespace(child, namespace, default_access)
            .map(ParsedModuleChild::Type)
            .map_err(|error| with_file_context(error, file_path)),
        "function" => {
            parse_function_declaration_node_with_namespace(child, namespace, default_access)
                .map(ParsedModuleChild::Function)
                .map_err(|error| with_file_context(error, file_path))
        }
        "var" => parse_defs_global_var_declaration(child, namespace, default_access)
            .map(ParsedModuleChild::DefsGlobalVar)
            .map_err(|error| with_file_context(error, file_path)),
        "const" => parse_defs_global_const_declaration(child, namespace, default_access)
            .map(ParsedModuleChild::DefsGlobalConst)
            .map_err(|error| with_file_context(error, file_path)),
        "script" => {
            let script_name = get_required_non_empty_attr(child, "name")
                .map_err(|error| with_file_context(error, file_path))?;
            assert_name_not_reserved(&script_name, "script", child.location.clone())
                .map_err(|error| with_file_context(error, file_path))?;
            let access = parse_access_attr(child, "access", default_access)
                .map_err(|error| with_file_context(error, file_path))?;
            Ok(ParsedModuleChild::Script(ParsedModuleScript {
                qualified_script_name: format!("{}.{}", namespace, script_name),
                access,
                root: child.clone(),
            }))
        }
        _ => Err(with_file_context(
            ScriptLangError::with_span(
                "XML_MODULE_CHILD_INVALID",
                format!("Unsupported child <{}> under <{}>.", child.name, root.name),
                child.location.clone(),
            ),
            file_path,
        )),
    }
}

fn with_file_context(error: ScriptLangError, file_path: &str) -> ScriptLangError {
    with_file_context_shared(error, file_path)
}

pub(crate) fn parse_defs_global_var_declaration(
    node: &XmlElementNode,
    namespace: &str,
    default_access: AccessLevel,
) -> Result<ParsedDefsGlobalVarDecl, ScriptLangError> {
    let parsed = parse_defs_global_binding_declaration(node, namespace, default_access, "var")?;
    Ok(ParsedDefsGlobalVarDecl {
        namespace: parsed.namespace,
        name: parsed.name,
        qualified_name: parsed.qualified_name,
        access: parsed.access,
        type_expr: parsed.type_expr,
        initial_value_expr: parsed.initial_value_expr,
        location: parsed.location,
    })
}

pub(crate) fn parse_defs_global_const_declaration(
    node: &XmlElementNode,
    namespace: &str,
    default_access: AccessLevel,
) -> Result<ParsedDefsGlobalConstDecl, ScriptLangError> {
    let parsed = parse_defs_global_binding_declaration(node, namespace, default_access, "const")?;
    Ok(ParsedDefsGlobalConstDecl {
        namespace: parsed.namespace,
        name: parsed.name,
        qualified_name: parsed.qualified_name,
        access: parsed.access,
        type_expr: parsed.type_expr,
        initial_value_expr: parsed.initial_value_expr,
        location: parsed.location,
    })
}

fn parse_defs_global_binding_declaration(
    node: &XmlElementNode,
    namespace: &str,
    default_access: AccessLevel,
    tag_name: &str,
) -> Result<ParsedDefsGlobalVarDecl, ScriptLangError> {
    let name = get_required_non_empty_attr(node, "name")?;
    assert_name_not_reserved(&name, "defs global", node.location.clone())?;
    let access = parse_access_attr(node, "access", default_access)?;

    let type_raw = get_required_non_empty_attr(node, "type")?;
    let type_expr = parse_type_expr(&type_raw, &node.location)?;

    if has_attr(node, "value") {
        return Err(ScriptLangError::with_span(
            "XML_ATTR_NOT_ALLOWED",
            format!(
                "Attribute \"value\" is not allowed on <{}>. Use inline content instead.",
                tag_name
            ),
            node.location.clone(),
        ));
    }

    if let Some(child) = element_children(node).next() {
        return Err(ScriptLangError::with_span(
            "XML_VAR_CHILD_INVALID",
            format!(
                "<{}> cannot contain child element <{}>. Use inline expression text only.",
                tag_name, child.name
            ),
            child.location.clone(),
        ));
    }

    let inline = inline_text_content(node);
    let initial_value_expr = if inline.trim().is_empty() {
        None
    } else {
        Some(inline.trim().to_string())
    };

    Ok(ParsedDefsGlobalVarDecl {
        namespace: namespace.to_string(),
        name: name.clone(),
        qualified_name: format!("{}.{}", namespace, name),
        access,
        type_expr,
        initial_value_expr,
        location: node.location.clone(),
    })
}

#[allow(dead_code)]
pub(crate) fn collect_global_json(
    sources: &BTreeMap<String, SourceFile>,
) -> Result<BTreeMap<String, SlValue>, ScriptLangError> {
    let mut out = BTreeMap::new();

    for (file_path, source) in sources {
        if !matches!(source.kind, SourceKind::Json) {
            continue;
        }
        let symbol = parse_json_global_symbol(file_path)?;
        if out.contains_key(&symbol) {
            return Err(ScriptLangError::new(
                "JSON_SYMBOL_DUPLICATE",
                format!("Duplicate JSON symbol \"{}\".", symbol),
            ));
        }
        let value = source.json_value.clone().ok_or(ScriptLangError::new(
            "JSON_MISSING_VALUE",
            "Missing JSON value.",
        ))?;
        out.insert(symbol, value);
    }

    Ok(out)
}

#[allow(dead_code)]
pub(crate) fn collect_visible_json_symbols(
    reachable: &BTreeSet<String>,
    sources: &BTreeMap<String, SourceFile>,
) -> Result<Vec<String>, ScriptLangError> {
    let mut symbols = Vec::new();
    let mut seen = HashSet::new();

    for file_path in reachable {
        let Some(source) = sources.get(file_path) else {
            continue;
        };
        if !matches!(source.kind, SourceKind::Json) {
            continue;
        }

        let symbol = parse_json_global_symbol(file_path)?;
        if !seen.insert(symbol.clone()) {
            return Err(ScriptLangError::new(
                "JSON_SYMBOL_DUPLICATE",
                format!("Duplicate JSON symbol \"{}\" in visible closure.", symbol),
            ));
        }
        symbols.push(symbol);
    }

    symbols.sort();
    Ok(symbols)
}

#[allow(dead_code)]
pub(crate) fn parse_json_global_symbol(file_path: &str) -> Result<String, ScriptLangError> {
    let path = Path::new(file_path);
    let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
        return Err(ScriptLangError::new(
            "JSON_SYMBOL_INVALID",
            format!("Invalid JSON file name: {}", file_path),
        ));
    };

    if !json_symbol_regex().is_match(stem) {
        return Err(ScriptLangError::new(
            "JSON_SYMBOL_INVALID",
            format!("JSON basename \"{}\" is not a valid identifier.", stem),
        ));
    }

    assert_name_not_reserved(stem, "json symbol", SourceSpan::synthetic())?;
    Ok(stem.to_string())
}

#[allow(dead_code)]
pub(crate) fn json_symbol_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"^[$A-Za-z_][$0-9A-Za-z_]*$").expect("json symbol regex must compile")
    })
}

pub(crate) fn resolve_visible_defs(
    reachable: &BTreeSet<String>,
    defs_by_path: &BTreeMap<String, DefsDeclarations>,
    local_module_name: Option<&str>,
) -> Result<VisibleDefsResolution, ScriptLangError> {
    let mut type_decls_map: BTreeMap<String, ParsedTypeDecl> = BTreeMap::new();
    let mut local_type_short_candidates: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut namespace_type_aliases: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();

    for path in reachable {
        let Some(defs) = defs_by_path.get(path) else {
            continue;
        };
        for decl in &defs.type_decls {
            let is_local = local_module_name.is_some_and(|module_name| {
                decl.qualified_name.starts_with(&format!("{module_name}."))
            });
            if !is_local && decl.access != AccessLevel::Public {
                continue;
            }
            if type_decls_map.contains_key(&decl.qualified_name) {
                return Err(ScriptLangError::with_span(
                    "TYPE_DECL_DUPLICATE",
                    format!("Duplicate type declaration \"{}\".", decl.qualified_name),
                    decl.location.clone(),
                ));
            }
            type_decls_map.insert(decl.qualified_name.clone(), decl.clone());
            if let Some((namespace, _)) = decl.qualified_name.split_once('.') {
                namespace_type_aliases
                    .entry(namespace.to_string())
                    .or_default()
                    .insert(decl.name.clone(), decl.qualified_name.clone());
            }
            if is_local {
                local_type_short_candidates
                    .entry(decl.name.clone())
                    .or_default()
                    .push(decl.qualified_name.clone());
            }
        }
    }

    let type_aliases = local_type_short_candidates
        .into_iter()
        .map(|(short, qualified)| (short, qualified[0].clone()))
        .collect::<BTreeMap<_, _>>();

    let mut resolved_types: BTreeMap<String, ScriptType> = BTreeMap::new();
    let mut visiting = HashSet::new();

    for type_name in type_decls_map.keys() {
        let namespace = type_name
            .split_once('.')
            .map(|(namespace, _)| namespace)
            .unwrap_or_default();
        let aliases = namespace_type_aliases
            .get(namespace)
            .cloned()
            .unwrap_or_default();
        resolve_named_type_with_aliases(
            type_name,
            &type_decls_map,
            &aliases,
            &mut resolved_types,
            &mut visiting,
        )?;
    }

    let mut visible_types = resolved_types.clone();
    for (alias, qualified_name) in &type_aliases {
        let ty = resolved_types
            .get(qualified_name)
            .cloned()
            .expect("resolved type aliases must point to resolved types");
        visible_types.insert(alias.clone(), ty);
    }

    let mut functions: BTreeMap<String, FunctionDecl> = BTreeMap::new();
    let mut function_short_candidates: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for path in reachable {
        let Some(defs) = defs_by_path.get(path) else {
            continue;
        };

        for decl in &defs.function_decls {
            let is_local = local_module_name.is_some_and(|module_name| {
                decl.qualified_name.starts_with(&format!("{module_name}."))
            });
            if !is_local && decl.access != AccessLevel::Public {
                continue;
            }
            if functions.contains_key(&decl.qualified_name) {
                return Err(ScriptLangError::with_span(
                    "FUNCTION_DECL_DUPLICATE",
                    format!(
                        "Duplicate function declaration \"{}\".",
                        decl.qualified_name
                    ),
                    decl.location.clone(),
                ));
            }
            let function_namespace = decl
                .qualified_name
                .split_once('.')
                .map(|(namespace, _)| namespace)
                .unwrap_or_default();

            let mut params = Vec::new();
            for param in &decl.params {
                params.push(FunctionParam {
                    name: param.name.clone(),
                    r#type: resolve_type_expr_in_namespace(
                        &param.type_expr,
                        &visible_types,
                        function_namespace,
                        &param.location,
                    )?,
                    location: param.location.clone(),
                });
            }

            let rb = &decl.return_binding;
            let return_type = resolve_type_expr_in_namespace(
                &rb.type_expr,
                &visible_types,
                function_namespace,
                &rb.location,
            )?;

            functions.insert(
                decl.qualified_name.clone(),
                FunctionDecl {
                    name: decl.qualified_name.clone(),
                    params,
                    return_binding: FunctionReturn {
                        name: decl.return_binding.name.clone(),
                        r#type: return_type,
                        location: decl.return_binding.location.clone(),
                    },
                    code: decl.code.clone(),
                    location: decl.location.clone(),
                },
            );
            if is_local {
                function_short_candidates
                    .entry(decl.name.clone())
                    .or_default()
                    .push(decl.qualified_name.clone());
            }
        }
    }

    for (alias, qualified_names) in function_short_candidates {
        let qualified = &qualified_names[0];
        let decl = functions
            .get(qualified)
            .cloned()
            .expect("qualified function should exist in function map");
        if !functions.contains_key(&alias) {
            functions.insert(
                alias.clone(),
                FunctionDecl {
                    name: alias,
                    ..decl
                },
            );
        }
    }

    let mut defs_globals_qualified = BTreeMap::new();
    let mut defs_global_short_candidates: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for path in reachable {
        let Some(defs) = defs_by_path.get(path) else {
            continue;
        };

        for decl in &defs.defs_global_var_decls {
            let is_local = local_module_name == Some(decl.namespace.as_str());
            if !is_local && decl.access != AccessLevel::Public {
                continue;
            }
            if defs_globals_qualified.contains_key(&decl.qualified_name) {
                return Err(ScriptLangError::with_span(
                    "DEFS_GLOBAL_VAR_DUPLICATE",
                    format!(
                        "Duplicate defs global variable declaration \"{}\".",
                        decl.qualified_name
                    ),
                    decl.location.clone(),
                ));
            }
            defs_globals_qualified.insert(
                decl.qualified_name.clone(),
                DefsGlobalVarDecl {
                    namespace: decl.namespace.clone(),
                    name: decl.name.clone(),
                    qualified_name: decl.qualified_name.clone(),
                    access: decl.access,
                    r#type: resolve_type_expr_in_namespace(
                        &decl.type_expr,
                        &visible_types,
                        &decl.namespace,
                        &decl.location,
                    )?,
                    initial_value_expr: decl.initial_value_expr.clone(),
                    location: decl.location.clone(),
                },
            );
            if is_local {
                defs_global_short_candidates
                    .entry(decl.name.clone())
                    .or_default()
                    .push(decl.qualified_name.clone());
            }
        }
    }

    let mut defs_globals = defs_globals_qualified.clone();
    for (alias, qualified_names) in defs_global_short_candidates {
        let qualified_name = &qualified_names[0];
        let decl = defs_globals_qualified
            .get(qualified_name)
            .cloned()
            .expect("defs global alias target should exist");
        defs_globals.entry(alias).or_insert(decl);
    }

    let mut defs_consts_qualified = BTreeMap::new();
    let mut defs_const_short_candidates: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for path in reachable {
        let Some(defs) = defs_by_path.get(path) else {
            continue;
        };

        for decl in &defs.defs_global_const_decls {
            let is_local = local_module_name == Some(decl.namespace.as_str());
            if !is_local && decl.access != AccessLevel::Public {
                continue;
            }
            if defs_consts_qualified.contains_key(&decl.qualified_name) {
                return Err(ScriptLangError::with_span(
                    "DEFS_GLOBAL_CONST_DUPLICATE",
                    format!(
                        "Duplicate defs global const declaration \"{}\".",
                        decl.qualified_name
                    ),
                    decl.location.clone(),
                ));
            }
            defs_consts_qualified.insert(
                decl.qualified_name.clone(),
                DefsGlobalConstDecl {
                    namespace: decl.namespace.clone(),
                    name: decl.name.clone(),
                    qualified_name: decl.qualified_name.clone(),
                    access: decl.access,
                    r#type: resolve_type_expr_in_namespace(
                        &decl.type_expr,
                        &visible_types,
                        &decl.namespace,
                        &decl.location,
                    )?,
                    initial_value_expr: decl.initial_value_expr.clone(),
                    location: decl.location.clone(),
                },
            );
            if is_local {
                defs_const_short_candidates
                    .entry(decl.name.clone())
                    .or_default()
                    .push(decl.qualified_name.clone());
            }
        }
    }

    let mut defs_consts = defs_consts_qualified.clone();
    for (alias, qualified_names) in defs_const_short_candidates {
        let qualified_name = &qualified_names[0];
        let decl = defs_consts_qualified
            .get(qualified_name)
            .cloned()
            .expect("defs const alias target should exist");
        defs_consts.entry(alias).or_insert(decl);
    }

    Ok((visible_types, functions, defs_globals, defs_consts))
}

pub(crate) fn collect_functions_for_bundle(
    defs_by_path: &BTreeMap<String, DefsDeclarations>,
) -> Result<(BTreeMap<String, FunctionDecl>, BTreeSet<String>), ScriptLangError> {
    let mut type_decls_map: BTreeMap<String, ParsedTypeDecl> = BTreeMap::new();
    let mut type_short_candidates: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for defs in defs_by_path.values() {
        for decl in &defs.type_decls {
            if type_decls_map.contains_key(&decl.qualified_name) {
                return Err(ScriptLangError::with_span(
                    "TYPE_DECL_DUPLICATE",
                    format!("Duplicate type declaration \"{}\".", decl.qualified_name),
                    decl.location.clone(),
                ));
            }
            type_decls_map.insert(decl.qualified_name.clone(), decl.clone());
            type_short_candidates
                .entry(decl.name.clone())
                .or_default()
                .push(decl.qualified_name.clone());
        }
    }

    let type_aliases = type_short_candidates
        .into_iter()
        .filter_map(|(short, qualified)| {
            if qualified.len() == 1 {
                Some((short, qualified[0].clone()))
            } else {
                None
            }
        })
        .collect::<BTreeMap<_, _>>();

    let mut resolved_types: BTreeMap<String, ScriptType> = BTreeMap::new();
    let mut visiting = HashSet::new();
    for type_name in type_decls_map.keys() {
        resolve_named_type_with_aliases(
            type_name,
            &type_decls_map,
            &type_aliases,
            &mut resolved_types,
            &mut visiting,
        )?;
    }

    let mut visible_types = resolved_types.clone();
    for (alias, qualified_name) in &type_aliases {
        let ty = resolved_types
            .get(qualified_name)
            .cloned()
            .expect("resolved type aliases must point to resolved types");
        visible_types.insert(alias.clone(), ty);
    }

    let mut functions = BTreeMap::new();
    let mut public_functions = BTreeSet::new();
    for defs in defs_by_path.values() {
        for decl in &defs.function_decls {
            if functions.contains_key(&decl.qualified_name) {
                return Err(ScriptLangError::with_span(
                    "FUNCTION_DECL_DUPLICATE",
                    format!(
                        "Duplicate function declaration \"{}\".",
                        decl.qualified_name
                    ),
                    decl.location.clone(),
                ));
            }

            let function_namespace = decl
                .qualified_name
                .split_once('.')
                .map(|(namespace, _)| namespace)
                .unwrap_or_default();

            let mut params = Vec::new();
            for param in &decl.params {
                params.push(FunctionParam {
                    name: param.name.clone(),
                    r#type: resolve_type_expr_in_namespace(
                        &param.type_expr,
                        &visible_types,
                        function_namespace,
                        &param.location,
                    )?,
                    location: param.location.clone(),
                });
            }

            let return_type = resolve_type_expr_in_namespace(
                &decl.return_binding.type_expr,
                &visible_types,
                function_namespace,
                &decl.return_binding.location,
            )?;

            functions.insert(
                decl.qualified_name.clone(),
                FunctionDecl {
                    name: decl.qualified_name.clone(),
                    params,
                    return_binding: FunctionReturn {
                        name: decl.return_binding.name.clone(),
                        r#type: return_type,
                        location: decl.return_binding.location.clone(),
                    },
                    code: decl.code.clone(),
                    location: decl.location.clone(),
                },
            );
            if decl.access == AccessLevel::Public {
                public_functions.insert(decl.qualified_name.clone());
            }
        }
    }

    Ok((functions, public_functions))
}

pub(crate) fn collect_defs_globals_for_bundle(
    defs_by_path: &BTreeMap<String, DefsDeclarations>,
) -> Result<(BTreeMap<String, DefsGlobalVarDecl>, Vec<String>), ScriptLangError> {
    let mut type_decls_map: BTreeMap<String, ParsedTypeDecl> = BTreeMap::new();
    let mut type_short_candidates: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for defs in defs_by_path.values() {
        for decl in &defs.type_decls {
            if type_decls_map.contains_key(&decl.qualified_name) {
                return Err(ScriptLangError::with_span(
                    "TYPE_DECL_DUPLICATE",
                    format!("Duplicate type declaration \"{}\".", decl.qualified_name),
                    decl.location.clone(),
                ));
            }
            type_decls_map.insert(decl.qualified_name.clone(), decl.clone());
            type_short_candidates
                .entry(decl.name.clone())
                .or_default()
                .push(decl.qualified_name.clone());
        }
    }

    let type_aliases = type_short_candidates
        .into_iter()
        .filter_map(|(short, qualified)| {
            if qualified.len() == 1 {
                Some((short, qualified[0].clone()))
            } else {
                None
            }
        })
        .collect::<BTreeMap<_, _>>();

    let mut resolved_types: BTreeMap<String, ScriptType> = BTreeMap::new();
    let mut visiting = HashSet::new();
    for type_name in type_decls_map.keys() {
        resolve_named_type_with_aliases(
            type_name,
            &type_decls_map,
            &type_aliases,
            &mut resolved_types,
            &mut visiting,
        )?;
    }

    let mut visible_types = resolved_types.clone();
    for (alias, qualified_name) in &type_aliases {
        let ty = resolved_types
            .get(qualified_name)
            .cloned()
            .expect("resolved type aliases must point to resolved types");
        visible_types.insert(alias.clone(), ty);
    }

    let mut defs_globals = BTreeMap::new();
    let mut init_order = Vec::new();
    for defs in defs_by_path.values() {
        for decl in &defs.defs_global_var_decls {
            if defs_globals.contains_key(&decl.qualified_name) {
                return Err(ScriptLangError::with_span(
                    "DEFS_GLOBAL_VAR_DUPLICATE",
                    format!(
                        "Duplicate defs global variable declaration \"{}\".",
                        decl.qualified_name
                    ),
                    decl.location.clone(),
                ));
            }
            defs_globals.insert(
                decl.qualified_name.clone(),
                DefsGlobalVarDecl {
                    namespace: decl.namespace.clone(),
                    name: decl.name.clone(),
                    qualified_name: decl.qualified_name.clone(),
                    access: decl.access,
                    r#type: resolve_type_expr(&decl.type_expr, &visible_types, &decl.location)?,
                    initial_value_expr: decl.initial_value_expr.clone(),
                    location: decl.location.clone(),
                },
            );
            init_order.push(decl.qualified_name.clone());
        }
    }

    validate_defs_global_init_order(&defs_globals, &init_order)?;
    Ok((defs_globals, init_order))
}

pub(crate) fn collect_defs_consts_for_bundle(
    defs_by_path: &BTreeMap<String, DefsDeclarations>,
    defs_globals: &BTreeMap<String, DefsGlobalVarDecl>,
) -> Result<(BTreeMap<String, DefsGlobalConstDecl>, Vec<String>), ScriptLangError> {
    let mut type_decls_map: BTreeMap<String, ParsedTypeDecl> = BTreeMap::new();
    let mut type_short_candidates: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for defs in defs_by_path.values() {
        for decl in &defs.type_decls {
            if type_decls_map.contains_key(&decl.qualified_name) {
                return Err(ScriptLangError::with_span(
                    "TYPE_DECL_DUPLICATE",
                    format!("Duplicate type declaration \"{}\".", decl.qualified_name),
                    decl.location.clone(),
                ));
            }
            type_decls_map.insert(decl.qualified_name.clone(), decl.clone());
            type_short_candidates
                .entry(decl.name.clone())
                .or_default()
                .push(decl.qualified_name.clone());
        }
    }

    let type_aliases = type_short_candidates
        .into_iter()
        .filter_map(|(short, qualified)| {
            if qualified.len() == 1 {
                Some((short, qualified[0].clone()))
            } else {
                None
            }
        })
        .collect::<BTreeMap<_, _>>();

    let mut resolved_types: BTreeMap<String, ScriptType> = BTreeMap::new();
    let mut visiting = HashSet::new();
    for type_name in type_decls_map.keys() {
        resolve_named_type_with_aliases(
            type_name,
            &type_decls_map,
            &type_aliases,
            &mut resolved_types,
            &mut visiting,
        )?;
    }

    let mut visible_types = resolved_types.clone();
    for (alias, qualified_name) in &type_aliases {
        let ty = resolved_types
            .get(qualified_name)
            .cloned()
            .expect("resolved type aliases must point to resolved types");
        visible_types.insert(alias.clone(), ty);
    }

    let mut defs_consts = BTreeMap::new();
    let mut init_order = Vec::new();
    for defs in defs_by_path.values() {
        for decl in &defs.defs_global_const_decls {
            if defs_consts.contains_key(&decl.qualified_name) {
                return Err(ScriptLangError::with_span(
                    "DEFS_GLOBAL_CONST_DUPLICATE",
                    format!(
                        "Duplicate defs global const declaration \"{}\".",
                        decl.qualified_name
                    ),
                    decl.location.clone(),
                ));
            }
            defs_consts.insert(
                decl.qualified_name.clone(),
                DefsGlobalConstDecl {
                    namespace: decl.namespace.clone(),
                    name: decl.name.clone(),
                    qualified_name: decl.qualified_name.clone(),
                    access: decl.access,
                    r#type: resolve_type_expr(&decl.type_expr, &visible_types, &decl.location)?,
                    initial_value_expr: decl.initial_value_expr.clone(),
                    location: decl.location.clone(),
                },
            );
            init_order.push(decl.qualified_name.clone());
        }
    }

    validate_defs_const_init_rules(&defs_consts, &init_order, defs_globals)?;
    Ok((defs_consts, init_order))
}

pub(crate) fn validate_defs_global_init_order(
    defs_globals: &BTreeMap<String, DefsGlobalVarDecl>,
    init_order: &[String],
) -> Result<(), ScriptLangError> {
    let mut name_candidates: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (qualified, decl) in defs_globals {
        name_candidates
            .entry(qualified.clone())
            .or_default()
            .push(qualified.clone());
        name_candidates
            .entry(decl.name.clone())
            .or_default()
            .push(qualified.clone());
    }
    let name_to_qualified = name_candidates
        .into_iter()
        .filter_map(|(name, candidates)| {
            if candidates.len() == 1 {
                Some((name, candidates[0].clone()))
            } else {
                None
            }
        })
        .collect::<BTreeMap<_, _>>();

    let mut initialized = BTreeSet::new();
    for qualified in init_order {
        let decl = defs_globals
            .get(qualified)
            .expect("init order should only contain declared defs globals");
        if let Some(expr) = &decl.initial_value_expr {
            let sanitized = sanitize_rhai_source(expr);
            for (name, target_qualified) in &name_to_qualified {
                if !contains_root_identifier(&sanitized, name) {
                    continue;
                }
                if !initialized.contains(target_qualified) {
                    return Err(ScriptLangError::with_span(
                        "DEFS_GLOBAL_INIT_ORDER",
                        format!(
                            "Defs global \"{}\" initializer references \"{}\" before initialization.",
                            qualified, name
                        ),
                        decl.location.clone(),
                    ));
                }
            }
        }
        initialized.insert(qualified.clone());
    }
    Ok(())
}

pub(crate) fn validate_defs_const_init_rules(
    defs_consts: &BTreeMap<String, DefsGlobalConstDecl>,
    init_order: &[String],
    defs_globals: &BTreeMap<String, DefsGlobalVarDecl>,
) -> Result<(), ScriptLangError> {
    let mut const_name_candidates: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (qualified, decl) in defs_consts {
        const_name_candidates
            .entry(qualified.clone())
            .or_default()
            .push(qualified.clone());
        const_name_candidates
            .entry(decl.name.clone())
            .or_default()
            .push(qualified.clone());
    }
    let const_name_to_qualified = const_name_candidates
        .into_iter()
        .filter_map(|(name, candidates)| {
            if candidates.len() == 1 {
                Some((name, candidates[0].clone()))
            } else {
                None
            }
        })
        .collect::<BTreeMap<_, _>>();

    let mut var_name_candidates: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (qualified, decl) in defs_globals {
        var_name_candidates
            .entry(qualified.clone())
            .or_default()
            .push(qualified.clone());
        var_name_candidates
            .entry(decl.name.clone())
            .or_default()
            .push(qualified.clone());
    }
    let var_name_to_qualified = var_name_candidates
        .into_iter()
        .filter_map(|(name, candidates)| {
            if candidates.len() == 1 {
                Some((name, candidates[0].clone()))
            } else {
                None
            }
        })
        .collect::<BTreeMap<_, _>>();

    let mut initialized = BTreeSet::new();
    for qualified in init_order {
        let decl = defs_consts
            .get(qualified)
            .expect("init order should only contain declared defs consts");
        if let Some(expr) = &decl.initial_value_expr {
            let sanitized = sanitize_rhai_source(expr);
            for (name, target_qualified) in &var_name_to_qualified {
                if contains_root_identifier(&sanitized, name) {
                    return Err(ScriptLangError::with_span(
                        "DEFS_CONST_INIT_REF_NON_CONST",
                        format!(
                            "Defs const \"{}\" initializer references mutable defs global \"{}\".",
                            qualified, name
                        ),
                        decl.location.clone(),
                    ));
                }
                if contains_root_identifier(&sanitized, target_qualified) {
                    return Err(ScriptLangError::with_span(
                        "DEFS_CONST_INIT_REF_NON_CONST",
                        format!(
                            "Defs const \"{}\" initializer references mutable defs global \"{}\".",
                            qualified, target_qualified
                        ),
                        decl.location.clone(),
                    ));
                }
            }
            for (name, target_qualified) in &const_name_to_qualified {
                if !contains_root_identifier(&sanitized, name) {
                    continue;
                }
                if !initialized.contains(target_qualified) {
                    return Err(ScriptLangError::with_span(
                        "DEFS_CONST_INIT_ORDER",
                        format!(
                            "Defs const \"{}\" initializer references \"{}\" before initialization.",
                            qualified, name
                        ),
                        decl.location.clone(),
                    ));
                }
            }
        }
        initialized.insert(qualified.clone());
    }

    Ok(())
}

#[cfg(test)]
mod defs_resolver_tests {
    use super::*;
    use crate::compiler_test_support::*;

    fn script_type_kind(ty: &ScriptType) -> &'static str {
        match ty {
            ScriptType::Primitive { .. } => "primitive",
            ScriptType::Array { .. } => "array",
            ScriptType::Map { .. } => "map",
            ScriptType::Object { .. } => "object",
        }
    }

    #[test]
    fn resolve_visible_defs_builds_function_signatures() {
        let span = SourceSpan::synthetic();
        let defs = DefsDeclarations {
            type_decls: vec![ParsedTypeDecl {
                name: "Obj".to_string(),
                qualified_name: "shared.Obj".to_string(),
                access: AccessLevel::Public,
                fields: vec![ParsedTypeFieldDecl {
                    name: "value".to_string(),
                    type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                    location: span.clone(),
                }],
                location: span.clone(),
            }],
            function_decls: vec![ParsedFunctionDecl {
                name: "make".to_string(),
                qualified_name: "shared.make".to_string(),
                access: AccessLevel::Public,
                params: vec![ParsedFunctionParamDecl {
                    name: "seed".to_string(),
                    type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                    location: span.clone(),
                }],
                return_binding: ParsedFunctionParamDecl {
                    name: "ret".to_string(),
                    type_expr: ParsedTypeExpr::Custom("Obj".to_string()),
                    location: span.clone(),
                },
                code: "ret = #{value: seed};".to_string(),
                location: span.clone(),
            }],
            defs_global_var_decls: Vec::new(),
            defs_global_const_decls: Vec::new(),
        };

        let reachable = BTreeSet::from(["shared.xml".to_string()]);
        let defs_by_path = BTreeMap::from([("shared.xml".to_string(), defs)]);

        let (types, functions, defs_globals, _defs_consts) =
            resolve_visible_defs(&reachable, &defs_by_path, Some("shared"))
                .expect("defs should resolve");
        assert!(types.contains_key("Obj"));
        let function = functions.get("make").expect("function should exist");
        assert_eq!(function.params.len(), 1);
        assert!(defs_globals.is_empty());
        assert_eq!(script_type_kind(&function.return_binding.r#type), "object");
    }

    #[test]
    fn resolve_visible_defs_handles_namespace_collisions_and_alias_edges() {
        let span = SourceSpan::synthetic();

        let duplicate_qualified = DefsDeclarations {
            type_decls: vec![ParsedTypeDecl {
                name: "T".to_string(),
                qualified_name: "shared.T".to_string(),
                access: AccessLevel::Public,
                fields: vec![ParsedTypeFieldDecl {
                    name: "v".to_string(),
                    type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                    location: span.clone(),
                }],
                location: span.clone(),
            }],
            function_decls: Vec::new(),
            defs_global_var_decls: Vec::new(),
            defs_global_const_decls: Vec::new(),
        };
        let duplicate_defs_by_path = BTreeMap::from([
            ("a.xml".to_string(), duplicate_qualified.clone()),
            ("b.xml".to_string(), duplicate_qualified),
        ]);
        let duplicate_reachable = BTreeSet::from(["a.xml".to_string(), "b.xml".to_string()]);
        let duplicate_error = resolve_visible_defs(
            &duplicate_reachable,
            &duplicate_defs_by_path,
            Some("shared"),
        )
        .expect_err("duplicate qualified type should fail");
        assert_eq!(duplicate_error.code, "TYPE_DECL_DUPLICATE");

        let defs_by_path = BTreeMap::from([
            (
                "a.xml".to_string(),
                DefsDeclarations {
                    type_decls: Vec::new(),
                    function_decls: vec![ParsedFunctionDecl {
                        name: "doit".to_string(),
                        qualified_name: "a.doit".to_string(),
                        access: AccessLevel::Public,
                        params: Vec::new(),
                        return_binding: ParsedFunctionParamDecl {
                            name: "out".to_string(),
                            type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                            location: span.clone(),
                        },
                        code: "out = 1;".to_string(),
                        location: span.clone(),
                    }],
                    defs_global_var_decls: Vec::new(),
                    defs_global_const_decls: Vec::new(),
                },
            ),
            (
                "b.xml".to_string(),
                DefsDeclarations {
                    type_decls: Vec::new(),
                    function_decls: vec![ParsedFunctionDecl {
                        name: "doit".to_string(),
                        qualified_name: "b.doit".to_string(),
                        access: AccessLevel::Public,
                        params: Vec::new(),
                        return_binding: ParsedFunctionParamDecl {
                            name: "out".to_string(),
                            type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                            location: span.clone(),
                        },
                        code: "out = 2;".to_string(),
                        location: span.clone(),
                    }],
                    defs_global_var_decls: Vec::new(),
                    defs_global_const_decls: Vec::new(),
                },
            ),
        ]);
        let reachable = BTreeSet::from(["a.xml".to_string(), "b.xml".to_string()]);
        let (_types, functions, defs_globals, _defs_consts) =
            resolve_visible_defs(&reachable, &defs_by_path, Some("a"))
                .expect("defs should resolve");
        assert!(functions.contains_key("a.doit"));
        assert!(functions.contains_key("b.doit"));
        assert!(functions.contains_key("doit"));
        assert_eq!(
            functions.get("doit").expect("local short alias").name,
            "doit"
        );
        assert!(defs_globals.is_empty());
    }

    #[test]
    fn resolve_visible_defs_applies_defs_global_short_alias_rules() {
        let span = SourceSpan::synthetic();
        let make_decl = |namespace: &str, name: &str| ParsedDefsGlobalVarDecl {
            namespace: namespace.to_string(),
            name: name.to_string(),
            qualified_name: format!("{}.{}", namespace, name),
            access: AccessLevel::Public,
            type_expr: ParsedTypeExpr::Primitive("int".to_string()),
            initial_value_expr: None,
            location: span.clone(),
        };

        let unique_defs = BTreeMap::from([(
            "a.xml".to_string(),
            DefsDeclarations {
                type_decls: Vec::new(),
                function_decls: Vec::new(),
                defs_global_var_decls: vec![make_decl("a", "hp")],
                defs_global_const_decls: Vec::new(),
            },
        )]);
        let unique_reachable = BTreeSet::from(["a.xml".to_string()]);
        let (_types, _functions, unique_globals, _defs_consts) =
            resolve_visible_defs(&unique_reachable, &unique_defs, Some("a"))
                .expect("defs should resolve");
        assert!(unique_globals.contains_key("a.hp"));
        assert!(unique_globals.contains_key("hp"));
        assert_eq!(
            unique_globals
                .get("hp")
                .expect("short alias should exist")
                .qualified_name,
            "a.hp"
        );

        let collision_defs = BTreeMap::from([
            (
                "a.xml".to_string(),
                DefsDeclarations {
                    type_decls: Vec::new(),
                    function_decls: Vec::new(),
                    defs_global_var_decls: vec![make_decl("a", "hp")],
                    defs_global_const_decls: Vec::new(),
                },
            ),
            (
                "b.xml".to_string(),
                DefsDeclarations {
                    type_decls: Vec::new(),
                    function_decls: Vec::new(),
                    defs_global_var_decls: vec![make_decl("b", "hp")],
                    defs_global_const_decls: Vec::new(),
                },
            ),
        ]);
        let collision_reachable = BTreeSet::from(["a.xml".to_string(), "b.xml".to_string()]);
        let (_types, _functions, collision_globals, _defs_consts) =
            resolve_visible_defs(&collision_reachable, &collision_defs, Some("a"))
                .expect("defs should resolve");
        assert!(collision_globals.contains_key("a.hp"));
        assert!(collision_globals.contains_key("b.hp"));
        assert!(collision_globals.contains_key("hp"));
        assert_eq!(
            collision_globals
                .get("hp")
                .expect("local short alias should exist")
                .qualified_name,
            "a.hp"
        );
    }

    #[test]
    fn compile_bundle_rejects_defs_global_forward_reference() {
        let files = map(&[
            (
                "shared.xml",
                r#"
<module name="shared" default_access="public">
  <var name="a" type="int">b + 1</var>
  <var name="b" type="int">1</var>
</module>
"#,
            ),
            (
                "main.xml",
                r#"
<!-- import shared from shared.xml -->
<module name="main" default_access="public">
<script name="main"><text>ok</text></script>
</module>
"#,
            ),
        ]);

        let error =
            compile_project_bundle_from_xml_map(&files).expect_err("forward reference should fail");
        assert_eq!(error.code, "DEFS_GLOBAL_INIT_ORDER");
    }

    #[test]
    fn compile_bundle_allows_defs_global_reference_to_initialized_symbol() {
        let files = map(&[
            (
                "shared.xml",
                r#"
<module name="shared" default_access="public">
  <var name="b" type="int">1</var>
  <var name="a" type="int">b + 1</var>
</module>
"#,
            ),
            (
                "main.xml",
                r#"
<!-- import shared from shared.xml -->
<module name="main" default_access="public">
<script name="main"><text>ok</text></script>
</module>
"#,
            ),
        ]);

        let bundle =
            compile_project_bundle_from_xml_map(&files).expect("back reference should pass");
        assert!(bundle.defs_global_declarations.contains_key("shared.a"));
        assert!(bundle.defs_global_declarations.contains_key("shared.b"));
    }

    #[test]
    fn parse_defs_global_var_rejects_value_attr_and_child_elements() {
        let files_with_value = map(&[
            (
                "shared.xml",
                r#"<module name="shared" default_access="public"><var name="hp" type="int" value="1"/></module>"#,
            ),
            (
                "main.xml",
                r#"
<!-- import shared from shared.xml -->
<module name="main" default_access="public">
<script name="main"><text>ok</text></script>
</module>
"#,
            ),
        ]);
        let value_error = compile_project_bundle_from_xml_map(&files_with_value)
            .expect_err("value attr should fail");
        assert_eq!(value_error.code, "XML_ATTR_NOT_ALLOWED");

        let files_with_child = map(&[
            (
                "shared.xml",
                r#"<module name="shared" default_access="public"><var name="hp" type="int"><text>1</text></var></module>"#,
            ),
            (
                "main.xml",
                r#"
<!-- import shared from shared.xml -->
<module name="main" default_access="public">
<script name="main"><text>ok</text></script>
</module>
"#,
            ),
        ]);
        let child_error = compile_project_bundle_from_xml_map(&files_with_child)
            .expect_err("child element should fail");
        assert_eq!(child_error.code, "XML_VAR_CHILD_INVALID");
    }

    #[test]
    fn parse_defs_global_const_rejects_missing_name_or_type() {
        // Missing name attribute
        let files_missing_name = map(&[
            (
                "shared.xml",
                r#"<module name="shared" default_access="public"><const type="int">1</const></module>"#,
            ),
            (
                "main.xml",
                r#"
<!-- import shared from shared.xml -->
<module name="main" default_access="public">
<script name="main"><text>ok</text></script>
</module>
"#,
            ),
        ]);
        let name_error = compile_project_bundle_from_xml_map(&files_missing_name)
            .expect_err("missing name should fail");
        assert_eq!(name_error.code, "XML_MISSING_ATTR");

        // Missing type attribute
        let files_missing_type = map(&[
            (
                "shared.xml",
                r#"<module name="shared" default_access="public"><const name="base">1</const></module>"#,
            ),
            (
                "main.xml",
                r#"
<!-- import shared from shared.xml -->
<module name="main" default_access="public">
<script name="main"><text>ok</text></script>
</module>
"#,
            ),
        ]);
        let type_error = compile_project_bundle_from_xml_map(&files_missing_type)
            .expect_err("missing type should fail");
        assert_eq!(type_error.code, "XML_MISSING_ATTR");
    }

    #[test]
    fn parse_defs_const_rejects_invalid_type() {
        let files = map(&[
            (
                "shared.xml",
                r#"<module name="shared" default_access="public"><const name="base" type="UnknownType">1</const></module>"#,
            ),
            (
                "main.xml",
                r#"
<!-- import shared from shared.xml -->
<module name="main" default_access="public">
<script name="main"><text>ok</text></script>
</module>
"#,
            ),
        ]);
        let error =
            compile_project_bundle_from_xml_map(&files).expect_err("invalid type should fail");
        assert_eq!(error.code, "TYPE_UNKNOWN");
    }

    #[test]
    fn resolve_visible_defs_rejects_const_with_unresolved_type() {
        let span = SourceSpan::synthetic();
        let defs_with_bad_const = BTreeMap::from([(
            "shared.xml".to_string(),
            DefsDeclarations {
                type_decls: Vec::new(),
                function_decls: Vec::new(),
                defs_global_var_decls: Vec::new(),
                defs_global_const_decls: vec![ParsedDefsGlobalConstDecl {
                    namespace: "shared".to_string(),
                    name: "base".to_string(),
                    qualified_name: "shared.base".to_string(),
                    access: AccessLevel::Public,
                    type_expr: ParsedTypeExpr::Custom("UnknownType".to_string()),
                    initial_value_expr: Some("1".to_string()),
                    location: span.clone(),
                }],
            },
        )]);
        let reachable = BTreeSet::from(["shared.xml".to_string()]);
        let error = resolve_visible_defs(&reachable, &defs_with_bad_const, Some("shared"))
            .expect_err("unresolved type should fail");
        assert_eq!(error.code, "TYPE_UNKNOWN");
    }

    #[test]
    fn defs_global_resolution_rejects_duplicates_and_allows_empty_initializer() {
        let duplicate_types_bundle = map(&[
            (
                "a.xml",
                r#"<module name="shared" default_access="public"><type name="T"><field name="v" type="int"/></type></module>"#,
            ),
            (
                "b.xml",
                r#"<module name="shared" default_access="public"><type name="T"><field name="v" type="int"/></type></module>"#,
            ),
        ]);
        let duplicate_types_error = compile_project_bundle_from_xml_map(&duplicate_types_bundle)
            .expect_err("bundle duplicate type should fail");
        assert_eq!(duplicate_types_error.code, "TYPE_DECL_DUPLICATE");

        let duplicate_globals_bundle = map(&[
            (
                "a.xml",
                r#"<module name="shared" default_access="public"><var name="hp" type="int">1</var></module>"#,
            ),
            (
                "b.xml",
                r#"<module name="shared" default_access="public"><var name="hp" type="int">2</var></module>"#,
            ),
        ]);
        let duplicate_globals_error =
            compile_project_bundle_from_xml_map(&duplicate_globals_bundle)
                .expect_err("bundle duplicate defs global should fail");
        assert_eq!(duplicate_globals_error.code, "DEFS_GLOBAL_VAR_DUPLICATE");

        let empty_initializer = map(&[
            (
                "shared.xml",
                r#"<module name="shared" default_access="public"><var name="hp" type="int"/></module>"#,
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
        let bundle = compile_project_bundle_from_xml_map(&empty_initializer).expect("compile");
        let decl = bundle
            .defs_global_declarations
            .get("shared.hp")
            .expect("decl should exist");
        assert!(decl.initial_value_expr.is_none());
    }

    #[test]
    fn resolve_visible_defs_rejects_duplicate_defs_global_in_closure() {
        let span = SourceSpan::synthetic();
        let duplicate = ParsedDefsGlobalVarDecl {
            namespace: "shared".to_string(),
            name: "hp".to_string(),
            qualified_name: "shared.hp".to_string(),
            access: AccessLevel::Public,
            type_expr: ParsedTypeExpr::Primitive("int".to_string()),
            initial_value_expr: Some("1".to_string()),
            location: span.clone(),
        };
        let defs_by_path = BTreeMap::from([
            (
                "a.xml".to_string(),
                DefsDeclarations {
                    type_decls: Vec::new(),
                    function_decls: Vec::new(),
                    defs_global_var_decls: vec![duplicate.clone()],
                    defs_global_const_decls: Vec::new(),
                },
            ),
            (
                "b.xml".to_string(),
                DefsDeclarations {
                    type_decls: Vec::new(),
                    function_decls: Vec::new(),
                    defs_global_var_decls: vec![duplicate],
                    defs_global_const_decls: Vec::new(),
                },
            ),
        ]);
        let reachable = BTreeSet::from(["a.xml".to_string(), "b.xml".to_string()]);
        let error = resolve_visible_defs(&reachable, &defs_by_path, Some("a"))
            .expect_err("duplicate defs global should fail");
        assert_eq!(error.code, "DEFS_GLOBAL_VAR_DUPLICATE");
    }

    #[test]
    fn resolve_named_type_with_aliases_reports_missing_aliased_target() {
        let error = resolve_named_type_with_aliases(
            "Alias",
            &BTreeMap::new(),
            &BTreeMap::from([("Alias".to_string(), "missing.Type".to_string())]),
            &mut BTreeMap::new(),
            &mut HashSet::new(),
        )
        .expect_err("missing aliased target should fail");
        assert_eq!(error.code, "TYPE_UNKNOWN");
    }

    #[test]
    fn defs_and_type_resolution_helpers_cover_duplicate_and_recursive_errors() {
        let bad_defs = BTreeMap::from([(
            "x.xml".to_string(),
            "<script name=\"x\"></script>".to_string(),
        )]);
        let error = compile_project_bundle_from_xml_map(&bad_defs).expect_err("bad defs root");
        assert_eq!(error.code, "XML_ROOT_INVALID");

        let duplicate_types = map(&[
            (
                "a.xml",
                r#"<module name="a" default_access="public"><type name="T"><field name="v" type="int"/></type></module>"#,
            ),
            (
                "b.xml",
                r#"<module name="b" default_access="public"><type name="T"><field name="v" type="int"/></type></module>"#,
            ),
            (
                "main.xml",
                r#"
    <!-- import a from a.xml -->
    <!-- import b from b.xml -->
    <module name="main" default_access="public">
<script name="main"><temp name="v" type="T"/></script>
</module>
    "#,
            ),
        ]);
        let error = compile_project_bundle_from_xml_map(&duplicate_types)
            .expect_err("ambiguous unqualified type should fail");
        assert_eq!(error.code, "TYPE_UNKNOWN");

        let recursive = map(&[
            (
                "x.xml",
                r#"<module name="x" default_access="public"><type name="A"><field name="b" type="B"/></type><type name="B"><field name="a" type="A"/></type></module>"#,
            ),
            (
                "main.xml",
                r#"
    <!-- import x from x.xml -->
    <module name="main" default_access="public">
<script name="main"><temp name="v" type="A"/></script>
</module>
    "#,
            ),
        ]);
        let error = compile_project_bundle_from_xml_map(&recursive)
            .expect_err("recursive type declarations should fail");
        assert_eq!(error.code, "TYPE_UNKNOWN");
    }

    #[test]
    fn defs_function_parsing_and_resolution_is_covered() {
        // Test defs function parsing (covers line 40)
        let files = map(&[
            (
                "shared.xml",
                r#"<module name="shared" default_access="public">
  <function name="add" args="int:a,int:b" return="int:result">
    result = a + b;
  </function>
</module>"#,
            ),
            (
                "main.xml",
                r#"
<!-- import shared from shared.xml -->
<module name="main" default_access="public">
<script name="main">
  <code>let x = shared.add(1, 2);</code>
  <text>${x}</text>
</script>
</module>
"#,
            ),
        ]);
        let bundle = compile_project_bundle_from_xml_map(&files).expect("compile should pass");
        assert!(bundle.scripts.contains_key("main.main"));
    }

    #[test]
    fn parse_defs_files_and_type_resolution_success_paths_are_covered() {
        let files = map(&[(
            "shared.xml",
            r#"<module name="shared" default_access="public">
  <type name="Obj"><field name="value" type="int"/></type>
  <function name="make" args="int:seed" return="Obj:ret">
    ret = #{ value: seed };
  </function>
</module>"#,
        )]);
        let sources = parse_sources(&files).expect("parse sources");
        let defs_by_path = parse_defs_files(&sources).expect("parse defs");
        let reachable = BTreeSet::from(["shared.xml".to_string()]);
        let (types, functions, _, _defs_consts) =
            resolve_visible_defs(&reachable, &defs_by_path, Some("shared")).expect("resolve defs");
        assert!(types.contains_key("shared.Obj"));
        assert!(functions.contains_key("shared.make"));
    }

    #[test]
    fn parse_defs_files_attaches_file_path_for_defs_errors() {
        let files = map(&[(
            "bad.xml",
            r#"<module name="shared" default_access="public">
  <oops/>
</module>"#,
        )]);
        let sources = parse_sources(&files).expect("parse sources");
        let error = parse_defs_files(&sources).expect_err("defs parse should fail");
        assert_eq!(error.code, "XML_MODULE_CHILD_INVALID");
        assert!(error.message.contains("In file \"bad.xml\":"));
    }

    #[test]
    fn with_file_context_preserves_file_name_and_sets_synthetic_span_when_missing() {
        let error = ScriptLangError::new("SOME_CODE", "boom");
        let wrapped = with_file_context(error, "broken.xml");
        assert_eq!(wrapped.code, "SOME_CODE");
        assert!(wrapped.message.contains("In file \"broken.xml\": boom"));
        let span = wrapped.span.expect("span should be present");
        assert_eq!(span.start.line, 1);
        assert_eq!(span.start.column, 1);
        assert_eq!(span.end.line, 1);
        assert_eq!(span.end.column, 1);
    }

    #[test]
    fn parse_defs_files_wraps_attr_reserved_and_function_parse_errors_with_file_context() {
        let missing_name_error = parse_sources(&BTreeMap::from([(
            "missing-name.xml".to_string(),
            "<module></module>".to_string(),
        )]))
        .expect_err("missing name should fail during source parsing");
        assert_eq!(missing_name_error.code, "XML_MODULE_NAME_MISSING");
        assert!(missing_name_error
            .message
            .contains("In file \"missing-name.xml\":"));

        let reserved_name = map(&[(
            "reserved.xml",
            r#"<module name="__sl_bad" default_access="public"></module>"#,
        )]);
        let reserved_name_sources = parse_sources(&reserved_name).expect("parse sources");
        let reserved_name_error =
            parse_defs_files(&reserved_name_sources).expect_err("reserved name should fail");
        assert!(reserved_name_error
            .message
            .contains("In file \"reserved.xml\":"));

        let bad_function = map(&[(
            "bad-function.xml",
            r#"<module name="shared" default_access="public">
  <function name="bad" args="int:a" return="int">
    a = a + 1;
  </function>
</module>"#,
        )]);
        let bad_function_sources = parse_sources(&bad_function).expect("parse sources");
        let bad_function_error =
            parse_defs_files(&bad_function_sources).expect_err("bad function should fail");
        assert!(bad_function_error
            .message
            .contains("In file \"bad-function.xml\":"));
    }

    #[test]
    fn parse_defs_global_var_declaration_covers_success_and_error_paths() {
        let node = xml_element(
            "var",
            &[("name", "hp"), ("type", "int")],
            vec![xml_text("1")],
        );
        let parsed = parse_defs_global_var_declaration(&node, "shared", AccessLevel::Private)
            .expect("parse defs var");
        assert_eq!(parsed.qualified_name, "shared.hp");
        assert_eq!(parsed.initial_value_expr.as_deref(), Some("1"));

        let reserved_name = xml_element(
            "var",
            &[("name", "__sl_hp"), ("type", "int")],
            vec![xml_text("1")],
        );
        let error =
            parse_defs_global_var_declaration(&reserved_name, "shared", AccessLevel::Private)
                .expect_err("reserved name should fail");
        assert_eq!(error.code, "NAME_RESERVED_PREFIX");

        let invalid_type = xml_element(
            "var",
            &[("name", "hp"), ("type", "#{ }")],
            vec![xml_text("1")],
        );
        let error =
            parse_defs_global_var_declaration(&invalid_type, "shared", AccessLevel::Private)
                .expect_err("bad type");
        assert_eq!(error.code, "TYPE_PARSE_ERROR");

        let missing_name = xml_element("var", &[("type", "int")], vec![xml_text("1")]);
        let error =
            parse_defs_global_var_declaration(&missing_name, "shared", AccessLevel::Private)
                .expect_err("name should be required");
        assert_eq!(error.code, "XML_MISSING_ATTR");

        let missing_type = xml_element("var", &[("name", "hp")], vec![xml_text("1")]);
        let error =
            parse_defs_global_var_declaration(&missing_type, "shared", AccessLevel::Private)
                .expect_err("type should be required");
        assert_eq!(error.code, "XML_MISSING_ATTR");

        // Test with explicit access attribute (covers line 160)
        let with_access = xml_element(
            "var",
            &[("name", "gold"), ("type", "int"), ("access", "public")],
            vec![xml_text("100")],
        );
        let parsed =
            parse_defs_global_var_declaration(&with_access, "shared", AccessLevel::Private)
                .expect("var with access should parse");
        assert_eq!(parsed.access, AccessLevel::Public);

        let mut invalid_sources = BTreeMap::new();
        invalid_sources.insert(
            "/".to_string(),
            SourceFile {
                kind: SourceKind::Json,
                imports: Vec::new(),
                xml_root: None,
                json_value: Some(SlValue::Number(1.0)),
            },
        );
        let error = collect_global_json(&invalid_sources).expect_err("invalid json symbol path");
        assert_eq!(error.code, "JSON_SYMBOL_INVALID");

        let reachable = BTreeSet::from(["/".to_string()]);
        let error = collect_visible_json_symbols(&reachable, &invalid_sources)
            .expect_err("invalid visible json symbol path");
        assert_eq!(error.code, "JSON_SYMBOL_INVALID");

        let invalid_basename =
            parse_json_global_symbol("bad-name.json").expect_err("invalid json basename");
        assert_eq!(invalid_basename.code, "JSON_SYMBOL_INVALID");

        let missing_value = collect_global_json(&BTreeMap::from([(
            "game.json".to_string(),
            SourceFile {
                kind: SourceKind::Json,
                imports: Vec::new(),
                xml_root: None,
                json_value: None,
            },
        )]))
        .expect_err("json value should be required");
        assert_eq!(missing_value.code, "JSON_MISSING_VALUE");

        let reserved_json_symbol =
            parse_json_global_symbol("__hidden.json").expect_err("reserved json symbol");
        assert_eq!(reserved_json_symbol.code, "NAME_RESERVED_PREFIX");
    }

    #[test]
    fn resolve_visible_defs_error_propagation_and_alias_paths_are_covered() {
        let span = SourceSpan::synthetic();
        let defs_with_alias = BTreeMap::from([(
            "one.xml".to_string(),
            DefsDeclarations {
                type_decls: vec![ParsedTypeDecl {
                    name: "Obj".to_string(),
                    qualified_name: "one.Obj".to_string(),
                    access: AccessLevel::Public,
                    fields: vec![ParsedTypeFieldDecl {
                        name: "v".to_string(),
                        type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                        location: span.clone(),
                    }],
                    location: span.clone(),
                }],
                function_decls: vec![ParsedFunctionDecl {
                    name: "make".to_string(),
                    qualified_name: "one.make".to_string(),
                    access: AccessLevel::Public,
                    params: vec![ParsedFunctionParamDecl {
                        name: "x".to_string(),
                        type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                        location: span.clone(),
                    }],
                    return_binding: ParsedFunctionParamDecl {
                        name: "ret".to_string(),
                        type_expr: ParsedTypeExpr::Custom("Obj".to_string()),
                        location: span.clone(),
                    },
                    code: "ret = #{v: x};".to_string(),
                    location: span.clone(),
                }],
                defs_global_var_decls: vec![ParsedDefsGlobalVarDecl {
                    namespace: "one".to_string(),
                    name: "hp".to_string(),
                    qualified_name: "one.hp".to_string(),
                    access: AccessLevel::Public,
                    type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                    initial_value_expr: None,
                    location: span.clone(),
                }],
                defs_global_const_decls: Vec::new(),
            },
        )]);
        let reachable = BTreeSet::from(["one.xml".to_string()]);
        let (types, functions, defs_globals, _defs_consts) =
            resolve_visible_defs(&reachable, &defs_with_alias, Some("one"))
                .expect("resolve aliases");
        assert!(types.contains_key("Obj"));
        assert!(functions.contains_key("make"));
        assert!(defs_globals.contains_key("hp"));
        assert_eq!(
            script_type_kind(
                types
                    .get("Obj")
                    .expect("short type alias should be visible in resolved map")
            ),
            "object"
        );

        let defs_for_bundle = BTreeMap::from([(
            "bundle.xml".to_string(),
            DefsDeclarations {
                type_decls: vec![ParsedTypeDecl {
                    name: "T".to_string(),
                    qualified_name: "bundle.T".to_string(),
                    access: AccessLevel::Public,
                    fields: vec![ParsedTypeFieldDecl {
                        name: "v".to_string(),
                        type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                        location: span.clone(),
                    }],
                    location: span.clone(),
                }],
                function_decls: Vec::new(),
                defs_global_var_decls: vec![ParsedDefsGlobalVarDecl {
                    namespace: "bundle".to_string(),
                    name: "item".to_string(),
                    qualified_name: "bundle.item".to_string(),
                    access: AccessLevel::Public,
                    type_expr: ParsedTypeExpr::Custom("T".to_string()),
                    initial_value_expr: None,
                    location: span.clone(),
                }],
                defs_global_const_decls: Vec::new(),
            },
        )]);
        let (bundle_globals, init_order) =
            collect_defs_globals_for_bundle(&defs_for_bundle).expect("bundle alias should resolve");
        assert!(bundle_globals.contains_key("bundle.item"));
        assert_eq!(init_order, vec!["bundle.item".to_string()]);

        let bad_type_decl = BTreeMap::from([(
            "bad_type.xml".to_string(),
            DefsDeclarations {
                type_decls: vec![ParsedTypeDecl {
                    name: "Broken".to_string(),
                    qualified_name: "bad_type.Broken".to_string(),
                    access: AccessLevel::Public,
                    fields: vec![ParsedTypeFieldDecl {
                        name: "v".to_string(),
                        type_expr: ParsedTypeExpr::Custom("Missing".to_string()),
                        location: span.clone(),
                    }],
                    location: span.clone(),
                }],
                function_decls: Vec::new(),
                defs_global_var_decls: Vec::new(),
                defs_global_const_decls: Vec::new(),
            },
        )]);
        let reachable = BTreeSet::from(["bad_type.xml".to_string()]);
        let error = resolve_visible_defs(&reachable, &bad_type_decl, Some("bad_type"))
            .expect_err("type resolution in visible loop should fail");
        assert_eq!(error.code, "TYPE_UNKNOWN");

        let alias_already_exists = BTreeMap::from([(
            "alias.xml".to_string(),
            DefsDeclarations {
                type_decls: Vec::new(),
                function_decls: vec![ParsedFunctionDecl {
                    name: "make".to_string(),
                    qualified_name: "make".to_string(),
                    access: AccessLevel::Public,
                    params: Vec::new(),
                    return_binding: ParsedFunctionParamDecl {
                        name: "ret".to_string(),
                        type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                        location: span.clone(),
                    },
                    code: "ret = 1;".to_string(),
                    location: span.clone(),
                }],
                defs_global_var_decls: Vec::new(),
                defs_global_const_decls: Vec::new(),
            },
        )]);
        let reachable = BTreeSet::from(["alias.xml".to_string()]);
        let (_types, alias_functions, _defs_globals, _defs_consts) =
            resolve_visible_defs(&reachable, &alias_already_exists, None)
                .expect("existing alias key should skip insertion branch");
        assert!(alias_functions.contains_key("make"));

        let malformed_local_names = BTreeMap::from([(
            "odd.xml".to_string(),
            DefsDeclarations {
                type_decls: vec![ParsedTypeDecl {
                    name: "Obj".to_string(),
                    qualified_name: "Obj".to_string(),
                    access: AccessLevel::Public,
                    fields: vec![ParsedTypeFieldDecl {
                        name: "v".to_string(),
                        type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                        location: span.clone(),
                    }],
                    location: span.clone(),
                }],
                function_decls: vec![
                    ParsedFunctionDecl {
                        name: "make".to_string(),
                        qualified_name: "odd.make".to_string(),
                        access: AccessLevel::Public,
                        params: Vec::new(),
                        return_binding: ParsedFunctionParamDecl {
                            name: "ret".to_string(),
                            type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                            location: span.clone(),
                        },
                        code: "ret = 1;".to_string(),
                        location: span.clone(),
                    },
                    ParsedFunctionDecl {
                        name: "make".to_string(),
                        qualified_name: "make".to_string(),
                        access: AccessLevel::Public,
                        params: Vec::new(),
                        return_binding: ParsedFunctionParamDecl {
                            name: "ret".to_string(),
                            type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                            location: span.clone(),
                        },
                        code: "ret = 2;".to_string(),
                        location: span.clone(),
                    },
                ],
                defs_global_var_decls: Vec::new(),
                defs_global_const_decls: Vec::new(),
            },
        )]);
        let reachable = BTreeSet::from(["odd.xml".to_string()]);
        let (malformed_types, malformed_functions, _defs_globals, _defs_consts) =
            resolve_visible_defs(&reachable, &malformed_local_names, Some("odd"))
                .expect("malformed aliases should still resolve without duplicate insert");
        assert!(malformed_types.contains_key("Obj"));
        assert_eq!(
            malformed_functions
                .get("make")
                .expect("existing function alias should be preserved")
                .code,
            "ret = 2;"
        );

        let bad_param = BTreeMap::from([(
            "bad.xml".to_string(),
            DefsDeclarations {
                type_decls: Vec::new(),
                function_decls: vec![ParsedFunctionDecl {
                    name: "f".to_string(),
                    qualified_name: "bad.f".to_string(),
                    access: AccessLevel::Public,
                    params: vec![ParsedFunctionParamDecl {
                        name: "x".to_string(),
                        type_expr: ParsedTypeExpr::Custom("Missing".to_string()),
                        location: span.clone(),
                    }],
                    return_binding: ParsedFunctionParamDecl {
                        name: "ret".to_string(),
                        type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                        location: span.clone(),
                    },
                    code: "ret = 1;".to_string(),
                    location: span.clone(),
                }],
                defs_global_var_decls: Vec::new(),
                defs_global_const_decls: Vec::new(),
            },
        )]);
        let reachable = BTreeSet::from(["bad.xml".to_string()]);
        let error = resolve_visible_defs(&reachable, &bad_param, Some("bad"))
            .expect_err("function param type should resolve");
        assert_eq!(error.code, "TYPE_UNKNOWN");

        let bad_return = BTreeMap::from([(
            "bad.xml".to_string(),
            DefsDeclarations {
                type_decls: Vec::new(),
                function_decls: vec![ParsedFunctionDecl {
                    name: "f".to_string(),
                    qualified_name: "bad.f".to_string(),
                    access: AccessLevel::Public,
                    params: Vec::new(),
                    return_binding: ParsedFunctionParamDecl {
                        name: "ret".to_string(),
                        type_expr: ParsedTypeExpr::Custom("Missing".to_string()),
                        location: span.clone(),
                    },
                    code: "ret = 1;".to_string(),
                    location: span.clone(),
                }],
                defs_global_var_decls: Vec::new(),
                defs_global_const_decls: Vec::new(),
            },
        )]);
        let reachable = BTreeSet::from(["bad.xml".to_string()]);
        let error = resolve_visible_defs(&reachable, &bad_return, Some("bad"))
            .expect_err("function return type should resolve");
        assert_eq!(error.code, "TYPE_UNKNOWN");

        let bad_global_type = BTreeMap::from([(
            "bad.xml".to_string(),
            DefsDeclarations {
                type_decls: Vec::new(),
                function_decls: Vec::new(),
                defs_global_var_decls: vec![ParsedDefsGlobalVarDecl {
                    namespace: "bad".to_string(),
                    name: "hp".to_string(),
                    qualified_name: "bad.hp".to_string(),
                    access: AccessLevel::Public,
                    type_expr: ParsedTypeExpr::Custom("Missing".to_string()),
                    initial_value_expr: None,
                    location: span.clone(),
                }],
                defs_global_const_decls: Vec::new(),
            },
        )]);
        let reachable = BTreeSet::from(["bad.xml".to_string()]);
        let error = resolve_visible_defs(&reachable, &bad_global_type, Some("bad"))
            .expect_err("defs global type should resolve");
        assert_eq!(error.code, "TYPE_UNKNOWN");

        let bundle_error = collect_defs_globals_for_bundle(&bad_global_type)
            .expect_err("bundle defs global type should resolve");
        assert_eq!(bundle_error.code, "TYPE_UNKNOWN");

        assert_eq!(
            script_type_kind(&ScriptType::Primitive {
                name: "int".to_string()
            }),
            "primitive"
        );
        assert_eq!(
            script_type_kind(&ScriptType::Array {
                element_type: Box::new(ScriptType::Primitive {
                    name: "int".to_string()
                })
            }),
            "array"
        );
        assert_eq!(
            script_type_kind(&ScriptType::Map {
                key_type: "string".to_string(),
                value_type: Box::new(ScriptType::Primitive {
                    name: "int".to_string()
                })
            }),
            "map"
        );
    }

    #[test]
    fn parse_module_helpers_cover_module_specific_paths() {
        let sources = parse_sources(&compiler_test_support::map(&[(
            "battle.xml",
            r#"<module name="battle" default_access="public"><script name="main"><text>x</text></script></module>"#,
        )]))
        .expect("sources should parse");

        let module_scripts = parse_module_scripts(&sources).expect("module scripts should parse");
        assert_eq!(module_scripts["battle.xml"].len(), 1);
        assert!(parse_defs_files(&sources).is_ok());

        let bad_root = SourceFile {
            kind: SourceKind::ModuleXml,
            imports: Vec::new(),
            xml_root: Some(compiler_test_support::xml_element(
                "defs",
                &[("name", "x")],
                Vec::new(),
            )),
            json_value: None,
        };
        let bad_root_error =
            parse_module_source(&bad_root, "bad.xml").expect_err("module root should fail");
        assert_eq!(bad_root_error.code, "XML_ROOT_INVALID");

        let reserved_script = SourceFile {
            kind: SourceKind::ModuleXml,
            imports: Vec::new(),
            xml_root: Some(compiler_test_support::xml_element(
                "module",
                &[("name", "battle")],
                vec![XmlNode::Element(compiler_test_support::xml_element(
                    "script",
                    &[("name", "__sl_main")],
                    Vec::new(),
                ))],
            )),
            json_value: None,
        };
        let reserved_script_error = parse_module_source(&reserved_script, "battle.xml")
            .expect_err("reserved module script should fail");
        assert_eq!(reserved_script_error.code, "NAME_RESERVED_PREFIX");

        let missing_script_name = SourceFile {
            kind: SourceKind::ModuleXml,
            imports: Vec::new(),
            xml_root: Some(compiler_test_support::xml_element(
                "module",
                &[("name", "battle")],
                vec![XmlNode::Element(compiler_test_support::xml_element(
                    "script",
                    &[],
                    Vec::new(),
                ))],
            )),
            json_value: None,
        };
        let missing_script_name_error = parse_module_source(&missing_script_name, "battle.xml")
            .expect_err("module script name should be required");
        assert_eq!(missing_script_name_error.code, "XML_MISSING_ATTR");

        let unsupported_kind = SourceFile {
            kind: SourceKind::Json,
            imports: Vec::new(),
            xml_root: None,
            json_value: Some(SlValue::Bool(false)),
        };
        let unsupported_kind_error = parse_module_source(&unsupported_kind, "main.json")
            .expect_err("json source kind should fail");
        assert_eq!(unsupported_kind_error.code, "SOURCE_KIND_UNSUPPORTED");

        let bad_module_sources = BTreeMap::from([(
            "bad.xml".to_string(),
            SourceFile {
                kind: SourceKind::ModuleXml,
                imports: Vec::new(),
                xml_root: Some(compiler_test_support::xml_element(
                    "module",
                    &[("name", "battle")],
                    vec![XmlNode::Element(compiler_test_support::xml_element(
                        "script",
                        &[],
                        Vec::new(),
                    ))],
                )),
                json_value: None,
            },
        )]);
        let parse_module_scripts_error =
            parse_module_scripts(&bad_module_sources).expect_err("bad module scripts should fail");
        assert_eq!(parse_module_scripts_error.code, "XML_MISSING_ATTR");
    }

    #[test]
    fn defs_resolution_helpers_cover_json_and_missing_path_branches() {
        let json_source = SourceFile {
            kind: SourceKind::Json,
            imports: Vec::new(),
            xml_root: None,
            json_value: Some(SlValue::Bool(true)),
        };
        let module_source = SourceFile {
            kind: SourceKind::ModuleXml,
            imports: Vec::new(),
            xml_root: Some(compiler_test_support::xml_element(
                "module",
                &[("name", "main")],
                Vec::new(),
            )),
            json_value: None,
        };
        let sources = BTreeMap::from([
            ("main.xml".to_string(), module_source),
            ("shared.json".to_string(), json_source.clone()),
        ]);
        assert!(parse_defs_files(&sources).is_ok());
        assert!(parse_module_scripts(&sources).is_ok());

        let duplicate_json = collect_global_json(&BTreeMap::from([
            ("a/game.json".to_string(), json_source.clone()),
            ("b/game.json".to_string(), json_source.clone()),
        ]))
        .expect_err("duplicate json symbol should fail");
        assert_eq!(duplicate_json.code, "JSON_SYMBOL_DUPLICATE");

        let collected = collect_global_json(&BTreeMap::from([
            (
                "main.xml".to_string(),
                SourceFile {
                    kind: SourceKind::ModuleXml,
                    imports: Vec::new(),
                    xml_root: Some(compiler_test_support::xml_element(
                        "module",
                        &[("name", "main")],
                        Vec::new(),
                    )),
                    json_value: None,
                },
            ),
            ("game.json".to_string(), json_source.clone()),
        ]))
        .expect("non-json sources should be skipped");
        assert_eq!(collected.get("game"), Some(&SlValue::Bool(true)));

        let duplicate_visible = collect_visible_json_symbols(
            &BTreeSet::from(["a/game.json".to_string(), "b/game.json".to_string()]),
            &BTreeMap::from([
                ("a/game.json".to_string(), json_source.clone()),
                ("b/game.json".to_string(), json_source.clone()),
            ]),
        )
        .expect_err("duplicate visible json symbol should fail");
        assert_eq!(duplicate_visible.code, "JSON_SYMBOL_DUPLICATE");

        let visible = collect_visible_json_symbols(
            &BTreeSet::from(["main.xml".to_string(), "game.json".to_string()]),
            &BTreeMap::from([
                (
                    "main.xml".to_string(),
                    SourceFile {
                        kind: SourceKind::ModuleXml,
                        imports: Vec::new(),
                        xml_root: Some(compiler_test_support::xml_element(
                            "module",
                            &[("name", "main")],
                            Vec::new(),
                        )),
                        json_value: None,
                    },
                ),
                ("game.json".to_string(), json_source.clone()),
            ]),
        )
        .expect("non-json visible sources should be skipped");
        assert_eq!(visible, vec!["game".to_string()]);

        let span = SourceSpan::synthetic();
        let defs_by_path = BTreeMap::from([(
            "main.xml".to_string(),
            DefsDeclarations {
                type_decls: vec![ParsedTypeDecl {
                    name: "Player".to_string(),
                    qualified_name: "main.Player".to_string(),
                    access: AccessLevel::Public,
                    fields: vec![ParsedTypeFieldDecl {
                        name: "hp".to_string(),
                        type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                        location: span.clone(),
                    }],
                    location: span.clone(),
                }],
                function_decls: vec![ParsedFunctionDecl {
                    name: "boost".to_string(),
                    qualified_name: "main.boost".to_string(),
                    access: AccessLevel::Public,
                    params: Vec::new(),
                    return_binding: ParsedFunctionParamDecl {
                        name: "out".to_string(),
                        type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                        location: span.clone(),
                    },
                    code: "out = 1;".to_string(),
                    location: span.clone(),
                }],
                defs_global_var_decls: vec![ParsedDefsGlobalVarDecl {
                    namespace: "main".to_string(),
                    name: "hp".to_string(),
                    qualified_name: "main.hp".to_string(),
                    access: AccessLevel::Public,
                    type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                    initial_value_expr: None,
                    location: span,
                }],
                defs_global_const_decls: Vec::new(),
            },
        )]);
        let reachable = BTreeSet::from(["main.xml".to_string(), "missing.xml".to_string()]);
        let (types, functions, defs_globals, _defs_consts) =
            resolve_visible_defs(&reachable, &defs_by_path, Some("main"))
                .expect("missing paths in reachable closure should be skipped");
        assert!(types.contains_key("Player"));
        assert!(functions.contains_key("boost"));
        assert!(defs_globals.contains_key("hp"));
    }

    #[test]
    fn parse_defs_global_var_rejects_invalid_access() {
        let files = map(&[
            (
                "shared.xml",
                r#"<module name="shared" default_access="public"><var name="hp" type="int" access="invalid">1</var></module>"#,
            ),
            (
                "main.xml",
                r#"
<!-- import shared from shared.xml -->
<module name="main" default_access="public">
<script name="main"><text>ok</text></script>
</module>
"#,
            ),
        ]);
        let error =
            compile_project_bundle_from_xml_map(&files).expect_err("invalid access should fail");
        assert_eq!(error.code, "XML_ACCESS_INVALID");
    }

    #[test]
    fn compile_bundle_supports_module_const_declarations() {
        let files = map(&[(
            "main.xml",
            r#"<module name="main" default_access="public">
  <const name="base" type="int">7</const>
  <script name="main"><text>${base}</text></script>
</module>"#,
        )]);
        let bundle = compile_project_bundle_from_xml_map(&files).expect("compile should pass");
        assert!(bundle
            .defs_global_const_declarations
            .contains_key("main.base"));
        assert_eq!(
            bundle.defs_global_const_init_order,
            vec!["main.base".to_string()]
        );
    }

    #[test]
    fn compile_bundle_rejects_const_initializer_referencing_var() {
        let files = map(&[(
            "main.xml",
            r#"<module name="main" default_access="public">
  <var name="hp" type="int">10</var>
  <const name="bad" type="int">hp + 1</const>
  <script name="main"><text>${bad}</text></script>
</module>"#,
        )]);
        let error = compile_project_bundle_from_xml_map(&files)
            .expect_err("const initializer referencing var should fail");
        assert_eq!(error.code, "DEFS_CONST_INIT_REF_NON_CONST");

        let files_qualified = map(&[(
            "main.xml",
            r#"<module name="main" default_access="public">
  <var name="hp" type="int">10</var>
  <const name="bad" type="int">main.hp + 1</const>
  <script name="main"><text>${bad}</text></script>
</module>"#,
        )]);
        let qualified_error = compile_project_bundle_from_xml_map(&files_qualified)
            .expect_err("const initializer referencing qualified var should fail");
        assert_eq!(qualified_error.code, "DEFS_CONST_INIT_REF_NON_CONST");
    }

    #[test]
    fn resolve_visible_defs_skips_private_types_from_non_local_module() {
        let span = SourceSpan::synthetic();
        let defs = DefsDeclarations {
            type_decls: vec![ParsedTypeDecl {
                name: "Secret".to_string(),
                qualified_name: "other.Secret".to_string(),
                access: AccessLevel::Private,
                fields: vec![ParsedTypeFieldDecl {
                    name: "v".to_string(),
                    type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                    location: span.clone(),
                }],
                location: span.clone(),
            }],
            function_decls: Vec::new(),
            defs_global_var_decls: Vec::new(),
            defs_global_const_decls: Vec::new(),
        };

        let reachable = BTreeSet::from(["other.xml".to_string()]);
        let defs_by_path = BTreeMap::from([("other.xml".to_string(), defs)]);

        // Query from module "main" should NOT see "other.Secret" because it's private
        let (types, functions, defs_globals, _defs_consts) =
            resolve_visible_defs(&reachable, &defs_by_path, Some("main")).expect("should resolve");
        assert!(
            !types.contains_key("Secret"),
            "private type from non-local should be hidden"
        );
        assert!(functions.is_empty());
        assert!(defs_globals.is_empty());
    }

    #[test]
    fn resolve_visible_defs_skips_private_functions_from_non_local_module() {
        let span = SourceSpan::synthetic();
        let defs = DefsDeclarations {
            type_decls: Vec::new(),
            function_decls: vec![ParsedFunctionDecl {
                name: "hidden".to_string(),
                qualified_name: "other.hidden".to_string(),
                access: AccessLevel::Private,
                params: Vec::new(),
                return_binding: ParsedFunctionParamDecl {
                    name: "out".to_string(),
                    type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                    location: span.clone(),
                },
                code: "out = 1;".to_string(),
                location: span.clone(),
            }],
            defs_global_var_decls: Vec::new(),
            defs_global_const_decls: Vec::new(),
        };

        let reachable = BTreeSet::from(["other.xml".to_string()]);
        let defs_by_path = BTreeMap::from([("other.xml".to_string(), defs)]);

        // Query from module "main" should NOT see "other.hidden" because it's private
        let (types, functions, defs_globals, _defs_consts) =
            resolve_visible_defs(&reachable, &defs_by_path, Some("main")).expect("should resolve");
        assert!(types.is_empty());
        assert!(
            !functions.contains_key("hidden"),
            "private function from non-local should be hidden"
        );
        assert!(defs_globals.is_empty());
    }

    #[test]
    fn parse_defs_global_const_declaration_validates_shape() {
        let node = xml_element(
            "const",
            &[("name", "base"), ("type", "int")],
            vec![xml_text("7")],
        );
        let parsed = parse_defs_global_const_declaration(&node, "main", AccessLevel::Private)
            .expect("const should parse");
        assert_eq!(parsed.qualified_name, "main.base");

        let with_value = xml_element(
            "const",
            &[("name", "base"), ("type", "int"), ("value", "1")],
            vec![],
        );
        let value_error =
            parse_defs_global_const_declaration(&with_value, "main", AccessLevel::Private)
                .expect_err("value attr should fail");
        assert_eq!(value_error.code, "XML_ATTR_NOT_ALLOWED");

        let with_child = xml_element(
            "const",
            &[("name", "base"), ("type", "int")],
            vec![XmlNode::Element(xml_element(
                "text",
                &[],
                vec![xml_text("x")],
            ))],
        );
        let child_error =
            parse_defs_global_const_declaration(&with_child, "main", AccessLevel::Private)
                .expect_err("child should fail");
        assert_eq!(child_error.code, "XML_VAR_CHILD_INVALID");
    }

    #[test]
    fn resolve_visible_defs_includes_public_consts_and_local_private_consts() {
        let span = SourceSpan::synthetic();
        let defs_by_path = BTreeMap::from([
            (
                "main.xml".to_string(),
                DefsDeclarations {
                    type_decls: Vec::new(),
                    function_decls: Vec::new(),
                    defs_global_var_decls: Vec::new(),
                    defs_global_const_decls: vec![
                        ParsedDefsGlobalConstDecl {
                            namespace: "main".to_string(),
                            name: "localConst".to_string(),
                            qualified_name: "main.localConst".to_string(),
                            access: AccessLevel::Private,
                            type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                            initial_value_expr: Some("1".to_string()),
                            location: span.clone(),
                        },
                        ParsedDefsGlobalConstDecl {
                            namespace: "main".to_string(),
                            name: "sharedConst".to_string(),
                            qualified_name: "main.sharedConst".to_string(),
                            access: AccessLevel::Public,
                            type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                            initial_value_expr: Some("2".to_string()),
                            location: span,
                        },
                    ],
                },
            ),
            (
                "other.xml".to_string(),
                DefsDeclarations {
                    type_decls: Vec::new(),
                    function_decls: Vec::new(),
                    defs_global_var_decls: Vec::new(),
                    defs_global_const_decls: vec![ParsedDefsGlobalConstDecl {
                        namespace: "other".to_string(),
                        name: "hidden".to_string(),
                        qualified_name: "other.hidden".to_string(),
                        access: AccessLevel::Private,
                        type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                        initial_value_expr: Some("3".to_string()),
                        location: SourceSpan::synthetic(),
                    }],
                },
            ),
        ]);
        let reachable = BTreeSet::from(["main.xml".to_string(), "other.xml".to_string()]);
        let (_types, _functions, _defs_globals, defs_consts) =
            resolve_visible_defs(&reachable, &defs_by_path, Some("main")).expect("resolve");
        assert!(defs_consts.contains_key("main.localConst"));
        assert!(defs_consts.contains_key("sharedConst"));
        assert!(!defs_consts.contains_key("other.hidden"));
    }

    #[test]
    fn collect_defs_consts_for_bundle_rejects_duplicate_and_forward_reference() {
        let span = SourceSpan::synthetic();
        let defs_globals = BTreeMap::from([(
            "main.hp".to_string(),
            DefsGlobalVarDecl {
                namespace: "main".to_string(),
                name: "hp".to_string(),
                qualified_name: "main.hp".to_string(),
                access: AccessLevel::Public,
                r#type: ScriptType::Primitive {
                    name: "int".to_string(),
                },
                initial_value_expr: Some("1".to_string()),
                location: span.clone(),
            },
        )]);
        let duplicate = BTreeMap::from([
            (
                "a.xml".to_string(),
                DefsDeclarations {
                    type_decls: Vec::new(),
                    function_decls: Vec::new(),
                    defs_global_var_decls: Vec::new(),
                    defs_global_const_decls: vec![ParsedDefsGlobalConstDecl {
                        namespace: "main".to_string(),
                        name: "base".to_string(),
                        qualified_name: "main.base".to_string(),
                        access: AccessLevel::Public,
                        type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                        initial_value_expr: Some("1".to_string()),
                        location: span.clone(),
                    }],
                },
            ),
            (
                "b.xml".to_string(),
                DefsDeclarations {
                    type_decls: Vec::new(),
                    function_decls: Vec::new(),
                    defs_global_var_decls: Vec::new(),
                    defs_global_const_decls: vec![ParsedDefsGlobalConstDecl {
                        namespace: "main".to_string(),
                        name: "base".to_string(),
                        qualified_name: "main.base".to_string(),
                        access: AccessLevel::Public,
                        type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                        initial_value_expr: Some("2".to_string()),
                        location: span.clone(),
                    }],
                },
            ),
        ]);
        let duplicate_error = collect_defs_consts_for_bundle(&duplicate, &defs_globals)
            .expect_err("duplicate const should fail");
        assert_eq!(duplicate_error.code, "DEFS_GLOBAL_CONST_DUPLICATE");

        let bad_order = BTreeMap::from([(
            "main.xml".to_string(),
            DefsDeclarations {
                type_decls: Vec::new(),
                function_decls: Vec::new(),
                defs_global_var_decls: Vec::new(),
                defs_global_const_decls: vec![
                    ParsedDefsGlobalConstDecl {
                        namespace: "main".to_string(),
                        name: "a".to_string(),
                        qualified_name: "main.a".to_string(),
                        access: AccessLevel::Public,
                        type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                        initial_value_expr: Some("b + 1".to_string()),
                        location: SourceSpan::synthetic(),
                    },
                    ParsedDefsGlobalConstDecl {
                        namespace: "main".to_string(),
                        name: "b".to_string(),
                        qualified_name: "main.b".to_string(),
                        access: AccessLevel::Public,
                        type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                        initial_value_expr: Some("1".to_string()),
                        location: SourceSpan::synthetic(),
                    },
                ],
            },
        )]);
        let order_error = collect_defs_consts_for_bundle(&bad_order, &defs_globals)
            .expect_err("forward const reference should fail");
        assert_eq!(order_error.code, "DEFS_CONST_INIT_ORDER");
    }

    #[test]
    fn resolve_visible_defs_rejects_duplicate_defs_const_in_closure() {
        let span = SourceSpan::synthetic();
        let duplicate = ParsedDefsGlobalConstDecl {
            namespace: "shared".to_string(),
            name: "base".to_string(),
            qualified_name: "shared.base".to_string(),
            access: AccessLevel::Public,
            type_expr: ParsedTypeExpr::Primitive("int".to_string()),
            initial_value_expr: Some("1".to_string()),
            location: span.clone(),
        };
        let defs_by_path = BTreeMap::from([
            (
                "a.xml".to_string(),
                DefsDeclarations {
                    type_decls: Vec::new(),
                    function_decls: Vec::new(),
                    defs_global_var_decls: Vec::new(),
                    defs_global_const_decls: vec![duplicate.clone()],
                },
            ),
            (
                "b.xml".to_string(),
                DefsDeclarations {
                    type_decls: Vec::new(),
                    function_decls: Vec::new(),
                    defs_global_var_decls: Vec::new(),
                    defs_global_const_decls: vec![duplicate],
                },
            ),
        ]);
        let reachable = BTreeSet::from(["a.xml".to_string(), "b.xml".to_string()]);
        let error = resolve_visible_defs(&reachable, &defs_by_path, Some("a"))
            .expect_err("duplicate defs const should fail");
        assert_eq!(error.code, "DEFS_GLOBAL_CONST_DUPLICATE");
    }

    #[test]
    fn collect_defs_consts_rejects_duplicate_type_in_bundle() {
        let span = SourceSpan::synthetic();
        let duplicate_type = ParsedTypeDecl {
            name: "T".to_string(),
            qualified_name: "main.T".to_string(),
            access: AccessLevel::Public,
            fields: vec![],
            location: span.clone(),
        };
        let defs_by_path = BTreeMap::from([
            (
                "a.xml".to_string(),
                DefsDeclarations {
                    type_decls: vec![duplicate_type.clone()],
                    function_decls: Vec::new(),
                    defs_global_var_decls: Vec::new(),
                    defs_global_const_decls: Vec::new(),
                },
            ),
            (
                "b.xml".to_string(),
                DefsDeclarations {
                    type_decls: vec![duplicate_type],
                    function_decls: Vec::new(),
                    defs_global_var_decls: Vec::new(),
                    defs_global_const_decls: Vec::new(),
                },
            ),
        ]);
        let defs_globals = BTreeMap::new();
        let error = collect_defs_consts_for_bundle(&defs_by_path, &defs_globals)
            .expect_err("duplicate type should fail");
        assert_eq!(error.code, "TYPE_DECL_DUPLICATE");
    }

    #[test]
    fn validate_defs_const_init_rules_handles_ambiguous_short_name() {
        // Test when multiple defs_const have the same short name (candidates.len() > 1)
        let span = SourceSpan::synthetic();
        let defs_consts = BTreeMap::from([
            (
                "main.base".to_string(),
                DefsGlobalConstDecl {
                    namespace: "main".to_string(),
                    name: "base".to_string(),
                    qualified_name: "main.base".to_string(),
                    access: AccessLevel::Public,
                    r#type: ScriptType::Primitive {
                        name: "int".to_string(),
                    },
                    initial_value_expr: Some("1".to_string()),
                    location: span.clone(),
                },
            ),
            (
                "other.base".to_string(),
                DefsGlobalConstDecl {
                    namespace: "other".to_string(),
                    name: "base".to_string(),
                    qualified_name: "other.base".to_string(),
                    access: AccessLevel::Public,
                    r#type: ScriptType::Primitive {
                        name: "int".to_string(),
                    },
                    initial_value_expr: Some("2".to_string()),
                    location: span.clone(),
                },
            ),
        ]);
        let defs_globals = BTreeMap::new();
        let init_order = vec!["main.base".to_string(), "other.base".to_string()];
        // This should NOT error because we just validate the init order
        let result = validate_defs_const_init_rules(&defs_consts, &init_order, &defs_globals);
        assert!(
            result.is_ok(),
            "ambiguous short name should be filtered out in mapping"
        );
    }

    #[test]
    fn validate_defs_const_init_rules_rejects_forward_reference() {
        // Test when a defs_const references another const that hasn't been initialized yet
        let span = SourceSpan::synthetic();
        let defs_consts = BTreeMap::from([
            (
                "main.first".to_string(),
                DefsGlobalConstDecl {
                    namespace: "main".to_string(),
                    name: "first".to_string(),
                    qualified_name: "main.first".to_string(),
                    access: AccessLevel::Public,
                    r#type: ScriptType::Primitive {
                        name: "int".to_string(),
                    },
                    initial_value_expr: Some("second".to_string()), // references second before init
                    location: span.clone(),
                },
            ),
            (
                "main.second".to_string(),
                DefsGlobalConstDecl {
                    namespace: "main".to_string(),
                    name: "second".to_string(),
                    qualified_name: "main.second".to_string(),
                    access: AccessLevel::Public,
                    r#type: ScriptType::Primitive {
                        name: "int".to_string(),
                    },
                    initial_value_expr: Some("1".to_string()),
                    location: span.clone(),
                },
            ),
        ]);
        let defs_globals = BTreeMap::new();
        // Initialize first before second - this should fail
        let init_order = vec!["main.first".to_string(), "main.second".to_string()];
        let error = validate_defs_const_init_rules(&defs_consts, &init_order, &defs_globals)
            .expect_err("forward reference should fail");
        assert_eq!(error.code, "DEFS_CONST_INIT_ORDER");
    }

    #[test]
    fn resolve_visible_defs_reports_type_resolution_error() {
        // Test that type resolution errors propagate through line 784
        // This creates a type with a field referencing a non-existent type
        let span = SourceSpan::synthetic();
        let defs = DefsDeclarations {
            type_decls: vec![ParsedTypeDecl {
                name: "MyType".to_string(),
                qualified_name: "shared.MyType".to_string(),
                access: AccessLevel::Public,
                fields: vec![ParsedTypeFieldDecl {
                    name: "field".to_string(),
                    type_expr: ParsedTypeExpr::Custom("NonExistentType".to_string()),
                    location: span.clone(),
                }],
                location: span.clone(),
            }],
            function_decls: Vec::new(),
            defs_global_var_decls: Vec::new(),
            defs_global_const_decls: Vec::new(),
        };

        let reachable = BTreeSet::from(["shared.xml".to_string()]);
        let defs_by_path = BTreeMap::from([("shared.xml".to_string(), defs)]);

        let error = resolve_visible_defs(&reachable, &defs_by_path, None)
            .expect_err("type resolution should fail for non-existent type");
        assert_eq!(error.code, "TYPE_UNKNOWN");
    }

    #[test]
    fn resolve_visible_defs_reports_duplicate_field_error() {
        // Test that duplicate field errors propagate through line 784
        let span = SourceSpan::synthetic();
        let defs = DefsDeclarations {
            type_decls: vec![ParsedTypeDecl {
                name: "MyType".to_string(),
                qualified_name: "shared.MyType".to_string(),
                access: AccessLevel::Public,
                fields: vec![
                    ParsedTypeFieldDecl {
                        name: "field".to_string(),
                        type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                        location: span.clone(),
                    },
                    ParsedTypeFieldDecl {
                        name: "field".to_string(), // duplicate field name
                        type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                        location: span.clone(),
                    },
                ],
                location: span.clone(),
            }],
            function_decls: Vec::new(),
            defs_global_var_decls: Vec::new(),
            defs_global_const_decls: Vec::new(),
        };

        let reachable = BTreeSet::from(["shared.xml".to_string()]);
        let defs_by_path = BTreeMap::from([("shared.xml".to_string(), defs)]);

        let error = resolve_visible_defs(&reachable, &defs_by_path, None)
            .expect_err("duplicate field should fail");
        assert_eq!(error.code, "TYPE_FIELD_DUPLICATE");
    }
}
