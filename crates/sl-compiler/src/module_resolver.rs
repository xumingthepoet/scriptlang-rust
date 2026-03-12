use crate::*;

struct ParsedModuleHeader {
    namespace: String,
    export_targets: ModuleExportTargets,
}

#[derive(Debug, Clone, Default)]
struct ModuleExportTargets {
    scripts: BTreeSet<String>,
    functions: BTreeSet<String>,
    vars: BTreeSet<String>,
    consts: BTreeSet<String>,
    types: BTreeSet<String>,
    enums: BTreeSet<String>,
}

enum ParsedModuleChild {
    Type(ParsedTypeDecl),
    Function(ParsedFunctionDecl),
    ModuleVar(ParsedModuleVarDecl),
    ModuleConst(ParsedModuleConstDecl),
    Script(ParsedModuleScript),
}

pub(crate) fn parse_module_files(
    sources: &BTreeMap<String, SourceFile>,
) -> Result<BTreeMap<String, ModuleDeclarations>, ScriptLangError> {
    let mut module_by_path = BTreeMap::new();

    for (file_path, source) in sources {
        if !matches!(source.kind, SourceKind::ModuleXml) {
            continue;
        }

        let module = parse_module_source(source, file_path)?;
        module_by_path.insert(file_path.clone(), module.module);
    }

    Ok(module_by_path)
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
) -> Result<ParsedModuleSource, ScriptLangError> {
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
        export_targets,
    } = parse_module_header(root, file_path)?;

    let mut type_decls = Vec::new();
    let mut function_decls = Vec::new();
    let mut module_global_var_decls = Vec::new();
    let mut module_global_const_decls = Vec::new();
    let mut scripts = Vec::new();

    for child in element_children(root) {
        match parse_module_child(child, root, file_path, &namespace)? {
            ParsedModuleChild::Type(decl) => type_decls.push(decl),
            ParsedModuleChild::Function(decl) => function_decls.push(decl),
            ParsedModuleChild::ModuleVar(decl) => module_global_var_decls.push(decl),
            ParsedModuleChild::ModuleConst(decl) => module_global_const_decls.push(decl),
            ParsedModuleChild::Script(script) => scripts.push(script),
        }
    }
    apply_module_export_targets(
        &export_targets,
        &mut type_decls,
        &mut function_decls,
        &mut module_global_var_decls,
        &mut module_global_const_decls,
        &mut scripts,
        &root.location,
    )
    .map_err(|error| with_file_context(error, file_path))?;

    Ok(ParsedModuleSource {
        module: ModuleDeclarations {
            type_decls,
            function_decls,
            module_global_var_decls,
            module_global_const_decls,
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
    let export_targets =
        parse_module_export_targets(root).map_err(|error| with_file_context(error, file_path))?;
    Ok(ParsedModuleHeader {
        namespace,
        export_targets,
    })
}

fn parse_module_child(
    child: &XmlElementNode,
    root: &XmlElementNode,
    file_path: &str,
    namespace: &str,
) -> Result<ParsedModuleChild, ScriptLangError> {
    match child.name.as_str() {
        "type" => {
            parse_type_declaration_node_with_namespace(child, namespace, AccessLevel::Private)
                .map(ParsedModuleChild::Type)
                .map_err(|error| with_file_context(error, file_path))
        }
        "enum" => {
            parse_enum_declaration_node_with_namespace(child, namespace, AccessLevel::Private)
                .map(ParsedModuleChild::Type)
                .map_err(|error| with_file_context(error, file_path))
        }
        "function" => {
            parse_function_declaration_node_with_namespace(child, namespace, AccessLevel::Private)
                .map(ParsedModuleChild::Function)
                .map_err(|error| with_file_context(error, file_path))
        }
        "var" => parse_module_var_declaration(child, namespace, AccessLevel::Private)
            .map(ParsedModuleChild::ModuleVar)
            .map_err(|error| with_file_context(error, file_path)),
        "const" => parse_module_const_declaration(child, namespace, AccessLevel::Private)
            .map(ParsedModuleChild::ModuleConst)
            .map_err(|error| with_file_context(error, file_path)),
        "script" => {
            let script_name = get_required_non_empty_attr(child, "name")
                .map_err(|error| with_file_context(error, file_path))?;
            assert_decl_name_not_reserved_or_rhai_keyword(
                &script_name,
                "script",
                child.location.clone(),
            )
            .map_err(|error| with_file_context(error, file_path))?;
            Ok(ParsedModuleChild::Script(ParsedModuleScript {
                qualified_script_name: format!("{}.{}", namespace, script_name),
                access: AccessLevel::Private,
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

fn parse_module_export_targets(
    root: &XmlElementNode,
) -> Result<ModuleExportTargets, ScriptLangError> {
    let Some(raw) = get_optional_attr(root, "export") else {
        return Ok(ModuleExportTargets::default());
    };
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(ModuleExportTargets::default());
    }

    let mut targets = ModuleExportTargets::default();
    for group in raw.split(';') {
        let group = group.trim();
        if group.is_empty() {
            return Err(ScriptLangError::with_span(
                "XML_EXPORT_INVALID",
                "Attribute \"export\" contains an empty group.",
                root.location.clone(),
            ));
        }
        let Some((kind_raw, names_raw)) = group.split_once(':') else {
            return Err(ScriptLangError::with_span(
                "XML_EXPORT_INVALID",
                format!(
                    "Attribute \"export\" group \"{}\" must be in \"kind:name1,name2\" format.",
                    group
                ),
                root.location.clone(),
            ));
        };
        let kind = kind_raw.trim();
        let names_raw = names_raw.trim();
        if names_raw.is_empty() {
            return Err(ScriptLangError::with_span(
                "XML_EXPORT_INVALID",
                format!(
                    "Attribute \"export\" group \"{}\" must include at least one name.",
                    group
                ),
                root.location.clone(),
            ));
        }
        for name in names_raw.split(',') {
            let name = name.trim();
            if name.is_empty() {
                return Err(ScriptLangError::with_span(
                    "XML_EXPORT_INVALID",
                    format!(
                        "Attribute \"export\" group \"{}\" contains an empty name.",
                        group
                    ),
                    root.location.clone(),
                ));
            }
            let inserted = match kind {
                "script" => targets.scripts.insert(name.to_string()),
                "function" => targets.functions.insert(name.to_string()),
                "var" => targets.vars.insert(name.to_string()),
                "const" => targets.consts.insert(name.to_string()),
                "type" => targets.types.insert(name.to_string()),
                "enum" => targets.enums.insert(name.to_string()),
                _ => {
                    return Err(ScriptLangError::with_span(
                        "XML_EXPORT_KIND_INVALID",
                        format!(
                            "Unsupported export kind \"{}\". Allowed kinds: script/function/var/const/type/enum.",
                            kind
                        ),
                        root.location.clone(),
                    ))
                }
            };
            if !inserted {
                return Err(ScriptLangError::with_span(
                    "XML_EXPORT_DUPLICATE",
                    format!(
                        "Duplicate export entry \"{}:{}\" in module \"export\".",
                        kind, name
                    ),
                    root.location.clone(),
                ));
            }
        }
    }

    Ok(targets)
}

fn apply_module_export_targets(
    export_targets: &ModuleExportTargets,
    type_decls: &mut [ParsedTypeDecl],
    function_decls: &mut [ParsedFunctionDecl],
    module_var_decls: &mut [ParsedModuleVarDecl],
    module_const_decls: &mut [ParsedModuleConstDecl],
    scripts: &mut [ParsedModuleScript],
    span: &SourceSpan,
) -> Result<(), ScriptLangError> {
    let type_names = type_decls
        .iter()
        .filter(|decl| decl.enum_members.is_empty())
        .map(|decl| decl.name.as_str())
        .collect::<BTreeSet<_>>();
    let enum_names = type_decls
        .iter()
        .filter(|decl| !decl.enum_members.is_empty())
        .map(|decl| decl.name.as_str())
        .collect::<BTreeSet<_>>();
    let function_names = function_decls
        .iter()
        .map(|decl| decl.name.as_str())
        .collect::<BTreeSet<_>>();
    let var_names = module_var_decls
        .iter()
        .map(|decl| decl.name.as_str())
        .collect::<BTreeSet<_>>();
    let const_names = module_const_decls
        .iter()
        .map(|decl| decl.name.as_str())
        .collect::<BTreeSet<_>>();
    let script_names = scripts
        .iter()
        .map(|script| {
            script
                .qualified_script_name
                .split_once('.')
                .map(|(_, name)| name)
                .unwrap_or(script.qualified_script_name.as_str())
        })
        .collect::<BTreeSet<_>>();

    validate_export_names("type", &export_targets.types, &type_names, span)?;
    validate_export_names("enum", &export_targets.enums, &enum_names, span)?;
    validate_export_names("function", &export_targets.functions, &function_names, span)?;
    validate_export_names("var", &export_targets.vars, &var_names, span)?;
    validate_export_names("const", &export_targets.consts, &const_names, span)?;
    validate_export_names("script", &export_targets.scripts, &script_names, span)?;

    for decl in type_decls {
        decl.access = if (!decl.enum_members.is_empty()
            && export_targets.enums.contains(&decl.name))
            || (decl.enum_members.is_empty() && export_targets.types.contains(&decl.name))
        {
            AccessLevel::Public
        } else {
            AccessLevel::Private
        };
    }
    for decl in function_decls {
        decl.access = if export_targets.functions.contains(&decl.name) {
            AccessLevel::Public
        } else {
            AccessLevel::Private
        };
    }
    for decl in module_var_decls {
        decl.access = if export_targets.vars.contains(&decl.name) {
            AccessLevel::Public
        } else {
            AccessLevel::Private
        };
    }
    for decl in module_const_decls {
        decl.access = if export_targets.consts.contains(&decl.name) {
            AccessLevel::Public
        } else {
            AccessLevel::Private
        };
    }
    for script in scripts {
        let short_name = script
            .qualified_script_name
            .split_once('.')
            .map(|(_, name)| name)
            .unwrap_or(script.qualified_script_name.as_str());
        script.access = if export_targets.scripts.contains(short_name) {
            AccessLevel::Public
        } else {
            AccessLevel::Private
        };
    }

    Ok(())
}

fn validate_export_names(
    kind: &str,
    exported: &BTreeSet<String>,
    declared: &BTreeSet<&str>,
    span: &SourceSpan,
) -> Result<(), ScriptLangError> {
    for name in exported {
        if declared.contains(name.as_str()) {
            continue;
        }
        return Err(ScriptLangError::with_span(
            "XML_EXPORT_TARGET_NOT_FOUND",
            format!(
                "Export target \"{}:{}\" does not exist in this module.",
                kind, name
            ),
            span.clone(),
        ));
    }
    Ok(())
}

fn collect_namespace_module_symbol_aliases<'a>(
    modules: impl Iterator<Item = &'a ModuleDeclarations>,
) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut aliases = BTreeMap::new();
    for module in modules {
        for decl in &module.module_global_var_decls {
            aliases
                .entry(decl.namespace.clone())
                .or_insert_with(BTreeMap::new)
                .entry(decl.name.clone())
                .or_insert_with(|| decl.qualified_name.clone());
        }
        for decl in &module.module_global_const_decls {
            aliases
                .entry(decl.namespace.clone())
                .or_insert_with(BTreeMap::new)
                .entry(decl.name.clone())
                .or_insert_with(|| decl.qualified_name.clone());
        }
    }
    aliases
}

fn collect_module_symbol_targets<'a>(
    modules: impl Iterator<Item = &'a ModuleDeclarations>,
) -> BTreeSet<String> {
    let mut targets = BTreeSet::new();
    for module in modules {
        for decl in &module.module_global_var_decls {
            targets.insert(decl.qualified_name.clone());
        }
        for decl in &module.module_global_const_decls {
            targets.insert(decl.qualified_name.clone());
        }
    }
    targets
}

fn collect_module_explicit_visible_symbol_aliases(
    module_alias_directives_by_namespace: &BTreeMap<String, Vec<AliasDirective>>,
    module_symbol_targets: &BTreeSet<String>,
) -> Result<BTreeMap<String, BTreeMap<String, String>>, ScriptLangError> {
    let mut aliases_by_namespace = BTreeMap::new();

    for (namespace, directives) in module_alias_directives_by_namespace {
        let mut aliases = BTreeMap::new();
        for directive in directives {
            let target = directive.target_qualified_name.as_str();
            if !module_symbol_targets.contains(target) {
                continue;
            }
            let alias = directive.alias_name.as_str();
            if let Some(existing_target) = aliases.get(alias) {
                if existing_target == target {
                    continue;
                }
                return Err(ScriptLangError::new(
                    "ALIAS_NAME_CONFLICT",
                    format!(
                        "Alias \"{}\" points to both \"{}\" and \"{}\".",
                        alias, existing_target, target
                    ),
                ));
            }
            aliases.insert(alias.to_string(), target.to_string());
        }
        if !aliases.is_empty() {
            aliases_by_namespace.insert(namespace.clone(), aliases);
        }
    }

    Ok(aliases_by_namespace)
}

fn merge_namespace_module_symbol_aliases(
    base: &mut BTreeMap<String, BTreeMap<String, String>>,
    explicit: &BTreeMap<String, BTreeMap<String, String>>,
) -> Result<(), ScriptLangError> {
    for (namespace, explicit_aliases) in explicit {
        let namespace_aliases = base.entry(namespace.clone()).or_default();
        for (alias, target) in explicit_aliases {
            if let Some(existing_target) = namespace_aliases.get(alias) {
                if existing_target == target {
                    continue;
                }
                return Err(ScriptLangError::new(
                    "ALIAS_NAME_CONFLICT",
                    format!(
                        "Alias \"{}\" points to both \"{}\" and \"{}\".",
                        alias, existing_target, target
                    ),
                ));
            }
            namespace_aliases.insert(alias.clone(), target.clone());
        }
    }
    Ok(())
}

fn namespace_alias_rewrite_map(
    namespace_aliases_by_namespace: &BTreeMap<String, BTreeMap<String, String>>,
    namespace: &str,
    blocked_names: &BTreeSet<String>,
) -> BTreeMap<String, String> {
    let mut map = namespace_aliases_by_namespace
        .get(namespace)
        .cloned()
        .unwrap_or_default();
    for name in blocked_names {
        map.remove(name);
    }
    map
}

fn normalize_module_initializer(
    expr: &Option<String>,
    resolved_type: &ScriptType,
    alias_rewrite_map: &BTreeMap<String, String>,
    visible_types: &BTreeMap<String, ScriptType>,
    visible_functions: &BTreeMap<String, FunctionDecl>,
    module_name: &str,
    span: &SourceSpan,
) -> Result<Option<String>, ScriptLangError> {
    let Some(expr) = expr.as_ref() else {
        if matches!(resolved_type, ScriptType::Enum { .. }) {
            return Err(ScriptLangError::with_span(
                "ENUM_INIT_REQUIRED",
                "Enum declaration requires explicit Type.Member initializer.",
                span.clone(),
            ));
        }
        return Ok(None);
    };

    if let ScriptType::Enum { type_name, members } = resolved_type {
        let member = parse_enum_literal_initializer(expr, type_name, members, visible_types, span)?;
        return Ok(Some(format!("\"{}\"", member.replace('"', "\\\""))));
    }
    if let ScriptType::Map {
        key_type: MapKeyType::Enum { type_name, members },
        ..
    } = resolved_type
    {
        validate_enum_map_initializer_keys_if_static(expr, type_name, members, span)?;
    }

    let alias_rewritten = rewrite_module_symbol_aliases_in_expression(expr, alias_rewrite_map);
    let script_rewritten = normalize_and_validate_script_literals_in_expression(
        &alias_rewritten,
        span,
        Some(module_name),
        None,
    )?;
    let function_rewritten = normalize_and_validate_function_literals(
        &script_rewritten,
        span,
        Some(module_name),
        visible_functions,
    )?;
    Ok(Some(rewrite_and_validate_enum_literals_in_expression(
        &function_rewritten,
        visible_types,
        span,
    )?))
}

pub(crate) fn parse_module_var_declaration(
    node: &XmlElementNode,
    namespace: &str,
    declared_access: AccessLevel,
) -> Result<ParsedModuleVarDecl, ScriptLangError> {
    let parsed = parse_module_binding_declaration(node, namespace, declared_access, "var")?;
    Ok(ParsedModuleVarDecl {
        namespace: parsed.namespace,
        name: parsed.name,
        qualified_name: parsed.qualified_name,
        access: parsed.access,
        type_expr: parsed.type_expr,
        initial_value_expr: parsed.initial_value_expr,
        location: parsed.location,
    })
}

pub(crate) fn parse_module_const_declaration(
    node: &XmlElementNode,
    namespace: &str,
    declared_access: AccessLevel,
) -> Result<ParsedModuleConstDecl, ScriptLangError> {
    let parsed = parse_module_binding_declaration(node, namespace, declared_access, "const")?;
    Ok(ParsedModuleConstDecl {
        namespace: parsed.namespace,
        name: parsed.name,
        qualified_name: parsed.qualified_name,
        access: parsed.access,
        type_expr: parsed.type_expr,
        initial_value_expr: parsed.initial_value_expr,
        location: parsed.location,
    })
}

fn parse_module_binding_declaration(
    node: &XmlElementNode,
    namespace: &str,
    declared_access: AccessLevel,
    tag_name: &str,
) -> Result<ParsedModuleVarDecl, ScriptLangError> {
    let name = get_required_non_empty_attr(node, "name")?;
    assert_decl_name_not_reserved_or_rhai_keyword(&name, "module global", node.location.clone())?;

    let type_raw = get_required_non_empty_attr(node, "type")?;
    let type_expr = parse_type_expr(&type_raw, &node.location)?;

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

    Ok(ParsedModuleVarDecl {
        namespace: namespace.to_string(),
        name: name.clone(),
        qualified_name: format!("{}.{}", namespace, name),
        access: declared_access,
        type_expr,
        initial_value_expr,
        location: node.location.clone(),
    })
}

#[allow(dead_code)]
pub(crate) fn collect_global_data(
    sources: &BTreeMap<String, SourceFile>,
) -> Result<BTreeMap<String, SlValue>, ScriptLangError> {
    let mut out = BTreeMap::new();

    for (file_path, source) in sources {
        if !matches!(source.kind, SourceKind::Json) {
            continue;
        }
        let symbol = parse_global_data_symbol(file_path)?;
        if out.contains_key(&symbol) {
            return Err(ScriptLangError::new(
                "GLOBAL_DATA_SYMBOL_DUPLICATE",
                format!("Duplicate global data symbol \"{}\".", symbol),
            ));
        }
        let value = source.json_value.clone().ok_or(ScriptLangError::new(
            "GLOBAL_DATA_MISSING_VALUE",
            "Missing global data value.",
        ))?;
        out.insert(symbol, value);
    }

    Ok(out)
}

#[allow(dead_code)]
pub(crate) fn collect_visible_global_symbols(
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

        let symbol = parse_global_data_symbol(file_path)?;
        if !seen.insert(symbol.clone()) {
            return Err(ScriptLangError::new(
                "GLOBAL_DATA_SYMBOL_DUPLICATE",
                format!(
                    "Duplicate global data symbol \"{}\" in visible closure.",
                    symbol
                ),
            ));
        }
        symbols.push(symbol);
    }

    symbols.sort();
    Ok(symbols)
}

#[allow(dead_code)]
pub(crate) fn parse_global_data_symbol(file_path: &str) -> Result<String, ScriptLangError> {
    let path = Path::new(file_path);
    let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
        return Err(ScriptLangError::new(
            "GLOBAL_DATA_SYMBOL_INVALID",
            format!("Invalid global data file name: {}", file_path),
        ));
    };

    if !global_data_symbol_regex().is_match(stem) {
        return Err(ScriptLangError::new(
            "GLOBAL_DATA_SYMBOL_INVALID",
            format!(
                "global data basename \"{}\" is not a valid identifier.",
                stem
            ),
        ));
    }

    assert_name_not_reserved(stem, "global data symbol", SourceSpan::synthetic())?;
    Ok(stem.to_string())
}

#[allow(dead_code)]
pub(crate) fn global_data_symbol_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"^[$A-Za-z_][$0-9A-Za-z_]*$").expect("global data symbol regex must compile")
    })
}

