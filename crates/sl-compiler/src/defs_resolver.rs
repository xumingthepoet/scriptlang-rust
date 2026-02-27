use crate::*;

pub(crate) fn parse_defs_files(
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
        let mut defs_global_var_decls = Vec::new();

        for child in element_children(root) {
            match child.name.as_str() {
                "type" => type_decls.push(parse_type_declaration_node_with_namespace(
                    child,
                    &collection_name,
                )?),
                "function" => {
                    let function_decl =
                        parse_function_declaration_node_with_namespace(child, &collection_name)?;
                    function_decls.push(function_decl);
                }
                "var" => defs_global_var_decls
                    .push(parse_defs_global_var_declaration(child, &collection_name)?),
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
                defs_global_var_decls,
            },
        );
    }

    Ok(defs_by_path)
}

pub(crate) fn parse_defs_global_var_declaration(
    node: &XmlElementNode,
    namespace: &str,
) -> Result<ParsedDefsGlobalVarDecl, ScriptLangError> {
    let name = get_required_non_empty_attr(node, "name")?;
    assert_name_not_reserved(&name, "defs var", node.location.clone())?;

    let type_raw = get_required_non_empty_attr(node, "type")?;
    let type_expr = parse_type_expr(&type_raw, &node.location)?;

    if has_attr(node, "value") {
        return Err(ScriptLangError::with_span(
            "XML_ATTR_NOT_ALLOWED",
            "Attribute \"value\" is not allowed on <var>. Use inline content instead.",
            node.location.clone(),
        ));
    }

    if let Some(child) = element_children(node).next() {
        return Err(ScriptLangError::with_span(
            "XML_VAR_CHILD_INVALID",
            format!(
                "<var> cannot contain child element <{}>. Use inline expression text only.",
                child.name
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
        type_expr,
        initial_value_expr,
        location: node.location.clone(),
    })
}

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

pub(crate) fn json_symbol_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"^[$A-Za-z_][$0-9A-Za-z_]*$").expect("json symbol regex must compile")
    })
}

pub(crate) fn resolve_visible_defs(
    reachable: &BTreeSet<String>,
    defs_by_path: &BTreeMap<String, DefsDeclarations>,
) -> Result<
    (
        VisibleTypeMap,
        VisibleFunctionMap,
        BTreeMap<String, DefsGlobalVarDecl>,
    ),
    ScriptLangError,
> {
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

    let mut resolve_visible_type = |type_name: &String| {
        resolve_named_type_with_aliases(
            type_name,
            &type_decls_map,
            &type_aliases,
            &mut resolved_types,
            &mut visiting,
        )
    };
    for type_name in type_decls_map.keys() {
        resolve_visible_type(type_name)?;
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

    let mut defs_globals_qualified = BTreeMap::new();
    let mut defs_global_short_candidates: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for path in reachable {
        let Some(defs) = defs_by_path.get(path) else {
            continue;
        };

        for decl in &defs.defs_global_var_decls {
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
                    r#type: resolve_type_expr(&decl.type_expr, &visible_types, &decl.location)?,
                    initial_value_expr: decl.initial_value_expr.clone(),
                    location: decl.location.clone(),
                },
            );
            defs_global_short_candidates
                .entry(decl.name.clone())
                .or_default()
                .push(decl.qualified_name.clone());
        }
    }

    let mut defs_globals = defs_globals_qualified.clone();
    for (alias, qualified_names) in defs_global_short_candidates {
        if qualified_names.len() != 1 {
            continue;
        }
        let qualified_name = &qualified_names[0];
        let decl = defs_globals_qualified
            .get(qualified_name)
            .cloned()
            .expect("defs global alias target should exist");
        defs_globals.entry(alias).or_insert(decl);
    }

    Ok((visible_types, functions, defs_globals))
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
        if let Some(ty) = resolved_types.get(qualified_name).cloned() {
            visible_types.insert(alias.clone(), ty);
        }
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

#[cfg(test)]
mod defs_resolver_tests {
    use super::*;
    use crate::compiler_test_support::*;

