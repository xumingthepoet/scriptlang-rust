fn parse_defs_files(
    sources: &BTreeMap<String, SourceFile>,
) -> Result<BTreeMap<String, DefsDeclarations>, ScriptLangError> {
    let mut defs_by_path = BTreeMap::new();

    for (file_path, source) in sources {
        if !matches!(source.kind, SourceKind::DefsXml) {
            continue;
        }

        let root = source
            .xml_root
            .as_ref()
            .expect("defs sources should always carry parsed xml root");

        if root.name != "defs" {
            return Err(ScriptLangError::with_span(
                "XML_ROOT_INVALID",
                format!("Expected <defs> root in file \"{}\".", file_path),
                root.location.clone(),
            ));
        }

        let collection_name = get_required_non_empty_attr(root, "name")?;
        assert_name_not_reserved(&collection_name, "defs", root.location.clone())?;

        let mut type_decls = Vec::new();
        let mut function_decls = Vec::new();

        for child in element_children(root) {
            match child.name.as_str() {
                "type" => type_decls.push(parse_type_declaration_node_with_namespace(
                    child,
                    &collection_name,
                )?),
                "function" => function_decls.push(parse_function_declaration_node_with_namespace(
                    child,
                    &collection_name,
                )?),
                _ => {
                    return Err(ScriptLangError::with_span(
                        "XML_DEFS_CHILD_INVALID",
                        format!("Unsupported child <{}> under <defs>.", child.name),
                        child.location.clone(),
                    ))
                }
            }
        }

        defs_by_path.insert(
            file_path.clone(),
            DefsDeclarations {
                type_decls,
                function_decls,
            },
        );
    }

    Ok(defs_by_path)
}

fn collect_global_json(
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

fn collect_visible_json_symbols(
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

fn parse_json_global_symbol(file_path: &str) -> Result<String, ScriptLangError> {
    let path = Path::new(file_path);
    let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
        return Err(ScriptLangError::new(
            "JSON_SYMBOL_INVALID",
            format!("Invalid JSON file name: {}", file_path),
        ));
    };

    let symbol_regex =
        Regex::new(r"^[$A-Za-z_][$0-9A-Za-z_]*$").expect("json symbol regex must compile");
    if !symbol_regex.is_match(stem) {
        return Err(ScriptLangError::new(
            "JSON_SYMBOL_INVALID",
            format!("JSON basename \"{}\" is not a valid identifier.", stem),
        ));
    }

    assert_name_not_reserved(stem, "json symbol", SourceSpan::synthetic())?;
    Ok(stem.to_string())
}

fn resolve_visible_defs(
    reachable: &BTreeSet<String>,
    defs_by_path: &BTreeMap<String, DefsDeclarations>,
) -> Result<(VisibleTypeMap, VisibleFunctionMap), ScriptLangError> {
    let mut type_decls_map: BTreeMap<String, ParsedTypeDecl> = BTreeMap::new();
    let mut type_short_candidates: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for path in reachable {
        let Some(defs) = defs_by_path.get(path) else {
            continue;
        };
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
        if let Some(ty) = resolved_types.get(qualified_name).cloned() {
            visible_types.insert(alias.clone(), ty);
        }
    }

    let mut functions: BTreeMap<String, FunctionDecl> = BTreeMap::new();
    let mut function_short_candidates: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for path in reachable {
        let Some(defs) = defs_by_path.get(path) else {
            continue;
        };

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

            let mut params = Vec::new();
            for param in &decl.params {
                params.push(FunctionParam {
                    name: param.name.clone(),
                    r#type: resolve_type_expr(&param.type_expr, &visible_types, &param.location)?,
                    location: param.location.clone(),
                });
            }

            let rb = &decl.return_binding;
            let return_type = resolve_type_expr(&rb.type_expr, &visible_types, &rb.location)?;

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
            function_short_candidates
                .entry(decl.name.clone())
                .or_default()
                .push(decl.qualified_name.clone());
        }
    }

    for (alias, qualified_names) in function_short_candidates {
        if qualified_names.len() != 1 {
            continue;
        }
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

    Ok((visible_types, functions))
}