#[cfg(test)]
pub(crate) fn resolve_visible_module_symbols(
    reachable: &BTreeSet<String>,
    module_by_path: &BTreeMap<String, ModuleDeclarations>,
    local_module_name: Option<&str>,
) -> Result<VisibleModuleResolution, ScriptLangError> {
    resolve_visible_module_symbols_with_aliases_and_module_scoped_type_aliases(
        reachable,
        module_by_path,
        local_module_name,
        &[],
        &BTreeMap::new(),
    )
}

#[cfg(test)]
pub(crate) fn resolve_visible_module_symbols_with_aliases(
    reachable: &BTreeSet<String>,
    module_by_path: &BTreeMap<String, ModuleDeclarations>,
    local_module_name: Option<&str>,
    alias_directives: &[AliasDirective],
) -> Result<VisibleModuleResolution, ScriptLangError> {
    resolve_visible_module_symbols_with_aliases_and_module_scoped_type_aliases(
        reachable,
        module_by_path,
        local_module_name,
        alias_directives,
        &BTreeMap::new(),
    )
}

pub(crate) fn resolve_visible_module_symbols_with_aliases_and_module_scoped_type_aliases(
    reachable: &BTreeSet<String>,
    module_by_path: &BTreeMap<String, ModuleDeclarations>,
    local_module_name: Option<&str>,
    alias_directives: &[AliasDirective],
    module_alias_directives_by_namespace: &BTreeMap<String, Vec<AliasDirective>>,
) -> Result<VisibleModuleResolution, ScriptLangError> {
    let mut type_decls_map: BTreeMap<String, ParsedTypeDecl> = BTreeMap::new();
    let mut local_type_short_candidates: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut namespace_type_aliases: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();

    for path in reachable {
        let Some(module) = module_by_path.get(path) else {
            continue;
        };
        for decl in &module.type_decls {
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
    let explicit_type_aliases =
        collect_explicit_visible_type_aliases(alias_directives, &type_decls_map)?;
    let module_scoped_explicit_type_aliases = collect_module_explicit_visible_type_aliases(
        module_alias_directives_by_namespace,
        &type_decls_map,
    )?;

    let mut resolved_types: BTreeMap<String, ScriptType> = BTreeMap::new();
    let mut visiting = HashSet::new();

    for type_name in type_decls_map.keys() {
        let namespace = type_name
            .split_once('.')
            .map(|(namespace, _)| namespace)
            .unwrap_or_default();
        let mut aliases = namespace_type_aliases
            .get(namespace)
            .cloned()
            .unwrap_or_default();
        if let Some(module_scoped_aliases) = module_scoped_explicit_type_aliases.get(namespace) {
            for (alias, qualified_name) in module_scoped_aliases {
                aliases
                    .entry(alias.clone())
                    .or_insert_with(|| qualified_name.clone());
            }
        }
        if local_module_name == Some(namespace) {
            for (alias, qualified_name) in &explicit_type_aliases {
                aliases
                    .entry(alias.clone())
                    .or_insert_with(|| qualified_name.clone());
            }
        }
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
    let mut local_visible_types = visible_types.clone();
    for (alias, qualified_name) in &explicit_type_aliases {
        let ty = resolved_types
            .get(qualified_name)
            .cloned()
            .expect("explicit alias type target should exist in resolved type map");
        local_visible_types.entry(alias.clone()).or_insert(ty);
    }
    let reachable_modules = reachable.iter().filter_map(|path| module_by_path.get(path));
    let module_symbol_targets =
        collect_module_symbol_targets(reachable.iter().filter_map(|path| module_by_path.get(path)));
    let module_scoped_explicit_symbol_aliases = collect_module_explicit_visible_symbol_aliases(
        module_alias_directives_by_namespace,
        &module_symbol_targets,
    )?;
    let mut namespace_aliases_by_namespace =
        collect_namespace_module_symbol_aliases(reachable_modules);
    merge_namespace_module_symbol_aliases(
        &mut namespace_aliases_by_namespace,
        &module_scoped_explicit_symbol_aliases,
    )?;

    let mut visible_function_names = BTreeSet::new();
    for path in reachable {
        let Some(module) = module_by_path.get(path) else {
            continue;
        };
        for decl in &module.function_decls {
            if !visible_function_names.insert(decl.qualified_name.clone()) {
                return Err(ScriptLangError::with_span(
                    "FUNCTION_DECL_DUPLICATE",
                    format!(
                        "Duplicate function declaration \"{}\".",
                        decl.qualified_name
                    ),
                    decl.location.clone(),
                ));
            }
        }
    }

    let mut functions: BTreeMap<String, FunctionDecl> = BTreeMap::new();
    let mut function_short_candidates: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for path in reachable {
        let Some(module) = module_by_path.get(path) else {
            continue;
        };

        for decl in &module.function_decls {
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
            let visible_types_base = if is_local {
                &local_visible_types
            } else {
                &visible_types
            };
            let visible_types_in_scope = visible_types_with_namespace_type_aliases(
                visible_types_base,
                &resolved_types,
                module_scoped_explicit_type_aliases.get(function_namespace),
            );

            let mut params = Vec::new();
            for param in &decl.params {
                params.push(FunctionParam {
                    name: param.name.clone(),
                    r#type: resolve_type_expr_in_namespace(
                        &param.type_expr,
                        &visible_types_in_scope,
                        function_namespace,
                        &param.location,
                    )?,
                    location: param.location.clone(),
                });
            }

            let rb = &decl.return_decl;
            let return_type = resolve_type_expr_in_namespace(
                &rb.type_expr,
                &visible_types_in_scope,
                function_namespace,
                &rb.location,
            )?;
            let blocked_names = decl
                .params
                .iter()
                .map(|param| param.name.clone())
                .collect::<BTreeSet<_>>();
            let alias_rewrite_map = namespace_alias_rewrite_map(
                &namespace_aliases_by_namespace,
                function_namespace,
                &blocked_names,
            );
            let alias_rewritten_code =
                rewrite_module_symbol_aliases_in_expression(&decl.code, &alias_rewrite_map);
            let script_rewritten_code = normalize_and_validate_script_literals_in_expression(
                &alias_rewritten_code,
                &decl.location,
                Some(function_namespace),
                None,
            )?;
            let function_rewritten_code = normalize_and_validate_function_literals_with_names(
                &script_rewritten_code,
                &decl.location,
                Some(function_namespace),
                &visible_function_names,
            )?;
            let normalized_code = rewrite_and_validate_enum_literals_in_expression(
                &function_rewritten_code,
                &visible_types_in_scope,
                &decl.location,
            )?;

            functions.insert(
                decl.qualified_name.clone(),
                FunctionDecl {
                    name: decl.qualified_name.clone(),
                    params,
                    return_binding: FunctionReturn {
                        r#type: return_type,
                        location: decl.return_decl.location.clone(),
                    },
                    code: normalized_code,
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

    let mut module_vars_qualified = BTreeMap::new();
    let mut module_global_short_candidates: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for path in reachable {
        let Some(module) = module_by_path.get(path) else {
            continue;
        };

        for decl in &module.module_global_var_decls {
            let is_local = local_module_name == Some(decl.namespace.as_str());
            if !is_local && decl.access != AccessLevel::Public {
                continue;
            }
            let visible_types_base = if is_local {
                &local_visible_types
            } else {
                &visible_types
            };
            let visible_types_in_scope = visible_types_with_namespace_type_aliases(
                visible_types_base,
                &resolved_types,
                module_scoped_explicit_type_aliases.get(decl.namespace.as_str()),
            );
            if module_vars_qualified.contains_key(&decl.qualified_name) {
                return Err(ScriptLangError::with_span(
                    "MODULE_GLOBAL_VAR_DUPLICATE",
                    format!(
                        "Duplicate module global variable declaration \"{}\".",
                        decl.qualified_name
                    ),
                    decl.location.clone(),
                ));
            }
            module_vars_qualified.insert(decl.qualified_name.clone(), {
                let resolved_type = resolve_type_expr_in_namespace(
                    &decl.type_expr,
                    &visible_types_in_scope,
                    &decl.namespace,
                    &decl.location,
                )?;
                let alias_rewrite_map = namespace_alias_rewrite_map(
                    &namespace_aliases_by_namespace,
                    &decl.namespace,
                    &BTreeSet::new(),
                );
                let initial_value_expr = normalize_module_initializer(
                    &decl.initial_value_expr,
                    &resolved_type,
                    &alias_rewrite_map,
                    &visible_types_in_scope,
                    &functions,
                    &decl.namespace,
                    &decl.location,
                )?;
                ModuleVarDecl {
                    namespace: decl.namespace.clone(),
                    name: decl.name.clone(),
                    qualified_name: decl.qualified_name.clone(),
                    access: decl.access,
                    r#type: resolved_type,
                    initial_value_expr,
                    location: decl.location.clone(),
                }
            });
            if is_local {
                module_global_short_candidates
                    .entry(decl.name.clone())
                    .or_default()
                    .push(decl.qualified_name.clone());
            }
        }
    }

    let mut module_vars = module_vars_qualified.clone();
    for (alias, qualified_names) in module_global_short_candidates {
        let qualified_name = &qualified_names[0];
        let decl = module_vars_qualified
            .get(qualified_name)
            .cloned()
            .expect("module global alias target should exist");
        module_vars.entry(alias).or_insert(decl);
    }

    let mut module_consts_qualified = BTreeMap::new();
    let mut module_const_short_candidates: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for path in reachable {
        let Some(module) = module_by_path.get(path) else {
            continue;
        };

        for decl in &module.module_global_const_decls {
            let is_local = local_module_name == Some(decl.namespace.as_str());
            if !is_local && decl.access != AccessLevel::Public {
                continue;
            }
            let visible_types_base = if is_local {
                &local_visible_types
            } else {
                &visible_types
            };
            let visible_types_in_scope = visible_types_with_namespace_type_aliases(
                visible_types_base,
                &resolved_types,
                module_scoped_explicit_type_aliases.get(decl.namespace.as_str()),
            );
            if module_consts_qualified.contains_key(&decl.qualified_name) {
                return Err(ScriptLangError::with_span(
                    "MODULE_GLOBAL_CONST_DUPLICATE",
                    format!(
                        "Duplicate module global const declaration \"{}\".",
                        decl.qualified_name
                    ),
                    decl.location.clone(),
                ));
            }
            module_consts_qualified.insert(decl.qualified_name.clone(), {
                let resolved_type = resolve_type_expr_in_namespace(
                    &decl.type_expr,
                    &visible_types_in_scope,
                    &decl.namespace,
                    &decl.location,
                )?;
                let alias_rewrite_map = namespace_alias_rewrite_map(
                    &namespace_aliases_by_namespace,
                    &decl.namespace,
                    &BTreeSet::new(),
                );
                let initial_value_expr = normalize_module_initializer(
                    &decl.initial_value_expr,
                    &resolved_type,
                    &alias_rewrite_map,
                    &visible_types_in_scope,
                    &functions,
                    &decl.namespace,
                    &decl.location,
                )?;
                ModuleConstDecl {
                    namespace: decl.namespace.clone(),
                    name: decl.name.clone(),
                    qualified_name: decl.qualified_name.clone(),
                    access: decl.access,
                    r#type: resolved_type,
                    initial_value_expr,
                    location: decl.location.clone(),
                }
            });
            if is_local {
                module_const_short_candidates
                    .entry(decl.name.clone())
                    .or_default()
                    .push(decl.qualified_name.clone());
            }
        }
    }

    let mut module_consts = module_consts_qualified.clone();
    for (alias, qualified_names) in module_const_short_candidates {
        let qualified_name = &qualified_names[0];
        let decl = module_consts_qualified
            .get(qualified_name)
            .cloned()
            .expect("module const alias target should exist");
        module_consts.entry(alias).or_insert(decl);
    }

    apply_explicit_alias_directives(
        alias_directives,
        &mut visible_types,
        &functions,
        &mut module_vars,
        &mut module_consts,
    )?;

    Ok((visible_types, functions, module_vars, module_consts))
}

fn collect_module_explicit_visible_type_aliases(
    module_alias_directives_by_namespace: &BTreeMap<String, Vec<AliasDirective>>,
    type_decls_map: &BTreeMap<String, ParsedTypeDecl>,
) -> Result<BTreeMap<String, BTreeMap<String, String>>, ScriptLangError> {
    let mut aliases_by_namespace = BTreeMap::new();
    for (namespace, directives) in module_alias_directives_by_namespace {
        let aliases = collect_explicit_visible_type_aliases(directives, type_decls_map)?;
        if aliases.is_empty() {
            continue;
        }
        aliases_by_namespace.insert(namespace.clone(), aliases);
    }
    Ok(aliases_by_namespace)
}

fn visible_types_with_namespace_type_aliases(
    base_visible_types: &BTreeMap<String, ScriptType>,
    resolved_types: &BTreeMap<String, ScriptType>,
    namespace_alias_targets: Option<&BTreeMap<String, String>>,
) -> BTreeMap<String, ScriptType> {
    let Some(namespace_alias_targets) = namespace_alias_targets else {
        return base_visible_types.clone();
    };
    let mut scoped = base_visible_types.clone();
    for (alias, qualified_name) in namespace_alias_targets {
        let Some(resolved_type) = resolved_types.get(qualified_name) else {
            continue;
        };
        scoped
            .entry(alias.clone())
            .or_insert_with(|| resolved_type.clone());
    }
    scoped
}

fn collect_explicit_visible_type_aliases(
    alias_directives: &[AliasDirective],
    type_decls_map: &BTreeMap<String, ParsedTypeDecl>,
) -> Result<BTreeMap<String, String>, ScriptLangError> {
    let mut explicit_alias_target_by_name = BTreeMap::new();

    for directive in alias_directives {
        let target = directive.target_qualified_name.as_str();
        if !type_decls_map.contains_key(target) {
            continue;
        }
        let alias = directive.alias_name.as_str();
        if let Some(existing_target) = explicit_alias_target_by_name.get(alias) {
            if existing_target == target {
                continue;
            }
            return Err(ScriptLangError::new(
                "ALIAS_NAME_CONFLICT",
                format!(
                    "Alias \"{}\" points to both \"{}\" and \"{}\".",
                    alias, existing_target, target
                ),
            ));
        }
        explicit_alias_target_by_name.insert(alias.to_string(), target.to_string());
    }

    Ok(explicit_alias_target_by_name)
}

fn apply_explicit_alias_directives(
    alias_directives: &[AliasDirective],
    visible_types: &mut BTreeMap<String, ScriptType>,
    visible_functions: &BTreeMap<String, FunctionDecl>,
    visible_module_vars: &mut BTreeMap<String, ModuleVarDecl>,
    visible_module_consts: &mut BTreeMap<String, ModuleConstDecl>,
) -> Result<(), ScriptLangError> {
    let mut explicit_alias_target_by_name = BTreeMap::new();

    for directive in alias_directives {
        let target = directive.target_qualified_name.as_str();
        let alias = directive.alias_name.as_str();

        if let Some(existing_target) = explicit_alias_target_by_name.get(alias) {
            if existing_target == target {
                continue;
            }
            return Err(ScriptLangError::new(
                "ALIAS_NAME_CONFLICT",
                format!(
                    "Alias \"{}\" points to both \"{}\" and \"{}\".",
                    alias, existing_target, target
                ),
            ));
        }

        if let Some(target_type) = visible_types.get(target).cloned() {
            if visible_types.contains_key(alias) {
                return Err(ScriptLangError::new(
                    "ALIAS_NAME_CONFLICT",
                    format!(
                        "Alias name \"{}\" conflicts with existing visible type.",
                        alias
                    ),
                ));
            }
            visible_types.insert(alias.to_string(), target_type);
            explicit_alias_target_by_name.insert(alias.to_string(), target.to_string());
            continue;
        }

        if let Some(target_var) = visible_module_vars.get(target).cloned() {
            if visible_module_vars.contains_key(alias) {
                return Err(ScriptLangError::new(
                    "ALIAS_NAME_CONFLICT",
                    format!(
                        "Alias name \"{}\" conflicts with existing visible module variable.",
                        alias
                    ),
                ));
            }
            visible_module_vars.insert(alias.to_string(), target_var);
            explicit_alias_target_by_name.insert(alias.to_string(), target.to_string());
            continue;
        }

        if let Some(target_const) = visible_module_consts.get(target).cloned() {
            if visible_module_consts.contains_key(alias) {
                return Err(ScriptLangError::new(
                    "ALIAS_NAME_CONFLICT",
                    format!(
                        "Alias name \"{}\" conflicts with existing visible module constant.",
                        alias
                    ),
                ));
            }
            visible_module_consts.insert(alias.to_string(), target_const);
            explicit_alias_target_by_name.insert(alias.to_string(), target.to_string());
            continue;
        }

        if visible_functions.contains_key(target) {
            return Err(ScriptLangError::new(
                "ALIAS_TARGET_KIND_UNSUPPORTED",
                format!(
                    "Alias target \"{}\" is a function. Alias only supports type/module var/module const.",
                    target
                ),
            ));
        }

        return Err(ScriptLangError::new(
            "ALIAS_TARGET_NOT_FOUND",
            format!(
                "Alias target \"{}\" is not visible in current module closure.",
                target
            ),
        ));
    }

    Ok(())
}

pub(crate) fn collect_functions_for_bundle_with_aliases(
    module_by_path: &BTreeMap<String, ModuleDeclarations>,
    module_alias_directives_by_namespace: &BTreeMap<String, Vec<AliasDirective>>,
) -> Result<BTreeMap<String, FunctionDecl>, ScriptLangError> {
    let mut type_decls_map: BTreeMap<String, ParsedTypeDecl> = BTreeMap::new();
    let mut type_short_candidates: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for module in module_by_path.values() {
        for decl in &module.type_decls {
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
    let module_symbol_targets = collect_module_symbol_targets(module_by_path.values());
    let module_scoped_explicit_symbol_aliases = collect_module_explicit_visible_symbol_aliases(
        module_alias_directives_by_namespace,
        &module_symbol_targets,
    )?;
    let mut namespace_aliases_by_namespace =
        collect_namespace_module_symbol_aliases(module_by_path.values());
    merge_namespace_module_symbol_aliases(
        &mut namespace_aliases_by_namespace,
        &module_scoped_explicit_symbol_aliases,
    )?;

    let mut visible_function_names = BTreeSet::new();
    for module in module_by_path.values() {
        for decl in &module.function_decls {
            if !visible_function_names.insert(decl.qualified_name.clone()) {
                return Err(ScriptLangError::with_span(
                    "FUNCTION_DECL_DUPLICATE",
                    format!(
                        "Duplicate function declaration \"{}\".",
                        decl.qualified_name
                    ),
                    decl.location.clone(),
                ));
            }
        }
    }

    let mut functions = BTreeMap::new();
    for module in module_by_path.values() {
        for decl in &module.function_decls {
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
                &decl.return_decl.type_expr,
                &visible_types,
                function_namespace,
                &decl.return_decl.location,
            )?;
            let blocked_names = decl
                .params
                .iter()
                .map(|param| param.name.clone())
                .collect::<BTreeSet<_>>();
            let alias_rewrite_map = namespace_alias_rewrite_map(
                &namespace_aliases_by_namespace,
                function_namespace,
                &blocked_names,
            );
            let alias_rewritten_code =
                rewrite_module_symbol_aliases_in_expression(&decl.code, &alias_rewrite_map);
            let script_rewritten_code = normalize_and_validate_script_literals_in_expression(
                &alias_rewritten_code,
                &decl.location,
                Some(function_namespace),
                None,
            )?;
            let function_rewritten_code = normalize_and_validate_function_literals_with_names(
                &script_rewritten_code,
                &decl.location,
                Some(function_namespace),
                &visible_function_names,
            )?;
            let normalized_code = rewrite_and_validate_enum_literals_in_expression(
                &function_rewritten_code,
                &visible_types,
                &decl.location,
            )?;

            functions.insert(
                decl.qualified_name.clone(),
                FunctionDecl {
                    name: decl.qualified_name.clone(),
                    params,
                    return_binding: FunctionReturn {
                        r#type: return_type,
                        location: decl.return_decl.location.clone(),
                    },
                    code: normalized_code,
                    location: decl.location.clone(),
                },
            );
        }
    }

    Ok(functions)
}

pub(crate) fn collect_module_vars_for_bundle(
    module_by_path: &BTreeMap<String, ModuleDeclarations>,
    visible_functions: &BTreeMap<String, FunctionDecl>,
) -> Result<(BTreeMap<String, ModuleVarDecl>, Vec<String>), ScriptLangError> {
    let mut type_decls_map: BTreeMap<String, ParsedTypeDecl> = BTreeMap::new();
    let mut type_short_candidates: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for module in module_by_path.values() {
        for decl in &module.type_decls {
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
    let namespace_aliases_by_namespace =
        collect_namespace_module_symbol_aliases(module_by_path.values());

    let mut module_vars = BTreeMap::new();
    let mut init_order = Vec::new();
    for module in module_by_path.values() {
        for decl in &module.module_global_var_decls {
            if module_vars.contains_key(&decl.qualified_name) {
                return Err(ScriptLangError::with_span(
                    "MODULE_GLOBAL_VAR_DUPLICATE",
                    format!(
                        "Duplicate module global variable declaration \"{}\".",
                        decl.qualified_name
                    ),
                    decl.location.clone(),
                ));
            }
            module_vars.insert(decl.qualified_name.clone(), {
                let resolved_type =
                    resolve_type_expr(&decl.type_expr, &visible_types, &decl.location)?;
                let alias_rewrite_map = namespace_alias_rewrite_map(
                    &namespace_aliases_by_namespace,
                    &decl.namespace,
                    &BTreeSet::new(),
                );
                let initial_value_expr = normalize_module_initializer(
                    &decl.initial_value_expr,
                    &resolved_type,
                    &alias_rewrite_map,
                    &visible_types,
                    visible_functions,
                    &decl.namespace,
                    &decl.location,
                )?;
                ModuleVarDecl {
                    namespace: decl.namespace.clone(),
                    name: decl.name.clone(),
                    qualified_name: decl.qualified_name.clone(),
                    access: decl.access,
                    r#type: resolved_type,
                    initial_value_expr,
                    location: decl.location.clone(),
                }
            });
            init_order.push(decl.qualified_name.clone());
        }
    }

    validate_module_var_init_order(&module_vars, &init_order)?;
    Ok((module_vars, init_order))
}

pub(crate) fn collect_module_consts_for_bundle(
    module_by_path: &BTreeMap<String, ModuleDeclarations>,
    module_vars: &BTreeMap<String, ModuleVarDecl>,
    visible_functions: &BTreeMap<String, FunctionDecl>,
) -> Result<(BTreeMap<String, ModuleConstDecl>, Vec<String>), ScriptLangError> {
    let mut type_decls_map: BTreeMap<String, ParsedTypeDecl> = BTreeMap::new();
    let mut type_short_candidates: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for module in module_by_path.values() {
        for decl in &module.type_decls {
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
    let namespace_aliases_by_namespace =
        collect_namespace_module_symbol_aliases(module_by_path.values());

    let mut module_consts = BTreeMap::new();
    let mut init_order = Vec::new();
    for module in module_by_path.values() {
        for decl in &module.module_global_const_decls {
            if module_consts.contains_key(&decl.qualified_name) {
                return Err(ScriptLangError::with_span(
                    "MODULE_GLOBAL_CONST_DUPLICATE",
                    format!(
                        "Duplicate module global const declaration \"{}\".",
                        decl.qualified_name
                    ),
                    decl.location.clone(),
                ));
            }
            module_consts.insert(decl.qualified_name.clone(), {
                let resolved_type =
                    resolve_type_expr(&decl.type_expr, &visible_types, &decl.location)?;
                let alias_rewrite_map = namespace_alias_rewrite_map(
                    &namespace_aliases_by_namespace,
                    &decl.namespace,
                    &BTreeSet::new(),
                );
                let initial_value_expr = normalize_module_initializer(
                    &decl.initial_value_expr,
                    &resolved_type,
                    &alias_rewrite_map,
                    &visible_types,
                    visible_functions,
                    &decl.namespace,
                    &decl.location,
                )?;
                ModuleConstDecl {
                    namespace: decl.namespace.clone(),
                    name: decl.name.clone(),
                    qualified_name: decl.qualified_name.clone(),
                    access: decl.access,
                    r#type: resolved_type,
                    initial_value_expr,
                    location: decl.location.clone(),
                }
            });
            init_order.push(decl.qualified_name.clone());
        }
    }

    validate_module_const_init_rules(&module_consts, &init_order, module_vars)?;
    Ok((module_consts, init_order))
}

pub(crate) fn validate_module_var_init_order(
    module_vars: &BTreeMap<String, ModuleVarDecl>,
    init_order: &[String],
) -> Result<(), ScriptLangError> {
    let mut name_candidates: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (qualified, decl) in module_vars {
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
        let decl = module_vars
            .get(qualified)
            .expect("init order should only contain declared module globals");
        if let Some(expr) = &decl.initial_value_expr {
            let sanitized = sanitize_rhai_source(expr);
            for (name, target_qualified) in &name_to_qualified {
                if !contains_root_identifier(&sanitized, name) {
                    continue;
                }
                if !initialized.contains(target_qualified) {
                    return Err(ScriptLangError::with_span(
                        "MODULE_GLOBAL_INIT_ORDER",
                        format!(
                            "Module global \"{}\" initializer references \"{}\" before initialization.",
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

pub(crate) fn validate_module_const_init_rules(
    module_consts: &BTreeMap<String, ModuleConstDecl>,
    init_order: &[String],
    module_vars: &BTreeMap<String, ModuleVarDecl>,
) -> Result<(), ScriptLangError> {
    let mut const_name_candidates: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (qualified, decl) in module_consts {
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
    for (qualified, decl) in module_vars {
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
        let decl = module_consts
            .get(qualified)
            .expect("init order should only contain declared module consts");
        if let Some(expr) = &decl.initial_value_expr {
            let sanitized = sanitize_rhai_source(expr);
            for (name, target_qualified) in &var_name_to_qualified {
                if contains_root_identifier(&sanitized, name) {
                    return Err(ScriptLangError::with_span(
                        "MODULE_CONST_INIT_REF_NON_CONST",
                        format!(
                            "Module const \"{}\" initializer references mutable module global \"{}\".",
                            qualified, name
                        ),
                        decl.location.clone(),
                    ));
                }
                if contains_root_identifier(&sanitized, target_qualified) {
                    return Err(ScriptLangError::with_span(
                        "MODULE_CONST_INIT_REF_NON_CONST",
                        format!(
                            "Module const \"{}\" initializer references mutable module global \"{}\".",
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
                        "MODULE_CONST_INIT_ORDER",
                        format!(
                            "Module const \"{}\" initializer references \"{}\" before initialization.",
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
mod module_resolver_tests {
    use super::*;
    use crate::compiler_test_support::*;

    fn script_type_kind(ty: &ScriptType) -> &'static str {
        match ty {
            ScriptType::Primitive { .. } => "primitive",
            ScriptType::Enum { .. } => "enum",
            ScriptType::Script => "script",
            ScriptType::Function => "function",
            ScriptType::Array { .. } => "array",
            ScriptType::Map { .. } => "map",
            ScriptType::Object { .. } => "object",
        }
    }

    #[test]
    fn resolve_visible_module_symbols_builds_function_signatures() {
        assert_eq!(script_type_kind(&ScriptType::Script), "script");
        assert_eq!(script_type_kind(&ScriptType::Function), "function");
        assert_eq!(
            script_type_kind(&ScriptType::Enum {
                type_name: "Status".to_string(),
                members: vec![],
            }),
            "enum"
        );
        let span = SourceSpan::synthetic();
        let module = ModuleDeclarations {
            type_decls: vec![ParsedTypeDecl {
                name: "Obj".to_string(),
                qualified_name: "shared.Obj".to_string(),
                access: AccessLevel::Public,
                fields: vec![ParsedTypeFieldDecl {
                    name: "value".to_string(),
                    type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                    location: span.clone(),
                }],
                enum_members: Vec::new(),
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
                return_decl: ParsedFunctionReturnDecl {
                    type_expr: ParsedTypeExpr::Custom("Obj".to_string()),
                    location: span.clone(),
                },
                code: "dst = @next; ret = #{value: seed};".to_string(),
                location: span.clone(),
            }],
            module_global_var_decls: Vec::new(),
            module_global_const_decls: Vec::new(),
        };

        let reachable = BTreeSet::from(["shared.xml".to_string()]);
        let module_by_path = BTreeMap::from([("shared.xml".to_string(), module)]);

        let (types, functions, module_vars, _module_consts) =
            resolve_visible_module_symbols(&reachable, &module_by_path, Some("shared"))
                .expect("module should resolve");
        assert!(types.contains_key("Obj"));
        let function = functions.get("make").expect("function should exist");
        assert_eq!(function.params.len(), 1);
        assert!(module_vars.is_empty());
        assert!(function.code.contains("@shared.next"));
        assert_eq!(script_type_kind(&function.return_binding.r#type), "object");
    }

    #[test]
    fn resolve_visible_module_symbols_validates_enum_literals_in_function_code() {
        // Test line 529: rewrite_and_validate_enum_literals_in_expression in function code
        let span = SourceSpan::synthetic();
        let module = ModuleDeclarations {
            type_decls: vec![ParsedTypeDecl {
                name: "Status".to_string(),
                qualified_name: "main.Status".to_string(),
                access: AccessLevel::Public,
                fields: Vec::new(),
                enum_members: vec!["Active".to_string(), "Inactive".to_string()],
                location: span.clone(),
            }],
            function_decls: vec![ParsedFunctionDecl {
                name: "test".to_string(),
                qualified_name: "main.test".to_string(),
                access: AccessLevel::Public,
                params: vec![],
                return_decl: ParsedFunctionReturnDecl {
                    type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                    location: span.clone(),
                },
                code: "ret = Status.Unknown;".to_string(),
                location: span.clone(),
            }],
            module_global_var_decls: Vec::new(),
            module_global_const_decls: Vec::new(),
        };
        let reachable = BTreeSet::from(["main.xml".to_string()]);
        let module_by_path = BTreeMap::from([("main.xml".to_string(), module)]);

        // This should return error because Status.Unknown is not a valid enum member
        let error = resolve_visible_module_symbols(&reachable, &module_by_path, Some("main"))
            .expect_err("function with invalid enum literal should fail");
        assert_eq!(error.code, "ENUM_LITERAL_MEMBER_UNKNOWN");
    }

    #[test]
    fn resolve_visible_module_symbols_validates_enum_in_module_vars() {
        // Test lines 902, 999: normalize_module_initializer in resolve_visible_module_symbols
        // for module variables and constants with invalid enum literal initializers
        let span = SourceSpan::synthetic();
        let module = ModuleDeclarations {
            type_decls: vec![ParsedTypeDecl {
                name: "Status".to_string(),
                qualified_name: "main.Status".to_string(),
                access: AccessLevel::Public,
                fields: Vec::new(),
                enum_members: vec!["Active".to_string(), "Inactive".to_string()],
                location: span.clone(),
            }],
            function_decls: Vec::new(),
            module_global_var_decls: vec![ParsedModuleVarDecl {
                name: "status".to_string(),
                qualified_name: "main.status".to_string(),
                namespace: "main".to_string(),
                access: AccessLevel::Public,
                type_expr: ParsedTypeExpr::Custom("Status".to_string()),
                initial_value_expr: Some("Status.Unknown".to_string()),
                location: span.clone(),
            }],
            module_global_const_decls: Vec::new(),
        };
        let reachable = BTreeSet::from(["main.xml".to_string()]);
        let module_by_path = BTreeMap::from([("main.xml".to_string(), module)]);

        // This should return error because Status.Unknown is not a valid enum member
        let error = resolve_visible_module_symbols(&reachable, &module_by_path, Some("main"))
            .expect_err("module var with invalid enum literal should fail");
        assert_eq!(error.code, "ENUM_LITERAL_MEMBER_UNKNOWN");
    }

    #[test]
    fn resolve_visible_module_symbols_validates_enum_in_module_consts() {
        // Test lines 902, 999: normalize_module_initializer in resolve_visible_module_symbols
        // for module constants with invalid enum literal initializers
        let span = SourceSpan::synthetic();
        let module = ModuleDeclarations {
            type_decls: vec![ParsedTypeDecl {
                name: "Status".to_string(),
                qualified_name: "main.Status".to_string(),
                access: AccessLevel::Public,
                fields: Vec::new(),
                enum_members: vec!["Active".to_string(), "Inactive".to_string()],
                location: span.clone(),
            }],
            function_decls: Vec::new(),
            module_global_var_decls: Vec::new(),
            module_global_const_decls: vec![ParsedModuleConstDecl {
                name: "status".to_string(),
                qualified_name: "main.status".to_string(),
                namespace: "main".to_string(),
                access: AccessLevel::Public,
                type_expr: ParsedTypeExpr::Custom("Status".to_string()),
                initial_value_expr: Some("Status.Unknown".to_string()),
                location: span.clone(),
            }],
        };
        let reachable = BTreeSet::from(["main.xml".to_string()]);
        let module_by_path = BTreeMap::from([("main.xml".to_string(), module)]);

        // This should return error because Status.Unknown is not a valid enum member
        let error = resolve_visible_module_symbols(&reachable, &module_by_path, Some("main"))
            .expect_err("module const with invalid enum literal should fail");
        assert_eq!(error.code, "ENUM_LITERAL_MEMBER_UNKNOWN");
    }

    #[test]
    fn resolve_visible_module_symbols_handles_namespace_collisions_and_alias_edges() {
        let span = SourceSpan::synthetic();

        let duplicate_qualified = ModuleDeclarations {
            type_decls: vec![ParsedTypeDecl {
                name: "T".to_string(),
                qualified_name: "shared.T".to_string(),
                access: AccessLevel::Public,
                fields: vec![ParsedTypeFieldDecl {
                    name: "v".to_string(),
                    type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                    location: span.clone(),
                }],
                enum_members: Vec::new(),
                location: span.clone(),
            }],
            function_decls: Vec::new(),
            module_global_var_decls: Vec::new(),
            module_global_const_decls: Vec::new(),
        };
        let duplicate_module_by_path = BTreeMap::from([
            ("a.xml".to_string(), duplicate_qualified.clone()),
            ("b.xml".to_string(), duplicate_qualified),
        ]);
        let duplicate_reachable = BTreeSet::from(["a.xml".to_string(), "b.xml".to_string()]);
        let duplicate_error = resolve_visible_module_symbols(
            &duplicate_reachable,
            &duplicate_module_by_path,
            Some("shared"),
        )
        .expect_err("duplicate qualified type should fail");
        assert_eq!(duplicate_error.code, "TYPE_DECL_DUPLICATE");

        let module_by_path = BTreeMap::from([
            (
                "a.xml".to_string(),
                ModuleDeclarations {
                    type_decls: Vec::new(),
                    function_decls: vec![ParsedFunctionDecl {
                        name: "doit".to_string(),
                        qualified_name: "a.doit".to_string(),
                        access: AccessLevel::Public,
                        params: Vec::new(),
                        return_decl: ParsedFunctionReturnDecl {
                            type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                            location: span.clone(),
                        },
                        code: "out = 1;".to_string(),
                        location: span.clone(),
                    }],
                    module_global_var_decls: Vec::new(),
                    module_global_const_decls: Vec::new(),
                },
            ),
            (
                "b.xml".to_string(),
                ModuleDeclarations {
                    type_decls: Vec::new(),
                    function_decls: vec![ParsedFunctionDecl {
                        name: "doit".to_string(),
                        qualified_name: "b.doit".to_string(),
                        access: AccessLevel::Public,
                        params: Vec::new(),
                        return_decl: ParsedFunctionReturnDecl {
                            type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                            location: span.clone(),
                        },
                        code: "out = 2;".to_string(),
                        location: span.clone(),
                    }],
                    module_global_var_decls: Vec::new(),
                    module_global_const_decls: Vec::new(),
                },
            ),
        ]);
        let reachable = BTreeSet::from(["a.xml".to_string(), "b.xml".to_string()]);
        let (_types, functions, module_vars, _module_consts) =
            resolve_visible_module_symbols(&reachable, &module_by_path, Some("a"))
                .expect("module should resolve");
        assert!(functions.contains_key("a.doit"));
        assert!(functions.contains_key("b.doit"));
        assert!(functions.contains_key("doit"));
        assert_eq!(
            functions.get("doit").expect("local short alias").name,
            "doit"
        );
        assert!(module_vars.is_empty());
    }

    #[test]
    fn resolve_visible_module_symbols_rejects_duplicate_function_names() {
        let span = SourceSpan::synthetic();
        // Two files with the same function qualified name
        let duplicate_func = ModuleDeclarations {
            type_decls: Vec::new(),
            function_decls: vec![ParsedFunctionDecl {
                name: "foo".to_string(),
                qualified_name: "shared.foo".to_string(),
                access: AccessLevel::Public,
                params: vec![],
                return_decl: ParsedFunctionReturnDecl {
                    type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                    location: span.clone(),
                },
                code: "out = 1;".to_string(),
                location: span.clone(),
            }],
            module_global_var_decls: Vec::new(),
            module_global_const_decls: Vec::new(),
        };
        let duplicate_module_by_path = BTreeMap::from([
            ("a.xml".to_string(), duplicate_func.clone()),
            ("b.xml".to_string(), duplicate_func),
        ]);
        let reachable = BTreeSet::from(["a.xml".to_string(), "b.xml".to_string()]);
        let error =
            resolve_visible_module_symbols(&reachable, &duplicate_module_by_path, Some("shared"))
                .expect_err("duplicate function should fail");
        assert_eq!(error.code, "FUNCTION_DECL_DUPLICATE");
    }

    #[test]
    fn resolve_visible_module_symbols_rejects_unknown_param_type() {
        let span = SourceSpan::synthetic();
        // Function with param type that doesn't exist
        let unknown_param_type = ModuleDeclarations {
            type_decls: Vec::new(),
            function_decls: vec![ParsedFunctionDecl {
                name: "foo".to_string(),
                qualified_name: "shared.foo".to_string(),
                access: AccessLevel::Public,
                params: vec![ParsedFunctionParamDecl {
                    name: "x".to_string(),
                    type_expr: ParsedTypeExpr::Custom("UnknownType".to_string()),
                    location: span.clone(),
                }],
                return_decl: ParsedFunctionReturnDecl {
                    type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                    location: span.clone(),
                },
                code: "out = 1;".to_string(),
                location: span.clone(),
            }],
            module_global_var_decls: Vec::new(),
            module_global_const_decls: Vec::new(),
        };
        let module_by_path = BTreeMap::from([("a.xml".to_string(), unknown_param_type)]);
        let reachable = BTreeSet::from(["a.xml".to_string()]);
        let error = resolve_visible_module_symbols(&reachable, &module_by_path, Some("shared"))
            .expect_err("unknown param type should fail");
        assert_eq!(error.code, "TYPE_UNKNOWN");
    }

    #[test]
    fn resolve_visible_module_symbols_rejects_unknown_return_type() {
        let span = SourceSpan::synthetic();
        // Function with return type that doesn't exist
        let unknown_return_type = ModuleDeclarations {
            type_decls: Vec::new(),
            function_decls: vec![ParsedFunctionDecl {
                name: "foo".to_string(),
                qualified_name: "shared.foo".to_string(),
                access: AccessLevel::Public,
                params: vec![],
                return_decl: ParsedFunctionReturnDecl {
                    type_expr: ParsedTypeExpr::Custom("NonExistentType".to_string()),
                    location: span.clone(),
                },
                code: "out = 1;".to_string(),
                location: span.clone(),
            }],
            module_global_var_decls: Vec::new(),
            module_global_const_decls: Vec::new(),
        };
        let module_by_path = BTreeMap::from([("a.xml".to_string(), unknown_return_type)]);
        let reachable = BTreeSet::from(["a.xml".to_string()]);
        let error = resolve_visible_module_symbols(&reachable, &module_by_path, Some("shared"))
            .expect_err("unknown return type should fail");
        assert_eq!(error.code, "TYPE_UNKNOWN");
    }

    #[test]
    fn resolve_visible_module_symbols_applies_module_global_short_alias_rules() {
        let span = SourceSpan::synthetic();
        let make_decl = |namespace: &str, name: &str| ParsedModuleVarDecl {
            namespace: namespace.to_string(),
            name: name.to_string(),
            qualified_name: format!("{}.{}", namespace, name),
            access: AccessLevel::Public,
            type_expr: ParsedTypeExpr::Primitive("int".to_string()),
            initial_value_expr: None,
            location: span.clone(),
        };

        let unique_modules = BTreeMap::from([(
            "a.xml".to_string(),
            ModuleDeclarations {
                type_decls: Vec::new(),
                function_decls: Vec::new(),
                module_global_var_decls: vec![make_decl("a", "hp")],
                module_global_const_decls: Vec::new(),
            },
        )]);
        let unique_reachable = BTreeSet::from(["a.xml".to_string()]);
        let (_types, _functions, unique_globals, _module_consts) =
            resolve_visible_module_symbols(&unique_reachable, &unique_modules, Some("a"))
                .expect("module should resolve");
        assert!(unique_globals.contains_key("a.hp"));
        assert!(unique_globals.contains_key("hp"));
        assert_eq!(
            unique_globals
                .get("hp")
                .expect("short alias should exist")
                .qualified_name,
            "a.hp"
        );

        let collision_module = BTreeMap::from([
            (
                "a.xml".to_string(),
                ModuleDeclarations {
                    type_decls: Vec::new(),
                    function_decls: Vec::new(),
                    module_global_var_decls: vec![make_decl("a", "hp")],
                    module_global_const_decls: Vec::new(),
                },
            ),
            (
                "b.xml".to_string(),
                ModuleDeclarations {
                    type_decls: Vec::new(),
                    function_decls: Vec::new(),
                    module_global_var_decls: vec![make_decl("b", "hp")],
                    module_global_const_decls: Vec::new(),
                },
            ),
        ]);
        let collision_reachable = BTreeSet::from(["a.xml".to_string(), "b.xml".to_string()]);
        let (_types, _functions, collision_globals, _module_consts) =
            resolve_visible_module_symbols(&collision_reachable, &collision_module, Some("a"))
                .expect("module should resolve");
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
    fn resolve_visible_module_symbols_with_aliases_supports_and_validates_alias_rules() {
        let span = SourceSpan::synthetic();
        let modules = BTreeMap::from([(
            "shared.xml".to_string(),
            ModuleDeclarations {
                type_decls: vec![ParsedTypeDecl {
                    name: "Unit".to_string(),
                    qualified_name: "shared.Unit".to_string(),
                    access: AccessLevel::Public,
                    fields: vec![ParsedTypeFieldDecl {
                        name: "hp".to_string(),
                        type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                        location: span.clone(),
                    }],
                    enum_members: Vec::new(),
                    location: span.clone(),
                }],
                function_decls: vec![ParsedFunctionDecl {
                    name: "boost".to_string(),
                    qualified_name: "shared.boost".to_string(),
                    access: AccessLevel::Public,
                    params: vec![ParsedFunctionParamDecl {
                        name: "x".to_string(),
                        type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                        location: span.clone(),
                    }],
                    return_decl: ParsedFunctionReturnDecl {
                        type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                        location: span.clone(),
                    },
                    code: "ret = x + 1;".to_string(),
                    location: span.clone(),
                }],
                module_global_var_decls: vec![
                    ParsedModuleVarDecl {
                        namespace: "shared".to_string(),
                        name: "hp".to_string(),
                        qualified_name: "shared.hp".to_string(),
                        access: AccessLevel::Public,
                        type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                        initial_value_expr: Some("1".to_string()),
                        location: span.clone(),
                    },
                    ParsedModuleVarDecl {
                        namespace: "shared".to_string(),
                        name: "mp".to_string(),
                        qualified_name: "shared.mp".to_string(),
                        access: AccessLevel::Public,
                        type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                        initial_value_expr: Some("2".to_string()),
                        location: span.clone(),
                    },
                ],
                module_global_const_decls: vec![ParsedModuleConstDecl {
                    namespace: "shared".to_string(),
                    name: "BASE".to_string(),
                    qualified_name: "shared.BASE".to_string(),
                    access: AccessLevel::Public,
                    type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                    initial_value_expr: Some("10".to_string()),
                    location: span.clone(),
                }],
            },
        )]);
        let reachable = BTreeSet::from(["shared.xml".to_string()]);

        let aliases = vec![
            AliasDirective {
                target_qualified_name: "shared.Unit".to_string(),
                alias_name: "Hero".to_string(),
            },
            AliasDirective {
                target_qualified_name: "shared.hp".to_string(),
                alias_name: "health".to_string(),
            },
            AliasDirective {
                target_qualified_name: "shared.BASE".to_string(),
                alias_name: "base".to_string(),
            },
            AliasDirective {
                target_qualified_name: "shared.hp".to_string(),
                alias_name: "health".to_string(),
            },
        ];
        let (types, _functions, module_vars, module_consts) =
            resolve_visible_module_symbols_with_aliases(
                &reachable,
                &modules,
                Some("main"),
                &aliases,
            )
            .expect("aliases should resolve");
        assert!(types.contains_key("Hero"));
        assert_eq!(
            module_vars
                .get("health")
                .expect("module var alias should exist")
                .qualified_name,
            "shared.hp"
        );
        assert_eq!(
            module_consts
                .get("base")
                .expect("module const alias should exist")
                .qualified_name,
            "shared.BASE"
        );

        let not_found = resolve_visible_module_symbols_with_aliases(
            &reachable,
            &modules,
            Some("main"),
            &[AliasDirective {
                target_qualified_name: "shared.missing".to_string(),
                alias_name: "x".to_string(),
            }],
        )
        .expect_err("missing alias target should fail");
        assert_eq!(not_found.code, "ALIAS_TARGET_NOT_FOUND");

        let function_target = resolve_visible_module_symbols_with_aliases(
            &reachable,
            &modules,
            Some("main"),
            &[AliasDirective {
                target_qualified_name: "shared.boost".to_string(),
                alias_name: "boost2".to_string(),
            }],
        )
        .expect_err("function target should fail");
        assert_eq!(function_target.code, "ALIAS_TARGET_KIND_UNSUPPORTED");

        let name_conflict = resolve_visible_module_symbols_with_aliases(
            &reachable,
            &modules,
            Some("shared"),
            &[AliasDirective {
                target_qualified_name: "shared.hp".to_string(),
                alias_name: "hp".to_string(),
            }],
        )
        .expect_err("alias name collision should fail");
        assert_eq!(name_conflict.code, "ALIAS_NAME_CONFLICT");

        let divergent_rebind = resolve_visible_module_symbols_with_aliases(
            &reachable,
            &modules,
            Some("main"),
            &[
                AliasDirective {
                    target_qualified_name: "shared.hp".to_string(),
                    alias_name: "stat".to_string(),
                },
                AliasDirective {
                    target_qualified_name: "shared.mp".to_string(),
                    alias_name: "stat".to_string(),
                },
            ],
        )
        .expect_err("same alias to different targets should fail");
        assert_eq!(divergent_rebind.code, "ALIAS_NAME_CONFLICT");

        // Test lines 1245-1249: type alias conflict in collect_explicit_visible_type_aliases
        // Same alias pointing to different type targets should fail
        let modules_with_multiple_types = BTreeMap::from([(
            "shared.xml".to_string(),
            ModuleDeclarations {
                type_decls: vec![
                    ParsedTypeDecl {
                        name: "Unit".to_string(),
                        qualified_name: "shared.Unit".to_string(),
                        access: AccessLevel::Public,
                        fields: vec![],
                        enum_members: Vec::new(),
                        location: span.clone(),
                    },
                    ParsedTypeDecl {
                        name: "OtherUnit".to_string(),
                        qualified_name: "shared.OtherUnit".to_string(),
                        access: AccessLevel::Public,
                        fields: vec![],
                        enum_members: Vec::new(),
                        location: span.clone(),
                    },
                ],
                function_decls: vec![],
                module_global_var_decls: vec![],
                module_global_const_decls: vec![],
            },
        )]);
        let reachable_with_types = BTreeSet::from(["shared.xml".to_string()]);
        let type_alias_conflict = resolve_visible_module_symbols_with_aliases(
            &reachable_with_types,
            &modules_with_multiple_types,
            Some("main"),
            &[
                AliasDirective {
                    target_qualified_name: "shared.Unit".to_string(),
                    alias_name: "Hero".to_string(),
                },
                AliasDirective {
                    target_qualified_name: "shared.OtherUnit".to_string(),
                    alias_name: "Hero".to_string(),
                },
            ],
        )
        .expect_err("same alias to different type targets should fail");
        assert_eq!(type_alias_conflict.code, "ALIAS_NAME_CONFLICT");

        // Test line 1247: same alias pointing to same target should be skipped (continue)
        // This tests the case where duplicate directives with identical alias and target are ignored
        let modules_dup = BTreeMap::from([(
            "shared.xml".to_string(),
            ModuleDeclarations {
                type_decls: vec![ParsedTypeDecl {
                    name: "Unit".to_string(),
                    qualified_name: "shared.Unit".to_string(),
                    access: AccessLevel::Public,
                    fields: vec![],
                    enum_members: Vec::new(),
                    location: span.clone(),
                }],
                function_decls: vec![],
                module_global_var_decls: vec![],
                module_global_const_decls: vec![],
            },
        )]);
        let reachable_dup = BTreeSet::from(["shared.xml".to_string()]);
        // Same alias pointing to same target twice should be allowed (skipped via continue at line 1247)
        let duplicate_alias = resolve_visible_module_symbols_with_aliases(
            &reachable_dup,
            &modules_dup,
            Some("main"),
            &[
                AliasDirective {
                    target_qualified_name: "shared.Unit".to_string(),
                    alias_name: "Hero".to_string(),
                },
                AliasDirective {
                    target_qualified_name: "shared.Unit".to_string(),
                    alias_name: "Hero".to_string(),
                },
            ],
        )
        .expect("duplicate alias to same target should succeed");
        // Only one alias should exist
        assert!(
            duplicate_alias.0.contains_key("Hero"),
            "Hero alias should exist"
        );
    }

    #[test]
    fn resolve_visible_module_symbols_with_aliases_resolves_local_type_positions() {
        let span = SourceSpan::synthetic();
        let modules = BTreeMap::from([
            (
                "ids.xml".to_string(),
                ModuleDeclarations {
                    type_decls: vec![
                        ParsedTypeDecl {
                            name: "LocationId".to_string(),
                            qualified_name: "ids.LocationId".to_string(),
                            access: AccessLevel::Public,
                            fields: Vec::new(),
                            enum_members: vec!["Home".to_string()],
                            location: span.clone(),
                        },
                        ParsedTypeDecl {
                            name: "MessageKey".to_string(),
                            qualified_name: "ids.MessageKey".to_string(),
                            access: AccessLevel::Public,
                            fields: Vec::new(),
                            enum_members: vec!["Ping".to_string()],
                            location: span.clone(),
                        },
                    ],
                    function_decls: Vec::new(),
                    module_global_var_decls: Vec::new(),
                    module_global_const_decls: Vec::new(),
                },
            ),
            (
                "main.xml".to_string(),
                ModuleDeclarations {
                    type_decls: vec![ParsedTypeDecl {
                        name: "Pair".to_string(),
                        qualified_name: "main.Pair".to_string(),
                        access: AccessLevel::Public,
                        fields: vec![
                            ParsedTypeFieldDecl {
                                name: "loc".to_string(),
                                type_expr: ParsedTypeExpr::Custom("LocationId".to_string()),
                                location: span.clone(),
                            },
                            ParsedTypeFieldDecl {
                                name: "msg".to_string(),
                                type_expr: ParsedTypeExpr::Custom("MessageKey".to_string()),
                                location: span.clone(),
                            },
                        ],
                        enum_members: Vec::new(),
                        location: span.clone(),
                    }],
                    function_decls: vec![ParsedFunctionDecl {
                        name: "check".to_string(),
                        qualified_name: "main.check".to_string(),
                        access: AccessLevel::Public,
                        params: vec![
                            ParsedFunctionParamDecl {
                                name: "message_key".to_string(),
                                type_expr: ParsedTypeExpr::Custom("MessageKey".to_string()),
                                location: span.clone(),
                            },
                            ParsedFunctionParamDecl {
                                name: "location_id".to_string(),
                                type_expr: ParsedTypeExpr::Custom("LocationId".to_string()),
                                location: span.clone(),
                            },
                        ],
                        return_decl: ParsedFunctionReturnDecl {
                            type_expr: ParsedTypeExpr::Primitive("boolean".to_string()),
                            location: span.clone(),
                        },
                        code: "ret = message_key == MessageKey.Ping && location_id == LocationId.Home;".to_string(),
                        location: span.clone(),
                    }],
                    module_global_var_decls: Vec::new(),
                    module_global_const_decls: Vec::new(),
                },
            ),
        ]);
        let reachable = BTreeSet::from(["ids.xml".to_string(), "main.xml".to_string()]);
        let aliases = vec![
            AliasDirective {
                target_qualified_name: "ids.LocationId".to_string(),
                alias_name: "LocationId".to_string(),
            },
            AliasDirective {
                target_qualified_name: "ids.MessageKey".to_string(),
                alias_name: "MessageKey".to_string(),
            },
        ];

        let (types, functions, _module_vars, _module_consts) =
            resolve_visible_module_symbols_with_aliases(
                &reachable,
                &modules,
                Some("main"),
                &aliases,
            )
            .expect("type aliases should resolve in local type and function signatures");
        assert!(types.contains_key("main.Pair"));
        assert!(types.contains_key("LocationId"));
        assert!(types.contains_key("MessageKey"));
        assert!(functions.contains_key("main.check"));
    }

    #[test]
    fn compile_bundle_rejects_module_global_forward_reference() {
        let files = map(&[
            (
                "shared.xml",
                r#"
<module name="shared" export="var:a,b">
  <var name="a" type="int">b + 1</var>
  <var name="b" type="int">1</var>
</module>
"#,
            ),
            (
                "main.xml",
                r#"
<!-- import shared from shared.xml -->
<module name="main" export="script:main">
<script name="main"><text>ok</text></script>
</module>
"#,
            ),
        ]);

        let error =
            compile_project_bundle_from_xml_map(&files).expect_err("forward reference should fail");
        assert_eq!(error.code, "MODULE_GLOBAL_INIT_ORDER");
    }

    #[test]
    fn compile_bundle_allows_module_global_reference_to_initialized_symbol() {
        let files = map(&[
            (
                "shared.xml",
                r#"
<module name="shared" export="var:b,a">
  <var name="b" type="int">1</var>
  <var name="a" type="int">b + 1</var>
</module>
"#,
            ),
            (
                "main.xml",
                r#"
<!-- import shared from shared.xml -->
<module name="main" export="script:main">
<script name="main"><text>ok</text></script>
</module>
"#,
            ),
        ]);

        let bundle =
            compile_project_bundle_from_xml_map(&files).expect("back reference should pass");
        assert!(bundle.module_var_declarations.contains_key("shared.a"));
        assert!(bundle.module_var_declarations.contains_key("shared.b"));
    }

    #[test]
    fn parse_module_global_var_rejects_child_elements() {
        let files_with_child = map(&[
            (
                "shared.xml",
                r#"<module name="shared" export="var:hp"><var name="hp" type="int"><text>1</text></var></module>"#,
            ),
            (
                "main.xml",
                r#"
<!-- import shared from shared.xml -->
<module name="main" export="script:main">
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
    fn parse_module_global_const_rejects_missing_name_or_type() {
        // Missing name attribute
        let files_missing_name = map(&[
            (
                "shared.xml",
                r#"<module name="shared"><const type="int">1</const></module>"#,
            ),
            (
                "main.xml",
                r#"
<!-- import shared from shared.xml -->
<module name="main" export="script:main">
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
                r#"<module name="shared" export="const:base"><const name="base">1</const></module>"#,
            ),
            (
                "main.xml",
                r#"
<!-- import shared from shared.xml -->
<module name="main" export="script:main">
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
    fn parse_module_const_rejects_invalid_type() {
        let files = map(&[
            (
                "shared.xml",
                r#"<module name="shared" export="const:base"><const name="base" type="UnknownType">1</const></module>"#,
            ),
            (
                "main.xml",
                r#"
<!-- import shared from shared.xml -->
<module name="main" export="script:main">
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
    fn resolve_visible_module_symbols_rejects_const_with_unresolved_type() {
        let span = SourceSpan::synthetic();
        let module_with_bad_const = BTreeMap::from([(
            "shared.xml".to_string(),
            ModuleDeclarations {
                type_decls: Vec::new(),
                function_decls: Vec::new(),
                module_global_var_decls: Vec::new(),
                module_global_const_decls: vec![ParsedModuleConstDecl {
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
        let error =
            resolve_visible_module_symbols(&reachable, &module_with_bad_const, Some("shared"))
                .expect_err("unresolved type should fail");
        assert_eq!(error.code, "TYPE_UNKNOWN");
    }

    #[test]
    fn module_global_resolution_rejects_duplicates_and_allows_empty_initializer() {
        let duplicate_types_bundle = map(&[
            (
                "a.xml",
                r#"<module name="shared" export="type:T"><type name="T"><field name="v" type="int"/></type></module>"#,
            ),
            (
                "b.xml",
                r#"<module name="shared" export="type:T"><type name="T"><field name="v" type="int"/></type></module>"#,
            ),
        ]);
        let duplicate_types_error = compile_project_bundle_from_xml_map(&duplicate_types_bundle)
            .expect_err("bundle duplicate type should fail");
        assert_eq!(duplicate_types_error.code, "TYPE_DECL_DUPLICATE");

        let duplicate_globals_bundle = map(&[
            (
                "a.xml",
                r#"<module name="shared" export="var:hp"><var name="hp" type="int">1</var></module>"#,
            ),
            (
                "b.xml",
                r#"<module name="shared" export="var:hp"><var name="hp" type="int">2</var></module>"#,
            ),
        ]);
        let duplicate_globals_error =
            compile_project_bundle_from_xml_map(&duplicate_globals_bundle)
                .expect_err("bundle duplicate module global should fail");
        assert_eq!(duplicate_globals_error.code, "MODULE_GLOBAL_VAR_DUPLICATE");

        let empty_initializer = map(&[
            (
                "shared.xml",
                r#"<module name="shared" export="var:hp"><var name="hp" type="int"/></module>"#,
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
        let bundle = compile_project_bundle_from_xml_map(&empty_initializer).expect("compile");
        let decl = bundle
            .module_var_declarations
            .get("shared.hp")
            .expect("decl should exist");
        assert!(decl.initial_value_expr.is_none());
    }

    #[test]
    fn resolve_visible_module_symbols_rejects_duplicate_module_global_in_closure() {
        let span = SourceSpan::synthetic();
        let duplicate = ParsedModuleVarDecl {
            namespace: "shared".to_string(),
            name: "hp".to_string(),
            qualified_name: "shared.hp".to_string(),
            access: AccessLevel::Public,
            type_expr: ParsedTypeExpr::Primitive("int".to_string()),
            initial_value_expr: Some("1".to_string()),
            location: span.clone(),
        };
        let module_by_path = BTreeMap::from([
            (
                "a.xml".to_string(),
                ModuleDeclarations {
                    type_decls: Vec::new(),
                    function_decls: Vec::new(),
                    module_global_var_decls: vec![duplicate.clone()],
                    module_global_const_decls: Vec::new(),
                },
            ),
            (
                "b.xml".to_string(),
                ModuleDeclarations {
                    type_decls: Vec::new(),
                    function_decls: Vec::new(),
                    module_global_var_decls: vec![duplicate],
                    module_global_const_decls: Vec::new(),
                },
            ),
        ]);
        let reachable = BTreeSet::from(["a.xml".to_string(), "b.xml".to_string()]);
        let error = resolve_visible_module_symbols(&reachable, &module_by_path, Some("a"))
            .expect_err("duplicate module global should fail");
        assert_eq!(error.code, "MODULE_GLOBAL_VAR_DUPLICATE");
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
    fn module_and_type_resolution_helpers_cover_duplicate_and_recursive_errors() {
        let bad_module = BTreeMap::from([(
            "x.xml".to_string(),
            "<script name=\"x\"></script>".to_string(),
        )]);
        let error = compile_project_bundle_from_xml_map(&bad_module).expect_err("bad module root");
        assert_eq!(error.code, "XML_ROOT_INVALID");

        let duplicate_types = map(&[
            (
                "a.xml",
                r#"<module name="a" export="type:T"><type name="T"><field name="v" type="int"/></type></module>"#,
            ),
            (
                "b.xml",
                r#"<module name="b" export="type:T"><type name="T"><field name="v" type="int"/></type></module>"#,
            ),
            (
                "main.xml",
                r#"
    <!-- import a from a.xml -->
    <!-- import b from b.xml -->
    <module name="main" export="script:main">
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
                r#"<module name="x" export="type:A,B"><type name="A"><field name="b" type="B"/></type><type name="B"><field name="a" type="A"/></type></module>"#,
            ),
            (
                "main.xml",
                r#"
    <!-- import x from x.xml -->
    <module name="main" export="script:main">
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
    fn module_function_parsing_and_resolution_is_covered() {
        // Test module function parsing (covers line 40)
        let files = map(&[
            (
                "shared.xml",
                r#"<module name="shared" export="function:add">
  <function name="add" args="int:a,int:b" return_type="int">
    return a + b;
  </function>
</module>"#,
            ),
            (
                "main.xml",
                r#"
<!-- import shared from shared.xml -->
<module name="main" export="script:main">
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
    fn parse_module_files_and_type_resolution_success_paths_are_covered() {
        let files = map(&[(
            "shared.xml",
            r#"<module name="shared" export="function:make;type:Obj">
  <type name="Obj"><field name="value" type="int"/></type>
  <function name="make" args="int:seed" return_type="Obj">
    return #{ value: seed };
  </function>
</module>"#,
        )]);
        let sources = parse_sources(&files).expect("parse sources");
        let module_by_path = parse_module_files(&sources).expect("parse module");
        let reachable = BTreeSet::from(["shared.xml".to_string()]);
        let (types, functions, _, _module_consts) =
            resolve_visible_module_symbols(&reachable, &module_by_path, Some("shared"))
                .expect("resolve module");
        assert!(types.contains_key("shared.Obj"));
        assert!(functions.contains_key("shared.make"));
    }

    #[test]
    fn parse_module_files_attaches_file_path_for_module_errors() {
        let files = map(&[(
            "bad.xml",
            r#"<module name="shared">
  <oops/>
</module>"#,
        )]);
        let sources = parse_sources(&files).expect("parse sources");
        let error = parse_module_files(&sources).expect_err("module parse should fail");
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
    fn parse_module_files_wraps_attr_reserved_and_function_parse_errors_with_file_context() {
        let missing_name_error = parse_sources(&BTreeMap::from([(
            "missing-name.xml".to_string(),
            "<module></module>".to_string(),
        )]))
        .expect_err("missing name should fail during source parsing");
        assert_eq!(missing_name_error.code, "XML_MODULE_NAME_MISSING");
        assert!(missing_name_error
            .message
            .contains("In file \"missing-name.xml\":"));

        let reserved_name = map(&[("reserved.xml", r#"<module name="__sl_bad"></module>"#)]);
        let reserved_name_sources = parse_sources(&reserved_name).expect("parse sources");
        let reserved_name_error =
            parse_module_files(&reserved_name_sources).expect_err("reserved name should fail");
        assert!(reserved_name_error
            .message
            .contains("In file \"reserved.xml\":"));

        let bad_function = map(&[(
            "bad-function.xml",
            r#"<module name="shared" export="function:bad">
  <function name="bad" args="int:a" return="int:r">
    r = a + 1;
  </function>
</module>"#,
        )]);
        let bad_function_sources = parse_sources(&bad_function).expect("parse sources");
        let bad_function_error =
            parse_module_files(&bad_function_sources).expect_err("bad function should fail");
        assert!(bad_function_error
            .message
            .contains("In file \"bad-function.xml\":"));

        let keyword_script = map(&[(
            "keyword-script.xml",
            r#"<module name="battle" export="script:shared">
  <script name="shared"/>
</module>"#,
        )]);
        let keyword_script_sources = parse_sources(&keyword_script).expect("parse sources");
        let keyword_script_error = parse_module_files(&keyword_script_sources)
            .expect_err("keyword script name should fail");
        assert_eq!(keyword_script_error.code, "NAME_RHAI_KEYWORD_RESERVED");
        assert!(keyword_script_error
            .message
            .contains("In file \"keyword-script.xml\":"));
    }

    #[test]
    fn parse_module_files_wraps_enum_parse_errors_with_file_context() {
        // Test line 151: enum parse error is wrapped with file context
        let duplicate_enum_member = BTreeMap::from([(
            "bad-enum.xml".to_string(),
            r#"<module name="bad" export="enum:Status">
<enum name="Status">
  <member name="Active"/>
  <member name="Active"/>
</enum>
</module>"#
                .to_string(),
        )]);
        let sources = parse_sources(&duplicate_enum_member).expect("parse sources");
        let error = parse_module_files(&sources).expect_err("duplicate enum member should fail");
        assert_eq!(error.code, "ENUM_MEMBER_DUPLICATE");
        assert!(error.message.contains("In file \"bad-enum.xml\":"));
    }

    #[test]
    fn parse_module_var_declaration_covers_success_and_error_paths() {
        let node = xml_element(
            "var",
            &[("name", "hp"), ("type", "int")],
            vec![xml_text("1")],
        );
        let parsed = parse_module_var_declaration(&node, "shared", AccessLevel::Private)
            .expect("parse module var");
        assert_eq!(parsed.qualified_name, "shared.hp");
        assert_eq!(parsed.initial_value_expr.as_deref(), Some("1"));

        let reserved_name = xml_element(
            "var",
            &[("name", "__sl_hp"), ("type", "int")],
            vec![xml_text("1")],
        );
        let error = parse_module_var_declaration(&reserved_name, "shared", AccessLevel::Private)
            .expect_err("reserved name should fail");
        assert_eq!(error.code, "NAME_RESERVED_PREFIX");

        let keyword_name = xml_element(
            "var",
            &[("name", "shared"), ("type", "int")],
            vec![xml_text("1")],
        );
        let error = parse_module_var_declaration(&keyword_name, "mod", AccessLevel::Private)
            .expect_err("keyword name should fail");
        assert_eq!(error.code, "NAME_RHAI_KEYWORD_RESERVED");

        let invalid_type = xml_element(
            "var",
            &[("name", "hp"), ("type", "#{ }")],
            vec![xml_text("1")],
        );
        let error = parse_module_var_declaration(&invalid_type, "shared", AccessLevel::Private)
            .expect_err("bad type");
        assert_eq!(error.code, "TYPE_PARSE_ERROR");

        let missing_name = xml_element("var", &[("type", "int")], vec![xml_text("1")]);
        let error = parse_module_var_declaration(&missing_name, "shared", AccessLevel::Private)
            .expect_err("name should be required");
        assert_eq!(error.code, "XML_MISSING_ATTR");

        let missing_type = xml_element("var", &[("name", "hp")], vec![xml_text("1")]);
        let error = parse_module_var_declaration(&missing_type, "shared", AccessLevel::Private)
            .expect_err("type should be required");
        assert_eq!(error.code, "XML_MISSING_ATTR");

        // Legacy access attribute is no longer supported.
        let mut invalid_sources = BTreeMap::new();
        invalid_sources.insert(
            "/".to_string(),
            SourceFile {
                kind: SourceKind::Json,
                imports: Vec::new(),
                alias_directives: Vec::new(),
                xml_root: None,
                json_value: Some(SlValue::Number(1.0)),
            },
        );
        let error =
            collect_global_data(&invalid_sources).expect_err("invalid global data symbol path");
        assert_eq!(error.code, "GLOBAL_DATA_SYMBOL_INVALID");

        let reachable = BTreeSet::from(["/".to_string()]);
        let error = collect_visible_global_symbols(&reachable, &invalid_sources)
            .expect_err("invalid visible global data symbol path");
        assert_eq!(error.code, "GLOBAL_DATA_SYMBOL_INVALID");

        let invalid_basename =
            parse_global_data_symbol("bad-name.json").expect_err("invalid json basename");
        assert_eq!(invalid_basename.code, "GLOBAL_DATA_SYMBOL_INVALID");

        let missing_value = collect_global_data(&BTreeMap::from([(
            "game.json".to_string(),
            SourceFile {
                kind: SourceKind::Json,
                imports: Vec::new(),
                alias_directives: Vec::new(),
                xml_root: None,
                json_value: None,
            },
        )]))
        .expect_err("json value should be required");
        assert_eq!(missing_value.code, "GLOBAL_DATA_MISSING_VALUE");

        let reserved_global_symbol =
            parse_global_data_symbol("__hidden.json").expect_err("reserved global data symbol");
        assert_eq!(reserved_global_symbol.code, "NAME_RESERVED_PREFIX");
    }

    #[test]
    fn resolve_visible_module_symbols_error_propagation_and_alias_paths_are_covered() {
        let span = SourceSpan::synthetic();
        let module_with_alias = BTreeMap::from([(
            "one.xml".to_string(),
            ModuleDeclarations {
                type_decls: vec![ParsedTypeDecl {
                    name: "Obj".to_string(),
                    qualified_name: "one.Obj".to_string(),
                    access: AccessLevel::Public,
                    fields: vec![ParsedTypeFieldDecl {
                        name: "v".to_string(),
                        type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                        location: span.clone(),
                    }],
                    enum_members: Vec::new(),
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
                    return_decl: ParsedFunctionReturnDecl {
                        type_expr: ParsedTypeExpr::Custom("Obj".to_string()),
                        location: span.clone(),
                    },
                    code: "ret = #{v: x};".to_string(),
                    location: span.clone(),
                }],
                module_global_var_decls: vec![ParsedModuleVarDecl {
                    namespace: "one".to_string(),
                    name: "hp".to_string(),
                    qualified_name: "one.hp".to_string(),
                    access: AccessLevel::Public,
                    type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                    initial_value_expr: None,
                    location: span.clone(),
                }],
                module_global_const_decls: Vec::new(),
            },
        )]);
        let reachable = BTreeSet::from(["one.xml".to_string()]);
        let (types, functions, module_vars, _module_consts) =
            resolve_visible_module_symbols(&reachable, &module_with_alias, Some("one"))
                .expect("resolve aliases");
        assert!(types.contains_key("Obj"));
        assert!(functions.contains_key("make"));
        assert!(module_vars.contains_key("hp"));
        assert_eq!(
            script_type_kind(
                types
                    .get("Obj")
                    .expect("short type alias should be visible in resolved map")
            ),
            "object"
        );

        let module_for_bundle = BTreeMap::from([(
            "bundle.xml".to_string(),
            ModuleDeclarations {
                type_decls: vec![ParsedTypeDecl {
                    name: "T".to_string(),
                    qualified_name: "bundle.T".to_string(),
                    access: AccessLevel::Public,
                    fields: vec![ParsedTypeFieldDecl {
                        name: "v".to_string(),
                        type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                        location: span.clone(),
                    }],
                    enum_members: Vec::new(),
                    location: span.clone(),
                }],
                function_decls: Vec::new(),
                module_global_var_decls: vec![ParsedModuleVarDecl {
                    namespace: "bundle".to_string(),
                    name: "item".to_string(),
                    qualified_name: "bundle.item".to_string(),
                    access: AccessLevel::Public,
                    type_expr: ParsedTypeExpr::Custom("T".to_string()),
                    initial_value_expr: None,
                    location: span.clone(),
                }],
                module_global_const_decls: Vec::new(),
            },
        )]);
        let (bundle_globals, init_order) =
            collect_module_vars_for_bundle(&module_for_bundle, &BTreeMap::new())
                .expect("bundle alias should resolve");
        assert!(bundle_globals.contains_key("bundle.item"));
        assert_eq!(init_order, vec!["bundle.item".to_string()]);

        let bad_type_decl = BTreeMap::from([(
            "bad_type.xml".to_string(),
            ModuleDeclarations {
                type_decls: vec![ParsedTypeDecl {
                    name: "Broken".to_string(),
                    qualified_name: "bad_type.Broken".to_string(),
                    access: AccessLevel::Public,
                    fields: vec![ParsedTypeFieldDecl {
                        name: "v".to_string(),
                        type_expr: ParsedTypeExpr::Custom("Missing".to_string()),
                        location: span.clone(),
                    }],
                    enum_members: Vec::new(),
                    location: span.clone(),
                }],
                function_decls: Vec::new(),
                module_global_var_decls: Vec::new(),
                module_global_const_decls: Vec::new(),
            },
        )]);
        let reachable = BTreeSet::from(["bad_type.xml".to_string()]);
        let error = resolve_visible_module_symbols(&reachable, &bad_type_decl, Some("bad_type"))
            .expect_err("type resolution in visible loop should fail");
        assert_eq!(error.code, "TYPE_UNKNOWN");

        let alias_already_exists = BTreeMap::from([(
            "alias.xml".to_string(),
            ModuleDeclarations {
                type_decls: Vec::new(),
                function_decls: vec![ParsedFunctionDecl {
                    name: "make".to_string(),
                    qualified_name: "make".to_string(),
                    access: AccessLevel::Public,
                    params: Vec::new(),
                    return_decl: ParsedFunctionReturnDecl {
                        type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                        location: span.clone(),
                    },
                    code: "ret = 1;".to_string(),
                    location: span.clone(),
                }],
                module_global_var_decls: Vec::new(),
                module_global_const_decls: Vec::new(),
            },
        )]);
        let reachable = BTreeSet::from(["alias.xml".to_string()]);
        let (_types, alias_functions, _module_vars, _module_consts) =
            resolve_visible_module_symbols(&reachable, &alias_already_exists, None)
                .expect("existing alias key should skip insertion branch");
        assert!(alias_functions.contains_key("make"));

        let malformed_local_names = BTreeMap::from([(
            "odd.xml".to_string(),
            ModuleDeclarations {
                type_decls: vec![ParsedTypeDecl {
                    name: "Obj".to_string(),
                    qualified_name: "Obj".to_string(),
                    access: AccessLevel::Public,
                    fields: vec![ParsedTypeFieldDecl {
                        name: "v".to_string(),
                        type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                        location: span.clone(),
                    }],
                    enum_members: Vec::new(),
                    location: span.clone(),
                }],
                function_decls: vec![
                    ParsedFunctionDecl {
                        name: "make".to_string(),
                        qualified_name: "odd.make".to_string(),
                        access: AccessLevel::Public,
                        params: Vec::new(),
                        return_decl: ParsedFunctionReturnDecl {
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
                        return_decl: ParsedFunctionReturnDecl {
                            type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                            location: span.clone(),
                        },
                        code: "ret = 2;".to_string(),
                        location: span.clone(),
                    },
                ],
                module_global_var_decls: Vec::new(),
                module_global_const_decls: Vec::new(),
            },
        )]);
        let reachable = BTreeSet::from(["odd.xml".to_string()]);
        let (malformed_types, malformed_functions, _module_vars, _module_consts) =
            resolve_visible_module_symbols(&reachable, &malformed_local_names, Some("odd"))
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
            ModuleDeclarations {
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
                    return_decl: ParsedFunctionReturnDecl {
                        type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                        location: span.clone(),
                    },
                    code: "ret = 1;".to_string(),
                    location: span.clone(),
                }],
                module_global_var_decls: Vec::new(),
                module_global_const_decls: Vec::new(),
            },
        )]);
        let reachable = BTreeSet::from(["bad.xml".to_string()]);
        let error = resolve_visible_module_symbols(&reachable, &bad_param, Some("bad"))
            .expect_err("function param type should resolve");
        assert_eq!(error.code, "TYPE_UNKNOWN");

        let bad_return = BTreeMap::from([(
            "bad.xml".to_string(),
            ModuleDeclarations {
                type_decls: Vec::new(),
                function_decls: vec![ParsedFunctionDecl {
                    name: "f".to_string(),
                    qualified_name: "bad.f".to_string(),
                    access: AccessLevel::Public,
                    params: Vec::new(),
                    return_decl: ParsedFunctionReturnDecl {
                        type_expr: ParsedTypeExpr::Custom("Missing".to_string()),
                        location: span.clone(),
                    },
                    code: "ret = 1;".to_string(),
                    location: span.clone(),
                }],
                module_global_var_decls: Vec::new(),
                module_global_const_decls: Vec::new(),
            },
        )]);
        let reachable = BTreeSet::from(["bad.xml".to_string()]);
        let error = resolve_visible_module_symbols(&reachable, &bad_return, Some("bad"))
            .expect_err("function return type should resolve");
        assert_eq!(error.code, "TYPE_UNKNOWN");

        let bad_global_type = BTreeMap::from([(
            "bad.xml".to_string(),
            ModuleDeclarations {
                type_decls: Vec::new(),
                function_decls: Vec::new(),
                module_global_var_decls: vec![ParsedModuleVarDecl {
                    namespace: "bad".to_string(),
                    name: "hp".to_string(),
                    qualified_name: "bad.hp".to_string(),
                    access: AccessLevel::Public,
                    type_expr: ParsedTypeExpr::Custom("Missing".to_string()),
                    initial_value_expr: None,
                    location: span.clone(),
                }],
                module_global_const_decls: Vec::new(),
            },
        )]);
        let reachable = BTreeSet::from(["bad.xml".to_string()]);
        let error = resolve_visible_module_symbols(&reachable, &bad_global_type, Some("bad"))
            .expect_err("module global type should resolve");
        assert_eq!(error.code, "TYPE_UNKNOWN");

        let bundle_error = collect_module_vars_for_bundle(&bad_global_type, &BTreeMap::new())
            .expect_err("bundle module global type should resolve");
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
                key_type: MapKeyType::String,
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
            r#"<module name="battle" export="script:main"><script name="main"><text>x</text></script></module>"#,
        )]))
        .expect("sources should parse");

        let module_scripts = parse_module_scripts(&sources).expect("module scripts should parse");
        assert_eq!(module_scripts["battle.xml"].len(), 1);
        assert!(parse_module_files(&sources).is_ok());

        // Test parsing module with enum declaration (covers line 149-151)
        let enum_sources = parse_sources(&compiler_test_support::map(&[(
            "status.xml",
            r#"<module name="status" export="script:main;enum:Status">
<enum name="Status"><member name="Active"/><member name="Inactive"/></enum>
<script name="main"><text>ok</text></script>
</module>"#,
        )]))
        .expect("sources with enum should parse");
        let module_by_path =
            parse_module_files(&enum_sources).expect("module with enum should parse");
        let status_module = module_by_path
            .get("status.xml")
            .expect("should have status.xml");
        assert!(!status_module.type_decls.is_empty());

        let bad_root = SourceFile {
            kind: SourceKind::ModuleXml,
            imports: Vec::new(),
            alias_directives: Vec::new(),
            xml_root: Some(compiler_test_support::xml_element(
                "script",
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
            alias_directives: Vec::new(),
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
            alias_directives: Vec::new(),
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
            alias_directives: Vec::new(),
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
                alias_directives: Vec::new(),
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
    fn module_resolution_helpers_cover_json_and_missing_path_branches() {
        let json_source = SourceFile {
            kind: SourceKind::Json,
            imports: Vec::new(),
            alias_directives: Vec::new(),
            xml_root: None,
            json_value: Some(SlValue::Bool(true)),
        };
        let module_source = SourceFile {
            kind: SourceKind::ModuleXml,
            imports: Vec::new(),
            alias_directives: Vec::new(),
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
        assert!(parse_module_files(&sources).is_ok());
        assert!(parse_module_scripts(&sources).is_ok());

        let duplicate_json = collect_global_data(&BTreeMap::from([
            ("a/game.json".to_string(), json_source.clone()),
            ("b/game.json".to_string(), json_source.clone()),
        ]))
        .expect_err("duplicate global data symbol should fail");
        assert_eq!(duplicate_json.code, "GLOBAL_DATA_SYMBOL_DUPLICATE");

        let collected = collect_global_data(&BTreeMap::from([
            (
                "main.xml".to_string(),
                SourceFile {
                    kind: SourceKind::ModuleXml,
                    imports: Vec::new(),
                    alias_directives: Vec::new(),
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

        let duplicate_visible = collect_visible_global_symbols(
            &BTreeSet::from(["a/game.json".to_string(), "b/game.json".to_string()]),
            &BTreeMap::from([
                ("a/game.json".to_string(), json_source.clone()),
                ("b/game.json".to_string(), json_source.clone()),
            ]),
        )
        .expect_err("duplicate visible global data symbol should fail");
        assert_eq!(duplicate_visible.code, "GLOBAL_DATA_SYMBOL_DUPLICATE");

        let visible = collect_visible_global_symbols(
            &BTreeSet::from(["main.xml".to_string(), "game.json".to_string()]),
            &BTreeMap::from([
                (
                    "main.xml".to_string(),
                    SourceFile {
                        kind: SourceKind::ModuleXml,
                        imports: Vec::new(),
                        alias_directives: Vec::new(),
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
        let module_by_path = BTreeMap::from([(
            "main.xml".to_string(),
            ModuleDeclarations {
                type_decls: vec![ParsedTypeDecl {
                    name: "Player".to_string(),
                    qualified_name: "main.Player".to_string(),
                    access: AccessLevel::Public,
                    fields: vec![ParsedTypeFieldDecl {
                        name: "hp".to_string(),
                        type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                        location: span.clone(),
                    }],
                    enum_members: Vec::new(),
                    location: span.clone(),
                }],
                function_decls: vec![ParsedFunctionDecl {
                    name: "boost".to_string(),
                    qualified_name: "main.boost".to_string(),
                    access: AccessLevel::Public,
                    params: Vec::new(),
                    return_decl: ParsedFunctionReturnDecl {
                        type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                        location: span.clone(),
                    },
                    code: "out = 1;".to_string(),
                    location: span.clone(),
                }],
                module_global_var_decls: vec![ParsedModuleVarDecl {
                    namespace: "main".to_string(),
                    name: "hp".to_string(),
                    qualified_name: "main.hp".to_string(),
                    access: AccessLevel::Public,
                    type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                    initial_value_expr: None,
                    location: span,
                }],
                module_global_const_decls: Vec::new(),
            },
        )]);
        let reachable = BTreeSet::from(["main.xml".to_string(), "missing.xml".to_string()]);
        let (types, functions, module_vars, _module_consts) =
            resolve_visible_module_symbols(&reachable, &module_by_path, Some("main"))
                .expect("missing paths in reachable closure should be skipped");
        assert!(types.contains_key("Player"));
        assert!(functions.contains_key("boost"));
        assert!(module_vars.contains_key("hp"));
    }

    #[test]
    fn compile_bundle_supports_module_const_declarations() {
        let files = map(&[(
            "main.xml",
            r#"<module name="main" export="script:main;const:base">
  <const name="base" type="int">7</const>
  <script name="main"><text>${base}</text></script>
</module>"#,
        )]);
        let bundle = compile_project_bundle_from_xml_map(&files).expect("compile should pass");
        assert!(bundle.module_const_declarations.contains_key("main.base"));
        assert_eq!(
            bundle.module_const_init_order,
            vec!["main.base".to_string()]
        );
    }

    #[test]
    fn compile_bundle_rejects_const_initializer_referencing_var() {
        let files = map(&[(
            "main.xml",
            r#"<module name="main" export="script:main;var:hp;const:bad">
  <var name="hp" type="int">10</var>
  <const name="bad" type="int">hp + 1</const>
  <script name="main"><text>${bad}</text></script>
</module>"#,
        )]);
        let error = compile_project_bundle_from_xml_map(&files)
            .expect_err("const initializer referencing var should fail");
        assert_eq!(error.code, "MODULE_CONST_INIT_REF_NON_CONST");

        let files_qualified = map(&[(
            "main.xml",
            r#"<module name="main" export="script:main;var:hp;const:bad">
  <var name="hp" type="int">10</var>
  <const name="bad" type="int">main.hp + 1</const>
  <script name="main"><text>${bad}</text></script>
</module>"#,
        )]);
        let qualified_error = compile_project_bundle_from_xml_map(&files_qualified)
            .expect_err("const initializer referencing qualified var should fail");
        assert_eq!(qualified_error.code, "MODULE_CONST_INIT_REF_NON_CONST");
    }

    #[test]
    fn resolve_visible_module_symbols_skips_private_types_from_non_local_module() {
        let span = SourceSpan::synthetic();
        let module = ModuleDeclarations {
            type_decls: vec![ParsedTypeDecl {
                name: "Secret".to_string(),
                qualified_name: "other.Secret".to_string(),
                access: AccessLevel::Private,
                fields: vec![ParsedTypeFieldDecl {
                    name: "v".to_string(),
                    type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                    location: span.clone(),
                }],
                enum_members: Vec::new(),
                location: span.clone(),
            }],
            function_decls: Vec::new(),
            module_global_var_decls: Vec::new(),
            module_global_const_decls: Vec::new(),
        };

        let reachable = BTreeSet::from(["other.xml".to_string()]);
        let module_by_path = BTreeMap::from([("other.xml".to_string(), module)]);

        // Query from module "main" should NOT see "other.Secret" because it's private
        let (types, functions, module_vars, _module_consts) =
            resolve_visible_module_symbols(&reachable, &module_by_path, Some("main"))
                .expect("should resolve");
        assert!(
            !types.contains_key("Secret"),
            "private type from non-local should be hidden"
        );
        assert!(functions.is_empty());
        assert!(module_vars.is_empty());
    }

    #[test]
    fn resolve_visible_module_symbols_skips_private_functions_from_non_local_module() {
        let span = SourceSpan::synthetic();
        let module = ModuleDeclarations {
            type_decls: Vec::new(),
            function_decls: vec![ParsedFunctionDecl {
                name: "hidden".to_string(),
                qualified_name: "other.hidden".to_string(),
                access: AccessLevel::Private,
                params: Vec::new(),
                return_decl: ParsedFunctionReturnDecl {
                    type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                    location: span.clone(),
                },
                code: "out = 1;".to_string(),
                location: span.clone(),
            }],
            module_global_var_decls: Vec::new(),
            module_global_const_decls: Vec::new(),
        };

        let reachable = BTreeSet::from(["other.xml".to_string()]);
        let module_by_path = BTreeMap::from([("other.xml".to_string(), module)]);

        // Query from module "main" should NOT see "other.hidden" because it's private
        let (types, functions, module_vars, _module_consts) =
            resolve_visible_module_symbols(&reachable, &module_by_path, Some("main"))
                .expect("should resolve");
        assert!(types.is_empty());
        assert!(
            !functions.contains_key("hidden"),
            "private function from non-local should be hidden"
        );
        assert!(module_vars.is_empty());
    }

    #[test]
    fn parse_module_const_declaration_validates_shape() {
        let node = xml_element(
            "const",
            &[("name", "base"), ("type", "int")],
            vec![xml_text("7")],
        );
        let parsed = parse_module_const_declaration(&node, "main", AccessLevel::Private)
            .expect("const should parse");
        assert_eq!(parsed.qualified_name, "main.base");

        let with_child = xml_element(
            "const",
            &[("name", "base"), ("type", "int")],
            vec![XmlNode::Element(xml_element(
                "text",
                &[],
                vec![xml_text("x")],
            ))],
        );
        let child_error = parse_module_const_declaration(&with_child, "main", AccessLevel::Private)
            .expect_err("child should fail");
        assert_eq!(child_error.code, "XML_VAR_CHILD_INVALID");
    }

    #[test]
    fn resolve_visible_module_symbols_includes_public_consts_and_local_private_consts() {
        let span = SourceSpan::synthetic();
        let module_by_path = BTreeMap::from([
            (
                "main.xml".to_string(),
                ModuleDeclarations {
                    type_decls: Vec::new(),
                    function_decls: Vec::new(),
                    module_global_var_decls: Vec::new(),
                    module_global_const_decls: vec![
                        ParsedModuleConstDecl {
                            namespace: "main".to_string(),
                            name: "localConst".to_string(),
                            qualified_name: "main.localConst".to_string(),
                            access: AccessLevel::Private,
                            type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                            initial_value_expr: Some("1".to_string()),
                            location: span.clone(),
                        },
                        ParsedModuleConstDecl {
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
                ModuleDeclarations {
                    type_decls: Vec::new(),
                    function_decls: Vec::new(),
                    module_global_var_decls: Vec::new(),
                    module_global_const_decls: vec![ParsedModuleConstDecl {
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
        let (_types, _functions, _module_vars, module_consts) =
            resolve_visible_module_symbols(&reachable, &module_by_path, Some("main"))
                .expect("resolve");
        assert!(module_consts.contains_key("main.localConst"));
        assert!(module_consts.contains_key("sharedConst"));
        assert!(!module_consts.contains_key("other.hidden"));
    }

    #[test]
    fn collect_module_consts_for_bundle_rejects_duplicate_and_forward_reference() {
        let span = SourceSpan::synthetic();
        let module_vars = BTreeMap::from([(
            "main.hp".to_string(),
            ModuleVarDecl {
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
                ModuleDeclarations {
                    type_decls: Vec::new(),
                    function_decls: Vec::new(),
                    module_global_var_decls: Vec::new(),
                    module_global_const_decls: vec![ParsedModuleConstDecl {
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
                ModuleDeclarations {
                    type_decls: Vec::new(),
                    function_decls: Vec::new(),
                    module_global_var_decls: Vec::new(),
                    module_global_const_decls: vec![ParsedModuleConstDecl {
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
        let duplicate_error =
            collect_module_consts_for_bundle(&duplicate, &module_vars, &BTreeMap::new())
                .expect_err("duplicate const should fail");
        assert_eq!(duplicate_error.code, "MODULE_GLOBAL_CONST_DUPLICATE");

        let bad_order = BTreeMap::from([(
            "main.xml".to_string(),
            ModuleDeclarations {
                type_decls: Vec::new(),
                function_decls: Vec::new(),
                module_global_var_decls: Vec::new(),
                module_global_const_decls: vec![
                    ParsedModuleConstDecl {
                        namespace: "main".to_string(),
                        name: "a".to_string(),
                        qualified_name: "main.a".to_string(),
                        access: AccessLevel::Public,
                        type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                        initial_value_expr: Some("b + 1".to_string()),
                        location: SourceSpan::synthetic(),
                    },
                    ParsedModuleConstDecl {
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
        let order_error =
            collect_module_consts_for_bundle(&bad_order, &module_vars, &BTreeMap::new())
                .expect_err("forward const reference should fail");
        assert_eq!(order_error.code, "MODULE_CONST_INIT_ORDER");
    }

    #[test]
    fn resolve_visible_module_symbols_rejects_duplicate_module_const_in_closure() {
        let span = SourceSpan::synthetic();
        let duplicate = ParsedModuleConstDecl {
            namespace: "shared".to_string(),
            name: "base".to_string(),
            qualified_name: "shared.base".to_string(),
            access: AccessLevel::Public,
            type_expr: ParsedTypeExpr::Primitive("int".to_string()),
            initial_value_expr: Some("1".to_string()),
            location: span.clone(),
        };
        let module_by_path = BTreeMap::from([
            (
                "a.xml".to_string(),
                ModuleDeclarations {
                    type_decls: Vec::new(),
                    function_decls: Vec::new(),
                    module_global_var_decls: Vec::new(),
                    module_global_const_decls: vec![duplicate.clone()],
                },
            ),
            (
                "b.xml".to_string(),
                ModuleDeclarations {
                    type_decls: Vec::new(),
                    function_decls: Vec::new(),
                    module_global_var_decls: Vec::new(),
                    module_global_const_decls: vec![duplicate],
                },
            ),
        ]);
        let reachable = BTreeSet::from(["a.xml".to_string(), "b.xml".to_string()]);
        let error = resolve_visible_module_symbols(&reachable, &module_by_path, Some("a"))
            .expect_err("duplicate module const should fail");
        assert_eq!(error.code, "MODULE_GLOBAL_CONST_DUPLICATE");
    }

    #[test]
    fn collect_module_consts_rejects_duplicate_type_in_bundle() {
        let span = SourceSpan::synthetic();
        let duplicate_type = ParsedTypeDecl {
            name: "T".to_string(),
            qualified_name: "main.T".to_string(),
            access: AccessLevel::Public,
            fields: vec![],
            enum_members: Vec::new(),
            location: span.clone(),
        };
        let module_by_path = BTreeMap::from([
            (
                "a.xml".to_string(),
                ModuleDeclarations {
                    type_decls: vec![duplicate_type.clone()],
                    function_decls: Vec::new(),
                    module_global_var_decls: Vec::new(),
                    module_global_const_decls: Vec::new(),
                },
            ),
            (
                "b.xml".to_string(),
                ModuleDeclarations {
                    type_decls: vec![duplicate_type],
                    function_decls: Vec::new(),
                    module_global_var_decls: Vec::new(),
                    module_global_const_decls: Vec::new(),
                },
            ),
        ]);
        let module_vars = BTreeMap::new();
        let error =
            collect_module_consts_for_bundle(&module_by_path, &module_vars, &BTreeMap::new())
                .expect_err("duplicate type should fail");
        assert_eq!(error.code, "TYPE_DECL_DUPLICATE");
    }

    #[test]
    fn validate_module_const_init_rules_handles_ambiguous_short_name() {
        // Test when multiple module_const have the same short name (candidates.len() > 1)
        let span = SourceSpan::synthetic();
        let module_consts = BTreeMap::from([
            (
                "main.base".to_string(),
                ModuleConstDecl {
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
                ModuleConstDecl {
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
        let module_vars = BTreeMap::new();
        let init_order = vec!["main.base".to_string(), "other.base".to_string()];
        // This should NOT error because we just validate the init order
        let result = validate_module_const_init_rules(&module_consts, &init_order, &module_vars);
        assert!(
            result.is_ok(),
            "ambiguous short name should be filtered out in mapping"
        );
    }

    #[test]
    fn validate_module_const_init_rules_rejects_forward_reference() {
        // Test when a module_const references another const that hasn't been initialized yet
        let span = SourceSpan::synthetic();
        let module_consts = BTreeMap::from([
            (
                "main.first".to_string(),
                ModuleConstDecl {
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
                ModuleConstDecl {
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
        let module_vars = BTreeMap::new();
        // Initialize first before second - this should fail
        let init_order = vec!["main.first".to_string(), "main.second".to_string()];
        let error = validate_module_const_init_rules(&module_consts, &init_order, &module_vars)
            .expect_err("forward reference should fail");
        assert_eq!(error.code, "MODULE_CONST_INIT_ORDER");
    }

    #[test]
    fn validate_module_const_init_rules_handles_name_not_in_mapping() {
        // Test when const references a name that is NOT in const_name_to_qualified
        // This covers lines 1082-1084: continue when name not in mapping
        let span = SourceSpan::synthetic();
        // Create a const that references a variable name (not a const name)
        let module_consts = BTreeMap::from([
            (
                "main.base".to_string(),
                ModuleConstDecl {
                    namespace: "main".to_string(),
                    name: "base".to_string(),
                    qualified_name: "main.base".to_string(),
                    access: AccessLevel::Public,
                    r#type: ScriptType::Primitive {
                        name: "int".to_string(),
                    },
                    // References "score" which is not a const name (no entry in const_name_to_qualified)
                    initial_value_expr: Some("score + 1".to_string()),
                    location: span.clone(),
                },
            ),
            (
                "main.value".to_string(),
                ModuleConstDecl {
                    namespace: "main".to_string(),
                    name: "value".to_string(),
                    qualified_name: "main.value".to_string(),
                    access: AccessLevel::Public,
                    r#type: ScriptType::Primitive {
                        name: "int".to_string(),
                    },
                    // References "base" which IS in const_name_to_qualified
                    initial_value_expr: Some("base + 10".to_string()),
                    location: span.clone(),
                },
            ),
            // Create a const without initial_value_expr to cover line 1096 (if block with Some)
            (
                "main.no_init".to_string(),
                ModuleConstDecl {
                    namespace: "main".to_string(),
                    name: "no_init".to_string(),
                    qualified_name: "main.no_init".to_string(),
                    access: AccessLevel::Public,
                    r#type: ScriptType::Primitive {
                        name: "int".to_string(),
                    },
                    // No initial value expression - triggers None branch at line 1057
                    initial_value_expr: None,
                    location: span.clone(),
                },
            ),
        ]);
        let module_vars = BTreeMap::new();
        // Initialize base first, then value, then no_init
        let init_order = vec![
            "main.base".to_string(),
            "main.value".to_string(),
            "main.no_init".to_string(),
        ];
        let result = validate_module_const_init_rules(&module_consts, &init_order, &module_vars);
        assert!(
            result.is_ok(),
            "referencing initialized const should be allowed"
        );
    }

    #[test]
    fn resolve_visible_module_symbols_reports_type_resolution_error() {
        // Test that type resolution errors propagate through line 784
        // This creates a type with a field referencing a non-existent type
        let span = SourceSpan::synthetic();
        let module = ModuleDeclarations {
            type_decls: vec![ParsedTypeDecl {
                name: "MyType".to_string(),
                qualified_name: "shared.MyType".to_string(),
                access: AccessLevel::Public,
                fields: vec![ParsedTypeFieldDecl {
                    name: "field".to_string(),
                    type_expr: ParsedTypeExpr::Custom("NonExistentType".to_string()),
                    location: span.clone(),
                }],
                enum_members: Vec::new(),
                location: span.clone(),
            }],
            function_decls: Vec::new(),
            module_global_var_decls: Vec::new(),
            module_global_const_decls: Vec::new(),
        };

        let reachable = BTreeSet::from(["shared.xml".to_string()]);
        let module_by_path = BTreeMap::from([("shared.xml".to_string(), module)]);

        let error = resolve_visible_module_symbols(&reachable, &module_by_path, None)
            .expect_err("type resolution should fail for non-existent type");
        assert_eq!(error.code, "TYPE_UNKNOWN");
    }

    #[test]
    fn resolve_visible_module_symbols_reports_duplicate_field_error() {
        // Test that duplicate field errors propagate through line 784
        let span = SourceSpan::synthetic();
        let module = ModuleDeclarations {
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
                enum_members: Vec::new(),
                location: span.clone(),
            }],
            function_decls: Vec::new(),
            module_global_var_decls: Vec::new(),
            module_global_const_decls: Vec::new(),
        };

        let reachable = BTreeSet::from(["shared.xml".to_string()]);
        let module_by_path = BTreeMap::from([("shared.xml".to_string(), module)]);

        let error = resolve_visible_module_symbols(&reachable, &module_by_path, None)
            .expect_err("duplicate field should fail");
        assert_eq!(error.code, "TYPE_FIELD_DUPLICATE");
    }

    #[test]
    fn collect_functions_for_bundle_rejects_unknown_param_type() {
        let span = SourceSpan::synthetic();
        // Function with param type that doesn't exist
        let module = ModuleDeclarations {
            type_decls: Vec::new(),
            function_decls: vec![ParsedFunctionDecl {
                name: "foo".to_string(),
                qualified_name: "shared.foo".to_string(),
                access: AccessLevel::Public,
                params: vec![ParsedFunctionParamDecl {
                    name: "x".to_string(),
                    type_expr: ParsedTypeExpr::Custom("UnknownType".to_string()),
                    location: span.clone(),
                }],
                return_decl: ParsedFunctionReturnDecl {
                    type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                    location: span.clone(),
                },
                code: "out = 1;".to_string(),
                location: span.clone(),
            }],
            module_global_var_decls: Vec::new(),
            module_global_const_decls: Vec::new(),
        };
        let module_by_path = BTreeMap::from([("a.xml".to_string(), module)]);
        let error = collect_functions_for_bundle_with_aliases(&module_by_path, &BTreeMap::new())
            .expect_err("unknown param type should fail");
        assert_eq!(error.code, "TYPE_UNKNOWN");
    }

    #[test]
    fn collect_functions_for_bundle_rejects_unknown_return_type() {
        let span = SourceSpan::synthetic();
        // Function with return type that doesn't exist
        let module = ModuleDeclarations {
            type_decls: Vec::new(),
            function_decls: vec![ParsedFunctionDecl {
                name: "foo".to_string(),
                qualified_name: "shared.foo".to_string(),
                access: AccessLevel::Public,
                params: vec![],
                return_decl: ParsedFunctionReturnDecl {
                    type_expr: ParsedTypeExpr::Custom("NonExistentType".to_string()),
                    location: span.clone(),
                },
                code: "out = 1;".to_string(),
                location: span.clone(),
            }],
            module_global_var_decls: Vec::new(),
            module_global_const_decls: Vec::new(),
        };
        let module_by_path = BTreeMap::from([("a.xml".to_string(), module)]);
        let error = collect_functions_for_bundle_with_aliases(&module_by_path, &BTreeMap::new())
            .expect_err("unknown return type should fail");
        assert_eq!(error.code, "TYPE_UNKNOWN");
    }

    #[test]
    fn collect_module_vars_for_bundle_rejects_duplicate_type() {
        let span = SourceSpan::synthetic();
        // Two module files with the same type
        let type_decl = ParsedTypeDecl {
            name: "Obj".to_string(),
            qualified_name: "shared.Obj".to_string(),
            access: AccessLevel::Public,
            fields: vec![],
            enum_members: Vec::new(),
            location: span.clone(),
        };
        let module1 = ModuleDeclarations {
            type_decls: vec![type_decl.clone()],
            function_decls: Vec::new(),
            module_global_var_decls: Vec::new(),
            module_global_const_decls: Vec::new(),
        };
        let module2 = ModuleDeclarations {
            type_decls: vec![type_decl],
            function_decls: Vec::new(),
            module_global_var_decls: Vec::new(),
            module_global_const_decls: Vec::new(),
        };
        let module_by_path = BTreeMap::from([
            ("a.xml".to_string(), module1),
            ("b.xml".to_string(), module2),
        ]);
        let error = collect_module_vars_for_bundle(&module_by_path, &BTreeMap::new())
            .expect_err("duplicate type should fail");
        assert_eq!(error.code, "TYPE_DECL_DUPLICATE");
    }

    #[test]
    fn collect_module_vars_for_bundle_rejects_recursive_type() {
        let span = SourceSpan::synthetic();
        // Type that references a non-existent type
        let invalid_type = ParsedTypeDecl {
            name: "Node".to_string(),
            qualified_name: "shared.Node".to_string(),
            access: AccessLevel::Public,
            fields: vec![ParsedTypeFieldDecl {
                name: "value".to_string(),
                type_expr: ParsedTypeExpr::Custom("NonExistent".to_string()), // doesn't exist
                location: span.clone(),
            }],
            enum_members: Vec::new(),
            location: span.clone(),
        };
        let module = ModuleDeclarations {
            type_decls: vec![invalid_type],
            function_decls: Vec::new(),
            module_global_var_decls: Vec::new(),
            module_global_const_decls: Vec::new(),
        };
        let module_by_path = BTreeMap::from([("a.xml".to_string(), module)]);
        let error = collect_module_vars_for_bundle(&module_by_path, &BTreeMap::new())
            .expect_err("invalid type should fail");
        assert_eq!(error.code, "TYPE_UNKNOWN");
    }

    #[test]
    fn collect_module_consts_for_bundle_rejects_recursive_type() {
        let span = SourceSpan::synthetic();
        // Type that references a non-existent type
        let invalid_type = ParsedTypeDecl {
            name: "Tree".to_string(),
            qualified_name: "shared.Tree".to_string(),
            access: AccessLevel::Public,
            fields: vec![ParsedTypeFieldDecl {
                name: "value".to_string(),
                type_expr: ParsedTypeExpr::Custom("DoesNotExist".to_string()), // doesn't exist
                location: span.clone(),
            }],
            enum_members: Vec::new(),
            location: span.clone(),
        };
        let module = ModuleDeclarations {
            type_decls: vec![invalid_type],
            function_decls: Vec::new(),
            module_global_var_decls: Vec::new(),
            module_global_const_decls: Vec::new(),
        };
        let module_by_path = BTreeMap::from([("a.xml".to_string(), module)]);
        let error =
            collect_module_consts_for_bundle(&module_by_path, &BTreeMap::new(), &BTreeMap::new())
                .expect_err("invalid type should fail");
        assert_eq!(error.code, "TYPE_UNKNOWN");
    }

    #[test]
    fn normalize_module_initializer_rejects_enum_without_init() {
        // Test line 199: enum type var without initializer returns ENUM_INIT_REQUIRED error
        let span = SourceSpan::synthetic();
        let enum_type = ScriptType::Enum {
            type_name: "Status".to_string(),
            members: vec!["Active".to_string(), "Inactive".to_string()],
        };

        // None expr with enum type should return error
        let result = normalize_module_initializer(
            &None,
            &enum_type,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            "main",
            &span,
        );
        let error = result.expect_err("enum without init should fail");
        assert_eq!(error.code, "ENUM_INIT_REQUIRED");
    }

    #[test]
    fn normalize_module_initializer_handles_enum_with_init() {
        // Test line 208-210: enum type var with initializer
        let span = SourceSpan::synthetic();
        let enum_type = ScriptType::Enum {
            type_name: "Status".to_string(),
            members: vec!["Active".to_string(), "Inactive".to_string()],
        };
        let visible_types = BTreeMap::from([("Status".to_string(), enum_type.clone())]);

        // Some expr with enum type should succeed
        let result = normalize_module_initializer(
            &Some("Status.Active".to_string()),
            &enum_type,
            &BTreeMap::new(),
            &visible_types,
            &BTreeMap::new(),
            "main",
            &span,
        );
        let value = result
            .expect("enum with init should succeed")
            .expect("should have value");
        assert!(value.contains("Active"));
    }

    #[test]
    fn normalize_module_initializer_rejects_invalid_enum_member() {
        // Test line 209: parse_enum_literal_initializer returns error for unknown member
        let span = SourceSpan::synthetic();
        let enum_type = ScriptType::Enum {
            type_name: "Status".to_string(),
            members: vec!["Active".to_string(), "Inactive".to_string()],
        };
        let visible_types = BTreeMap::from([("Status".to_string(), enum_type.clone())]);

        // Invalid member name should return error
        let result = normalize_module_initializer(
            &Some("Status.Unknown".to_string()),
            &enum_type,
            &BTreeMap::new(),
            &visible_types,
            &BTreeMap::new(),
            "main",
            &span,
        );
        let error = result.expect_err("invalid enum member should fail");
        assert_eq!(error.code, "ENUM_LITERAL_MEMBER_UNKNOWN");
    }

    #[test]
    fn normalize_module_initializer_rejects_string_literal_for_enum() {
        // Test line 209: parse_enum_literal_initializer returns error for string literal
        let span = SourceSpan::synthetic();
        let enum_type = ScriptType::Enum {
            type_name: "Status".to_string(),
            members: vec!["Active".to_string(), "Inactive".to_string()],
        };
        let visible_types = BTreeMap::from([("Status".to_string(), enum_type.clone())]);

        // String literal instead of Type.Member should return error
        let result = normalize_module_initializer(
            &Some("\"Active\"".to_string()),
            &enum_type,
            &BTreeMap::new(),
            &visible_types,
            &BTreeMap::new(),
            "main",
            &span,
        );
        let error = result.expect_err("string literal for enum should fail");
        assert_eq!(error.code, "ENUM_LITERAL_REQUIRED");
    }

    #[test]
    fn normalize_module_initializer_rejects_invalid_enum_in_non_enum_type() {
        // Test line 217: rewrite_and_validate_enum_literals_in_expression returns error
        // when non-enum type variable has invalid enum literal in expression
        let span = SourceSpan::synthetic();
        let int_type = ScriptType::Primitive {
            name: "int".to_string(),
        };
        let enum_type = ScriptType::Enum {
            type_name: "Status".to_string(),
            members: vec!["Active".to_string(), "Inactive".to_string()],
        };
        let visible_types = BTreeMap::from([("Status".to_string(), enum_type)]);

        // Non-enum type with invalid enum literal should return error
        let result = normalize_module_initializer(
            &Some("${Status.Unknown}".to_string()),
            &int_type,
            &BTreeMap::new(),
            &visible_types,
            &BTreeMap::new(),
            "main",
            &span,
        );
        let error = result.expect_err("invalid enum literal in non-enum type should fail");
        assert_eq!(error.code, "ENUM_LITERAL_MEMBER_UNKNOWN");
    }

    #[test]
    fn normalize_module_initializer_validates_enum_map_keys_for_static_literals() {
        let span = SourceSpan::synthetic();
        let enum_key_map = ScriptType::Map {
            key_type: MapKeyType::Enum {
                type_name: "Status".to_string(),
                members: vec!["Active".to_string(), "Inactive".to_string()],
            },
            value_type: Box::new(ScriptType::Primitive {
                name: "int".to_string(),
            }),
        };

        let valid = normalize_module_initializer(
            &Some("#{Active: 1}".to_string()),
            &enum_key_map,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            "main",
            &span,
        )
        .expect("valid enum map initializer should pass");
        assert_eq!(valid.as_deref(), Some("#{Active: 1}"));

        let invalid = normalize_module_initializer(
            &Some("#{Unknown: 1}".to_string()),
            &enum_key_map,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            "main",
            &span,
        )
        .expect_err("unknown enum map key should fail");
        assert_eq!(invalid.code, "ENUM_MAP_KEY_UNKNOWN");
    }

    #[test]
    fn normalize_module_initializer_rejects_missing_function_reference() {
        // Test line 571: normalize_and_validate_function_literals error for missing function
        let span = SourceSpan::synthetic();
        let int_type = ScriptType::Primitive {
            name: "int".to_string(),
        };

        // Provide an expression with a function reference (*nonexistent) not in visible_functions
        let result = normalize_module_initializer(
            &Some("*nonexistent_func".to_string()),
            &int_type,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(), // empty visible_functions
            "main",
            &span,
        );
        let error = result.expect_err("missing function reference should fail");
        assert_eq!(error.code, "XML_FUNCTION_LITERAL_NOT_FOUND");
    }

    #[test]
    fn normalize_module_initializer_rejects_function_call_syntax() {
        // Test lines 604, 668, 902, 1004: normalize_module_initializer error propagation
        // for module variables and constants with invalid enum literal initializers
        use crate::compiler_test_support::*;

        // Test module var with invalid enum literal (triggers lines 604/902)
        let files = map(&[(
            "main.xml",
            r#"<module name="main" export="script:main;enum:Status;var:status">
<enum name="Status"><member name="Active"/><member name="Inactive"/></enum>
<var name="status" type="Status">Status.Unknown</var>
<script name="main"><text>test</text></script>
</module>"#,
        )]);
        let error = crate::compile_project_bundle_from_xml_map(&files)
            .expect_err("module var with invalid enum member should fail");
        assert_eq!(error.code, "ENUM_LITERAL_MEMBER_UNKNOWN");

        // Test module const with invalid enum literal (triggers lines 668/1004)
        let files_const = map(&[(
            "main.xml",
            r#"<module name="main" export="script:main;enum:Status;const:status">
<enum name="Status"><member name="Active"/><member name="Inactive"/></enum>
<const name="status" type="Status">Status.Unknown</const>
<script name="main"><text>test</text></script>
</module>"#,
        )]);
        let error_const = crate::compile_project_bundle_from_xml_map(&files_const)
            .expect_err("module const with invalid enum member should fail");
        assert_eq!(error_const.code, "ENUM_LITERAL_MEMBER_UNKNOWN");
    }

    #[test]
    fn resolve_visible_module_symbols_rejects_function_with_invalid_enum_in_code() {
        // Test lines 533, 805: rewrite_and_validate_enum_literals_in_expression error propagation
        // for function code containing invalid enum literal
        use crate::compiler_test_support::*;

        // Test function with invalid enum literal in code body
        let files = map(&[(
            "main.xml",
            r#"<module name="main" export="script:main;function:test;enum:Status">
<enum name="Status"><member name="Active"/><member name="Inactive"/></enum>
<function name="test" args="" return_type="int">return Status.Unknown;</function>
<script name="main"><text>test</text></script>
</module>"#,
        )]);
        let error = crate::compile_project_bundle_from_xml_map(&files)
            .expect_err("function with invalid enum literal should fail");
        assert_eq!(error.code, "ENUM_LITERAL_MEMBER_UNKNOWN");
    }

    #[test]
    fn parse_module_source_validates_export_targets_not_found() {
        // Test lines 337-342: validate_export_names error for non-existent export targets
        // Use compile_project_bundle_from_xml_map for simpler test construction

        // Test export function that doesn't exist
        let files = BTreeMap::from([(
            "test.xml".to_string(),
            r#"<module name="test" export="function:NonExistent">
<function name="actual_func" args="" return_type="int">return 1;</function>
<script name="main"><text>x = 1;</text></script>
</module>"#
                .to_string(),
        )]);
        let error = crate::compile_project_bundle_from_xml_map(&files)
            .expect_err("exporting non-existent function should fail");
        assert_eq!(error.code, "XML_EXPORT_TARGET_NOT_FOUND");

        // Test export type that doesn't exist
        let files = BTreeMap::from([(
            "test.xml".to_string(),
            r#"<module name="test" export="type:NonExistentType">
<type name="ActualType"/>
<script name="main"><text>x = 1;</text></script>
</module>"#
                .to_string(),
        )]);
        let error = crate::compile_project_bundle_from_xml_map(&files)
            .expect_err("exporting non-existent type should fail");
        assert_eq!(error.code, "XML_EXPORT_TARGET_NOT_FOUND");

        // Test export enum that doesn't exist
        let files = BTreeMap::from([(
            "test.xml".to_string(),
            r#"<module name="test" export="enum:NonExistentEnum">
<enum name="ActualEnum"><member name="A"/></enum>
<script name="main"><text>x = 1;</text></script>
</module>"#
                .to_string(),
        )]);
        let error = crate::compile_project_bundle_from_xml_map(&files)
            .expect_err("exporting non-existent enum should fail");
        assert_eq!(error.code, "XML_EXPORT_TARGET_NOT_FOUND");

        // Test export var that doesn't exist
        let files = BTreeMap::from([(
            "test.xml".to_string(),
            r#"<module name="test" export="var:NonExistentVar">
<script name="main"><text>x = 1;</text></script>
</module>"#
                .to_string(),
        )]);
        let error = crate::compile_project_bundle_from_xml_map(&files)
            .expect_err("exporting non-existent var should fail");
        assert_eq!(error.code, "XML_EXPORT_TARGET_NOT_FOUND");

        // Test export const that doesn't exist
        let files = BTreeMap::from([(
            "test.xml".to_string(),
            r#"<module name="test" export="const:NonExistentConst">
<script name="main"><text>x = 1;</text></script>
</module>"#
                .to_string(),
        )]);
        let error = crate::compile_project_bundle_from_xml_map(&files)
            .expect_err("exporting non-existent const should fail");
        assert_eq!(error.code, "XML_EXPORT_TARGET_NOT_FOUND");

        // Test export script that doesn't exist
        let files = BTreeMap::from([(
            "test.xml".to_string(),
            r#"<module name="test" export="script:NonExistentScript">
<script name="main"><text>x = 1;</text></script>
</module>"#
                .to_string(),
        )]);
        let error = crate::compile_project_bundle_from_xml_map(&files)
            .expect_err("exporting non-existent script should fail");
        assert_eq!(error.code, "XML_EXPORT_TARGET_NOT_FOUND");
    }

    #[test]
    fn parse_module_source_sets_private_access_for_unexported_members() {
        // Test lines 351, 358, 365, 372: Private access level for unexported members
        // Create a module with multiple members but only export some
        let files = BTreeMap::from([(
            "test.xml".to_string(),
            r#"<module name="test" export="function:public_func">
<function name="public_func" args="" return_type="int">return 1;</function>
<function name="private_func" args="" return_type="int">return 2;</function>
<type name="PublicType"/>
<type name="PrivateType"/>
<var name="public_var" type="int">1</var>
<var name="private_var" type="int">2</var>
<const name="public_const" type="int">1</const>
<const name="private_const" type="int">2</const>
<enum name="PublicEnum"><member name="A"/></enum>
<enum name="PrivateEnum"><member name="B"/></enum>
<script name="main"><text>x = 1;</text></script>
</module>"#
                .to_string(),
        )]);
        // This should compile successfully - unexported members become private
        let _bundle = crate::compile_project_bundle_from_xml_map(&files)
            .expect("module with private members should compile");
    }

    #[test]
    fn module_scoped_symbol_alias_conflict() {
        // Test lines 465-469: collect_module_explicit_visible_symbol_aliases conflict detection
        // Create two module-scoped symbol aliases with same alias but different targets
        let span = SourceSpan::synthetic();
        let shared_module = ModuleDeclarations {
            type_decls: vec![],
            function_decls: vec![],
            module_global_var_decls: vec![
                ParsedModuleVarDecl {
                    namespace: "shared".to_string(),
                    name: "hp".to_string(),
                    qualified_name: "shared.hp".to_string(),
                    access: AccessLevel::Public,
                    type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                    initial_value_expr: None,
                    location: span.clone(),
                },
                ParsedModuleVarDecl {
                    namespace: "shared".to_string(),
                    name: "mp".to_string(),
                    qualified_name: "shared.mp".to_string(),
                    access: AccessLevel::Public,
                    type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                    initial_value_expr: None,
                    location: span.clone(),
                },
            ],
            module_global_const_decls: vec![],
        };

        let modules = BTreeMap::from([("shared.xml".to_string(), shared_module)]);
        let reachable = BTreeSet::from(["shared.xml".to_string()]);

        // Two aliases with same name "stat" pointing to different targets in same namespace
        let module_alias_directives_by_namespace = BTreeMap::from([(
            "shared".to_string(),
            vec![
                AliasDirective {
                    target_qualified_name: "shared.hp".to_string(),
                    alias_name: "stat".to_string(),
                },
                AliasDirective {
                    target_qualified_name: "shared.mp".to_string(),
                    alias_name: "stat".to_string(),
                },
            ],
        )]);

        let conflict = resolve_visible_module_symbols_with_aliases_and_module_scoped_type_aliases(
            &reachable,
            &modules,
            Some("main"),
            &[],
            &module_alias_directives_by_namespace,
        )
        .expect_err("same alias to different symbol targets should fail");
        assert_eq!(conflict.code, "ALIAS_NAME_CONFLICT");
    }

    #[test]
    fn namespace_merge_symbol_alias_conflict() {
        // Test lines 494-498: merge_namespace_module_symbol_aliases conflict detection
        // This happens when module-scoped explicit alias conflicts with namespace-level alias
        // created from module global var/const declarations
        let span = SourceSpan::synthetic();
        let shared_module = ModuleDeclarations {
            type_decls: vec![],
            function_decls: vec![],
            module_global_var_decls: vec![
                ParsedModuleVarDecl {
                    namespace: "shared".to_string(),
                    name: "hp".to_string(),
                    qualified_name: "shared.hp".to_string(),
                    access: AccessLevel::Public,
                    type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                    initial_value_expr: None,
                    location: span.clone(),
                },
                ParsedModuleVarDecl {
                    namespace: "shared".to_string(),
                    name: "mp".to_string(),
                    qualified_name: "shared.mp".to_string(),
                    access: AccessLevel::Public,
                    type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                    initial_value_expr: None,
                    location: span.clone(),
                },
            ],
            module_global_const_decls: vec![],
        };

        let modules = BTreeMap::from([("shared.xml".to_string(), shared_module)]);
        let reachable = BTreeSet::from(["shared.xml".to_string()]);

        // Module-scoped alias "hp" conflicts with namespace alias created from var declaration
        // The var "shared.hp" creates namespace alias: hp -> shared.hp
        // Adding module-scoped alias: hp -> shared.mp causes conflict
        let module_alias_directives_by_namespace = BTreeMap::from([(
            "shared".to_string(),
            vec![AliasDirective {
                target_qualified_name: "shared.mp".to_string(),
                alias_name: "hp".to_string(),
            }],
        )]);

        let conflict = resolve_visible_module_symbols_with_aliases_and_module_scoped_type_aliases(
            &reachable,
            &modules,
            Some("main"),
            &[],
            &module_alias_directives_by_namespace,
        )
        .expect_err("module-scoped alias conflicting with namespace alias should fail");
        assert_eq!(conflict.code, "ALIAS_NAME_CONFLICT");
    }

    #[test]
    fn module_scoped_duplicate_alias_same_target() {
        // Test line 467: duplicate alias pointing to same target should be skipped (continue)
        let span = SourceSpan::synthetic();
        let shared_module = ModuleDeclarations {
            type_decls: vec![],
            function_decls: vec![],
            module_global_var_decls: vec![ParsedModuleVarDecl {
                namespace: "shared".to_string(),
                name: "hp".to_string(),
                qualified_name: "shared.hp".to_string(),
                access: AccessLevel::Public,
                type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                initial_value_expr: None,
                location: span.clone(),
            }],
            module_global_const_decls: vec![],
        };

        let modules = BTreeMap::from([("shared.xml".to_string(), shared_module)]);
        let reachable = BTreeSet::from(["shared.xml".to_string()]);

        // Two aliases with same name "hp" pointing to same target "shared.hp"
        // This should NOT fail - it should be skipped via continue at line 467
        let module_alias_directives_by_namespace = BTreeMap::from([(
            "shared".to_string(),
            vec![
                AliasDirective {
                    target_qualified_name: "shared.hp".to_string(),
                    alias_name: "stat".to_string(),
                },
                AliasDirective {
                    target_qualified_name: "shared.hp".to_string(),
                    alias_name: "stat".to_string(),
                },
            ],
        )]);

        // This should succeed - duplicate alias to same target is allowed
        let result = resolve_visible_module_symbols_with_aliases_and_module_scoped_type_aliases(
            &reachable,
            &modules,
            Some("main"),
            &[],
            &module_alias_directives_by_namespace,
        );
        assert!(
            result.is_ok(),
            "duplicate alias to same target should be allowed"
        );
    }

    #[test]
    fn namespace_merge_duplicate_alias_same_target() {
        // Test line 496: merge_namespace_module_symbol_aliases duplicate alias to same target
        // When module-scoped explicit alias points to same target as namespace alias, skip it
        let span = SourceSpan::synthetic();
        let shared_module = ModuleDeclarations {
            type_decls: vec![],
            function_decls: vec![],
            module_global_var_decls: vec![ParsedModuleVarDecl {
                namespace: "shared".to_string(),
                name: "hp".to_string(),
                qualified_name: "shared.hp".to_string(),
                access: AccessLevel::Public,
                type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                initial_value_expr: None,
                location: span.clone(),
            }],
            module_global_const_decls: vec![],
        };

        let modules = BTreeMap::from([("shared.xml".to_string(), shared_module)]);
        let reachable = BTreeSet::from(["shared.xml".to_string()]);

        // Module-scoped alias "hp" pointing to same target "shared.hp"
        // This should NOT fail - it should be skipped via continue at line 496
        let module_alias_directives_by_namespace = BTreeMap::from([(
            "shared".to_string(),
            vec![AliasDirective {
                target_qualified_name: "shared.hp".to_string(),
                alias_name: "hp".to_string(),
            }],
        )]);

        // This should succeed - alias pointing to same target is allowed
        let result = resolve_visible_module_symbols_with_aliases_and_module_scoped_type_aliases(
            &reachable,
            &modules,
            Some("main"),
            &[],
            &module_alias_directives_by_namespace,
        );
        assert!(
            result.is_ok(),
            "module-scoped alias to same target as namespace alias should be allowed"
        );
    }

    #[test]
    fn module_scoped_type_alias_conflict() {
        // Test line 1204: collect_module_explicit_visible_type_aliases error propagation
        // When module-scoped type aliases have conflicts, it should return error
        let span = SourceSpan::synthetic();
        let shared_module = ModuleDeclarations {
            type_decls: vec![
                ParsedTypeDecl {
                    name: "Unit".to_string(),
                    qualified_name: "shared.Unit".to_string(),
                    access: AccessLevel::Public,
                    fields: vec![],
                    enum_members: Vec::new(),
                    location: span.clone(),
                },
                ParsedTypeDecl {
                    name: "OtherUnit".to_string(),
                    qualified_name: "shared.OtherUnit".to_string(),
                    access: AccessLevel::Public,
                    fields: vec![],
                    enum_members: Vec::new(),
                    location: span.clone(),
                },
            ],
            function_decls: vec![],
            module_global_var_decls: vec![],
            module_global_const_decls: vec![],
        };

        let modules = BTreeMap::from([("shared.xml".to_string(), shared_module)]);
        let reachable = BTreeSet::from(["shared.xml".to_string()]);

        // Two type aliases with same name "Hero" pointing to different targets
        let module_alias_directives_by_namespace = BTreeMap::from([(
            "shared".to_string(),
            vec![
                AliasDirective {
                    target_qualified_name: "shared.Unit".to_string(),
                    alias_name: "Hero".to_string(),
                },
                AliasDirective {
                    target_qualified_name: "shared.OtherUnit".to_string(),
                    alias_name: "Hero".to_string(),
                },
            ],
        )]);

        let conflict = resolve_visible_module_symbols_with_aliases_and_module_scoped_type_aliases(
            &reachable,
            &modules,
            Some("main"),
            &[],
            &module_alias_directives_by_namespace,
        )
        .expect_err("module-scoped type alias conflict should fail");
        assert_eq!(conflict.code, "ALIAS_NAME_CONFLICT");
    }

    #[test]
    fn alias_conflicts_with_existing_visible_type() {
        // Test line 1291: alias name conflicts with existing visible type
        // The conflict check uses qualified names as keys, so we need to use qualified name as alias
        let span = SourceSpan::synthetic();
        let shared_module = ModuleDeclarations {
            type_decls: vec![
                ParsedTypeDecl {
                    name: "Unit".to_string(),
                    qualified_name: "shared.Unit".to_string(),
                    access: AccessLevel::Public,
                    fields: vec![],
                    enum_members: Vec::new(),
                    location: span.clone(),
                },
                ParsedTypeDecl {
                    name: "Hero".to_string(),
                    qualified_name: "shared.Hero".to_string(),
                    access: AccessLevel::Public,
                    fields: vec![],
                    enum_members: Vec::new(),
                    location: span.clone(),
                },
            ],
            function_decls: vec![],
            module_global_var_decls: vec![],
            module_global_const_decls: vec![],
        };

        let modules = BTreeMap::from([("shared.xml".to_string(), shared_module)]);
        let reachable = BTreeSet::from(["shared.xml".to_string()]);

        // Alias name "shared.Hero" conflicts with existing visible type "shared.Hero"
        // (using qualified name as alias to match the key format in visible_types)
        let conflict = resolve_visible_module_symbols_with_aliases(
            &reachable,
            &modules,
            Some("main"),
            &[AliasDirective {
                target_qualified_name: "shared.Unit".to_string(),
                alias_name: "shared.Hero".to_string(),
            }],
        )
        .expect_err("alias name conflicts with existing type should fail");
        assert_eq!(conflict.code, "ALIAS_NAME_CONFLICT");
    }

    #[test]
    fn alias_conflicts_with_existing_module_const() {
        // Test line 1321: alias name conflicts with existing visible module constant
        let span = SourceSpan::synthetic();
        let shared_module = ModuleDeclarations {
            type_decls: vec![],
            function_decls: vec![],
            module_global_var_decls: vec![],
            module_global_const_decls: vec![ParsedModuleConstDecl {
                namespace: "shared".to_string(),
                name: "BASE".to_string(),
                qualified_name: "shared.BASE".to_string(),
                access: AccessLevel::Public,
                type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                initial_value_expr: Some("10".to_string()),
                location: span.clone(),
            }],
        };

        let modules = BTreeMap::from([("shared.xml".to_string(), shared_module)]);
        let reachable = BTreeSet::from(["shared.xml".to_string()]);

        // Alias name "shared.BASE" conflicts with existing module const "shared.BASE"
        let conflict = resolve_visible_module_symbols_with_aliases(
            &reachable,
            &modules,
            Some("main"),
            &[AliasDirective {
                target_qualified_name: "shared.BASE".to_string(),
                alias_name: "shared.BASE".to_string(),
            }],
        )
        .expect_err("alias name conflicts with existing const should fail");
        assert_eq!(conflict.code, "ALIAS_NAME_CONFLICT");
    }
}