    #[test]
    fn resolve_visible_defs_builds_function_signatures() {
        let span = SourceSpan::synthetic();
        let defs = DefsDeclarations {
            type_decls: vec![ParsedTypeDecl {
                name: "Obj".to_string(),
                qualified_name: "shared.Obj".to_string(),
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
        };

        let reachable = BTreeSet::from(["shared.defs.xml".to_string()]);
        let defs_by_path = BTreeMap::from([("shared.defs.xml".to_string(), defs)]);

        let (types, functions, defs_globals) =
            resolve_visible_defs(&reachable, &defs_by_path).expect("defs should resolve");
        assert!(types.contains_key("Obj"));
        let function = functions.get("make").expect("function should exist");
        assert_eq!(function.params.len(), 1);
        assert!(defs_globals.is_empty());
        assert!(matches!(
            function.return_binding.r#type,
            ScriptType::Object { .. }
        ));
    }

    #[test]
    fn resolve_visible_defs_handles_namespace_collisions_and_alias_edges() {
        let span = SourceSpan::synthetic();

        let duplicate_qualified = DefsDeclarations {
            type_decls: vec![ParsedTypeDecl {
                name: "T".to_string(),
                qualified_name: "shared.T".to_string(),
                fields: vec![ParsedTypeFieldDecl {
                    name: "v".to_string(),
                    type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                    location: span.clone(),
                }],
                location: span.clone(),
            }],
            function_decls: Vec::new(),
            defs_global_var_decls: Vec::new(),
        };
        let duplicate_defs_by_path = BTreeMap::from([
            ("a.defs.xml".to_string(), duplicate_qualified.clone()),
            ("b.defs.xml".to_string(), duplicate_qualified),
        ]);
        let duplicate_reachable =
            BTreeSet::from(["a.defs.xml".to_string(), "b.defs.xml".to_string()]);
        let duplicate_error = resolve_visible_defs(&duplicate_reachable, &duplicate_defs_by_path)
            .expect_err("duplicate qualified type should fail");
        assert_eq!(duplicate_error.code, "TYPE_DECL_DUPLICATE");

        let defs_by_path = BTreeMap::from([
            (
                "a.defs.xml".to_string(),
                DefsDeclarations {
                    type_decls: Vec::new(),
                    function_decls: vec![ParsedFunctionDecl {
                        name: "doit".to_string(),
                        qualified_name: "a.doit".to_string(),
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
                },
            ),
            (
                "b.defs.xml".to_string(),
                DefsDeclarations {
                    type_decls: Vec::new(),
                    function_decls: vec![ParsedFunctionDecl {
                        name: "doit".to_string(),
                        qualified_name: "b.doit".to_string(),
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
                },
            ),
        ]);
        let reachable = BTreeSet::from(["a.defs.xml".to_string(), "b.defs.xml".to_string()]);
        let (_types, functions, defs_globals) =
            resolve_visible_defs(&reachable, &defs_by_path).expect("defs should resolve");
        assert!(functions.contains_key("a.doit"));
        assert!(functions.contains_key("b.doit"));
        assert!(!functions.contains_key("doit"));
        assert!(defs_globals.is_empty());
    }

    #[test]
    fn resolve_visible_defs_applies_defs_global_short_alias_rules() {
        let span = SourceSpan::synthetic();
        let make_decl = |namespace: &str, name: &str| ParsedDefsGlobalVarDecl {
            namespace: namespace.to_string(),
            name: name.to_string(),
            qualified_name: format!("{}.{}", namespace, name),
            type_expr: ParsedTypeExpr::Primitive("int".to_string()),
            initial_value_expr: None,
            location: span.clone(),
        };

        let unique_defs = BTreeMap::from([(
            "a.defs.xml".to_string(),
            DefsDeclarations {
                type_decls: Vec::new(),
                function_decls: Vec::new(),
                defs_global_var_decls: vec![make_decl("a", "hp")],
            },
        )]);
        let unique_reachable = BTreeSet::from(["a.defs.xml".to_string()]);
        let (_types, _functions, unique_globals) =
            resolve_visible_defs(&unique_reachable, &unique_defs).expect("defs should resolve");
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
                "a.defs.xml".to_string(),
                DefsDeclarations {
                    type_decls: Vec::new(),
                    function_decls: Vec::new(),
                    defs_global_var_decls: vec![make_decl("a", "hp")],
                },
            ),
            (
                "b.defs.xml".to_string(),
                DefsDeclarations {
                    type_decls: Vec::new(),
                    function_decls: Vec::new(),
                    defs_global_var_decls: vec![make_decl("b", "hp")],
                },
            ),
        ]);
        let collision_reachable =
            BTreeSet::from(["a.defs.xml".to_string(), "b.defs.xml".to_string()]);
        let (_types, _functions, collision_globals) =
            resolve_visible_defs(&collision_reachable, &collision_defs)
                .expect("defs should resolve");
        assert!(collision_globals.contains_key("a.hp"));
        assert!(collision_globals.contains_key("b.hp"));
        assert!(!collision_globals.contains_key("hp"));
    }

    #[test]
    fn compile_bundle_rejects_defs_global_forward_reference() {
        let files = map(&[
            (
                "shared.defs.xml",
                r#"
<defs name="shared">
  <var name="a" type="int">b + 1</var>
  <var name="b" type="int">1</var>
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
        ]);

        let error =
            compile_project_bundle_from_xml_map(&files).expect_err("forward reference should fail");
        assert_eq!(error.code, "DEFS_GLOBAL_INIT_ORDER");
    }

    #[test]
    fn compile_bundle_allows_defs_global_reference_to_initialized_symbol() {
        let files = map(&[
            (
                "shared.defs.xml",
                r#"
<defs name="shared">
  <var name="b" type="int">1</var>
  <var name="a" type="int">b + 1</var>
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
                "shared.defs.xml",
                r#"<defs name="shared"><var name="hp" type="int" value="1"/></defs>"#,
            ),
            (
                "main.script.xml",
                r#"
<!-- include: shared.defs.xml -->
<script name="main"><text>ok</text></script>
"#,
            ),
        ]);
        let value_error = compile_project_bundle_from_xml_map(&files_with_value)
            .expect_err("value attr should fail");
        assert_eq!(value_error.code, "XML_ATTR_NOT_ALLOWED");

        let files_with_child = map(&[
            (
                "shared.defs.xml",
                r#"<defs name="shared"><var name="hp" type="int"><text>1</text></var></defs>"#,
            ),
            (
                "main.script.xml",
                r#"
<!-- include: shared.defs.xml -->
<script name="main"><text>ok</text></script>
"#,
            ),
        ]);
        let child_error = compile_project_bundle_from_xml_map(&files_with_child)
            .expect_err("child element should fail");
        assert_eq!(child_error.code, "XML_VAR_CHILD_INVALID");
    }

    #[test]
    fn defs_global_resolution_rejects_duplicates_and_allows_empty_initializer() {
        let duplicate_types_bundle = map(&[
            (
                "a.defs.xml",
                r#"<defs name="shared"><type name="T"><field name="v" type="int"/></type></defs>"#,
            ),
            (
                "b.defs.xml",
                r#"<defs name="shared"><type name="T"><field name="v" type="int"/></type></defs>"#,
            ),
        ]);
        let duplicate_types_error = compile_project_bundle_from_xml_map(&duplicate_types_bundle)
            .expect_err("bundle duplicate type should fail");
        assert_eq!(duplicate_types_error.code, "TYPE_DECL_DUPLICATE");

        let duplicate_globals_bundle = map(&[
            (
                "a.defs.xml",
                r#"<defs name="shared"><var name="hp" type="int">1</var></defs>"#,
            ),
            (
                "b.defs.xml",
                r#"<defs name="shared"><var name="hp" type="int">2</var></defs>"#,
            ),
        ]);
        let duplicate_globals_error =
            compile_project_bundle_from_xml_map(&duplicate_globals_bundle)
                .expect_err("bundle duplicate defs global should fail");
        assert_eq!(duplicate_globals_error.code, "DEFS_GLOBAL_VAR_DUPLICATE");

        let empty_initializer = map(&[
            (
                "shared.defs.xml",
                r#"<defs name="shared"><var name="hp" type="int"/></defs>"#,
            ),
            (
                "main.script.xml",
                r#"
<!-- include: shared.defs.xml -->
<script name="main"><text>${shared.hp}</text></script>
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
            type_expr: ParsedTypeExpr::Primitive("int".to_string()),
            initial_value_expr: Some("1".to_string()),
            location: span.clone(),
        };
        let defs_by_path = BTreeMap::from([
            (
                "a.defs.xml".to_string(),
                DefsDeclarations {
                    type_decls: Vec::new(),
                    function_decls: Vec::new(),
                    defs_global_var_decls: vec![duplicate.clone()],
                },
            ),
            (
                "b.defs.xml".to_string(),
                DefsDeclarations {
                    type_decls: Vec::new(),
                    function_decls: Vec::new(),
                    defs_global_var_decls: vec![duplicate],
                },
            ),
        ]);
        let reachable = BTreeSet::from(["a.defs.xml".to_string(), "b.defs.xml".to_string()]);
        let error = resolve_visible_defs(&reachable, &defs_by_path)
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
        let bad_defs = map(&[("x.defs.xml", "<script name=\"x\"></script>")]);
        let error = compile_project_bundle_from_xml_map(&bad_defs).expect_err("bad defs root");
        assert_eq!(error.code, "XML_ROOT_INVALID");

        let duplicate_types = map(&[
            (
                "a.defs.xml",
                r#"<defs name="a"><type name="T"><field name="v" type="int"/></type></defs>"#,
            ),
            (
                "b.defs.xml",
                r#"<defs name="b"><type name="T"><field name="v" type="int"/></type></defs>"#,
            ),
            (
                "main.script.xml",
                r#"
    <!-- include: a.defs.xml -->
    <!-- include: b.defs.xml -->
    <script name="main"><var name="v" type="T"/></script>
    "#,
            ),
        ]);
        let error = compile_project_bundle_from_xml_map(&duplicate_types)
            .expect_err("ambiguous unqualified type should fail");
        assert_eq!(error.code, "TYPE_UNKNOWN");

        let recursive = map(&[
            (
                "x.defs.xml",
                r#"<defs name="x"><type name="A"><field name="b" type="B"/></type><type name="B"><field name="a" type="A"/></type></defs>"#,
            ),
            (
                "main.script.xml",
                r#"
    <!-- include: x.defs.xml -->
    <script name="main"><var name="v" type="A"/></script>
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
                "shared.defs.xml",
                r#"<defs name="shared">
  <function name="add" args="int:a,int:b" return="int:result">
    result = a + b;
  </function>
</defs>"#,
            ),
            (
                "main.script.xml",
                r#"
<!-- include: shared.defs.xml -->
<script name="main">
  <code>let x = shared.add(1, 2);</code>
  <text>${x}</text>
</script>
"#,
            ),
        ]);
        let bundle = compile_project_bundle_from_xml_map(&files).expect("compile should pass");
        assert!(bundle.scripts.contains_key("main"));
    }

    #[test]
    fn parse_defs_files_and_type_resolution_success_paths_are_covered() {
        let files = map(&[(
            "shared.defs.xml",
            r#"<defs name="shared">
  <type name="Obj"><field name="value" type="int"/></type>
  <function name="make" args="int:seed" return="Obj:ret">
    ret = #{ value: seed };
  </function>
</defs>"#,
        )]);
        let sources = parse_sources(&files).expect("parse sources");
        let defs_by_path = parse_defs_files(&sources).expect("parse defs");
        let reachable = BTreeSet::from(["shared.defs.xml".to_string()]);
        let (types, functions, _) =
            resolve_visible_defs(&reachable, &defs_by_path).expect("resolve defs");
        assert!(types.contains_key("shared.Obj"));
        assert!(functions.contains_key("shared.make"));
    }
}
