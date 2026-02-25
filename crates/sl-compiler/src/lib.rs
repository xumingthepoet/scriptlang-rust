use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use regex::Regex;
use serde_json::Value as JsonValue;
use sl_core::{
    default_value_from_type, CallArgument, ChoiceOption, ContinueTarget, FunctionDecl,
    FunctionParam, FunctionReturn, ImplicitGroup, ScriptIr, ScriptLangError, ScriptNode,
    ScriptParam, ScriptType, SlValue, SourceSpan, VarDeclaration,
};
use sl_parser::{
    parse_include_directives, parse_xml_document, XmlElementNode, XmlNode, XmlTextNode,
};

pub const INTERNAL_RESERVED_NAME_PREFIX: &str = "__";
const LOOP_TEMP_VAR_PREFIX: &str = "__sl_loop_";

#[derive(Debug, Clone)]
pub struct CompileProjectBundleResult {
    pub scripts: BTreeMap<String, ScriptIr>,
    pub global_json: BTreeMap<String, SlValue>,
}

#[derive(Debug, Clone)]
enum SourceKind {
    ScriptXml,
    DefsXml,
    Json,
}

#[derive(Debug, Clone)]
struct SourceFile {
    kind: SourceKind,
    includes: Vec<String>,
    xml_root: Option<XmlElementNode>,
    json_value: Option<SlValue>,
}

#[derive(Debug, Clone)]
struct ParsedTypeDecl {
    name: String,
    qualified_name: String,
    fields: Vec<ParsedTypeFieldDecl>,
    location: SourceSpan,
}

#[derive(Debug, Clone)]
struct ParsedTypeFieldDecl {
    name: String,
    type_expr: ParsedTypeExpr,
    location: SourceSpan,
}

#[derive(Debug, Clone)]
struct ParsedFunctionDecl {
    name: String,
    qualified_name: String,
    params: Vec<ParsedFunctionParamDecl>,
    return_binding: ParsedFunctionParamDecl,
    code: String,
    location: SourceSpan,
}

#[derive(Debug, Clone)]
struct ParsedFunctionParamDecl {
    name: String,
    type_expr: ParsedTypeExpr,
    location: SourceSpan,
}

#[derive(Debug, Clone)]
enum ParsedTypeExpr {
    Primitive(String),
    Array(Box<ParsedTypeExpr>),
    Map(Box<ParsedTypeExpr>),
    Custom(String),
}

#[derive(Debug, Clone)]
struct DefsDeclarations {
    type_decls: Vec<ParsedTypeDecl>,
    function_decls: Vec<ParsedFunctionDecl>,
}

type VisibleTypeMap = BTreeMap<String, ScriptType>;
type VisibleFunctionMap = BTreeMap<String, FunctionDecl>;

#[derive(Debug, Clone)]
struct MacroExpansionContext {
    used_var_names: BTreeSet<String>,
    loop_counter: usize,
}

#[derive(Debug, Clone)]
struct GroupBuilder {
    script_path: String,
    group_counter: usize,
    node_counter: usize,
    choice_counter: usize,
    groups: BTreeMap<String, ImplicitGroup>,
}

impl GroupBuilder {
    fn new(script_path: impl Into<String>) -> Self {
        Self {
            script_path: script_path.into(),
            group_counter: 0,
            node_counter: 0,
            choice_counter: 0,
            groups: BTreeMap::new(),
        }
    }

    fn next_group_id(&mut self) -> String {
        let id = format!(
            "{}::g{}",
            stable_base(&self.script_path),
            self.group_counter
        );
        self.group_counter += 1;
        id
    }

    fn next_node_id(&mut self, kind: &str) -> String {
        let id = format!(
            "{}::n{}:{}",
            stable_base(&self.script_path),
            self.node_counter,
            kind
        );
        self.node_counter += 1;
        id
    }

    fn next_choice_id(&mut self) -> String {
        let id = format!(
            "{}::c{}",
            stable_base(&self.script_path),
            self.choice_counter
        );
        self.choice_counter += 1;
        id
    }
}

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

fn parse_sources(
    xml_by_path: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, SourceFile>, ScriptLangError> {
    let mut sources = BTreeMap::new();

    for (raw_path, source_text) in xml_by_path {
        let file_path = normalize_virtual_path(raw_path);
        let kind = detect_source_kind(&file_path).ok_or_else(|| {
            ScriptLangError::new(
                "SOURCE_KIND_UNSUPPORTED",
                format!("Unsupported source extension: {}", file_path),
            )
        })?;

        let source = match kind {
            SourceKind::Json => {
                let parsed = serde_json::from_str::<JsonValue>(source_text).map_err(|error| {
                    ScriptLangError::new(
                        "JSON_PARSE_ERROR",
                        format!("Failed to parse JSON include \"{}\": {}", file_path, error),
                    )
                })?;

                SourceFile {
                    kind,
                    includes: Vec::new(),
                    xml_root: None,
                    json_value: Some(slvalue_from_json(parsed)),
                }
            }
            SourceKind::ScriptXml | SourceKind::DefsXml => {
                let document = parse_xml_document(source_text)?;
                let includes = parse_include_directives(source_text)
                    .into_iter()
                    .map(|include| resolve_include_path(&file_path, &include))
                    .collect::<Vec<_>>();

                SourceFile {
                    kind,
                    includes,
                    xml_root: Some(document.root),
                    json_value: None,
                }
            }
        };

        sources.insert(file_path, source);
    }

    Ok(sources)
}

fn detect_source_kind(path: &str) -> Option<SourceKind> {
    if path.ends_with(".script.xml") {
        Some(SourceKind::ScriptXml)
    } else if path.ends_with(".defs.xml") {
        Some(SourceKind::DefsXml)
    } else if path.ends_with(".json") {
        Some(SourceKind::Json)
    } else {
        None
    }
}

fn resolve_include_path(current_path: &str, include: &str) -> String {
    let parent = match Path::new(current_path).parent() {
        Some(parent) => parent,
        None => Path::new(""),
    };
    let joined = if include.starts_with('/') {
        PathBuf::from(include)
    } else {
        parent.join(include)
    };
    normalize_virtual_path(joined.to_string_lossy().as_ref())
}

fn normalize_virtual_path(path: &str) -> String {
    let mut stack: Vec<String> = Vec::new();
    for part in path.replace('\\', "/").split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            if !stack.is_empty() {
                stack.pop();
            }
            continue;
        }
        stack.push(part.to_string());
    }
    stack.join("/")
}

fn validate_include_graph(sources: &BTreeMap<String, SourceFile>) -> Result<(), ScriptLangError> {
    for (file_path, source) in sources {
        for include in &source.includes {
            if !sources.contains_key(include) {
                return Err(ScriptLangError::new(
                    "INCLUDE_NOT_FOUND",
                    format!(
                        "Include \"{}\" referenced by \"{}\" not found.",
                        include, file_path
                    ),
                ));
            }
        }
    }

    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    enum State {
        Visiting,
        Done,
    }

    fn dfs(
        node: &str,
        sources: &BTreeMap<String, SourceFile>,
        states: &mut HashMap<String, State>,
        stack: &mut Vec<String>,
    ) -> Result<(), ScriptLangError> {
        if let Some(state) = states.get(node) {
            if *state == State::Visiting {
                stack.push(node.to_string());
                let cycle = stack.join(" -> ");
                return Err(ScriptLangError::new(
                    "INCLUDE_CYCLE",
                    format!("Include cycle detected: {}", cycle),
                ));
            }
            return Ok(());
        }

        states.insert(node.to_string(), State::Visiting);
        stack.push(node.to_string());

        let source = sources
            .get(node)
            .expect("include graph nodes should exist after validation");
        for include in &source.includes {
            dfs(include, sources, states, stack)?;
        }

        stack.pop();
        states.insert(node.to_string(), State::Done);
        Ok(())
    }

    let mut states: HashMap<String, State> = HashMap::new();
    for file_path in sources.keys() {
        dfs(file_path, sources, &mut states, &mut Vec::new())?;
    }

    Ok(())
}

fn collect_reachable_files(
    start: &str,
    sources: &BTreeMap<String, SourceFile>,
) -> BTreeSet<String> {
    let mut visited = BTreeSet::new();
    let mut stack = vec![start.to_string()];

    while let Some(path) = stack.pop() {
        if !visited.insert(path.clone()) {
            continue;
        }
        if let Some(source) = sources.get(&path) {
            for include in &source.includes {
                stack.push(include.clone());
            }
        }
    }

    visited
}

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

#[cfg(test)]
fn resolve_named_type(
    name: &str,
    type_decls_map: &BTreeMap<String, ParsedTypeDecl>,
    resolved: &mut BTreeMap<String, ScriptType>,
    visiting: &mut HashSet<String>,
) -> Result<ScriptType, ScriptLangError> {
    let empty_aliases = BTreeMap::new();
    resolve_named_type_with_aliases(name, type_decls_map, &empty_aliases, resolved, visiting)
}

fn resolve_named_type_with_aliases(
    name: &str,
    type_decls_map: &BTreeMap<String, ParsedTypeDecl>,
    type_aliases: &BTreeMap<String, String>,
    resolved: &mut BTreeMap<String, ScriptType>,
    visiting: &mut HashSet<String>,
) -> Result<ScriptType, ScriptLangError> {
    let lookup_name = if type_decls_map.contains_key(name) {
        name.to_string()
    } else if let Some(qualified) = type_aliases.get(name) {
        qualified.clone()
    } else {
        return Err(ScriptLangError::new(
            "TYPE_UNKNOWN",
            format!("Unknown type \"{}\".", name),
        ));
    };

    if let Some(found) = resolved.get(&lookup_name) {
        return Ok(found.clone());
    }

    if !visiting.insert(lookup_name.clone()) {
        return Err(ScriptLangError::new(
            "TYPE_DECL_RECURSIVE",
            format!("Recursive type declaration detected for \"{}\".", name),
        ));
    }

    let Some(decl) = type_decls_map.get(&lookup_name) else {
        visiting.remove(&lookup_name);
        return Err(ScriptLangError::new(
            "TYPE_UNKNOWN",
            format!("Unknown type \"{}\".", name),
        ));
    };

    let mut fields = BTreeMap::new();
    for field in &decl.fields {
        if fields.contains_key(&field.name) {
            visiting.remove(&lookup_name);
            return Err(ScriptLangError::with_span(
                "TYPE_FIELD_DUPLICATE",
                format!("Duplicate field \"{}\" in type \"{}\".", field.name, name),
                field.location.clone(),
            ));
        }
        let field_type = resolve_type_expr_with_lookup_with_aliases(
            &field.type_expr,
            type_decls_map,
            type_aliases,
            resolved,
            visiting,
            &field.location,
        )?;
        fields.insert(field.name.clone(), field_type);
    }

    visiting.remove(&lookup_name);

    let resolved_type = ScriptType::Object {
        type_name: lookup_name.clone(),
        fields,
    };
    resolved.insert(lookup_name, resolved_type.clone());
    Ok(resolved_type)
}

#[cfg(test)]
fn resolve_type_expr_with_lookup(
    expr: &ParsedTypeExpr,
    type_decls_map: &BTreeMap<String, ParsedTypeDecl>,
    resolved: &mut BTreeMap<String, ScriptType>,
    visiting: &mut HashSet<String>,
    span: &SourceSpan,
) -> Result<ScriptType, ScriptLangError> {
    let empty_aliases = BTreeMap::new();
    resolve_type_expr_with_lookup_with_aliases(
        expr,
        type_decls_map,
        &empty_aliases,
        resolved,
        visiting,
        span,
    )
}

fn resolve_type_expr_with_lookup_with_aliases(
    expr: &ParsedTypeExpr,
    type_decls_map: &BTreeMap<String, ParsedTypeDecl>,
    type_aliases: &BTreeMap<String, String>,
    resolved: &mut BTreeMap<String, ScriptType>,
    visiting: &mut HashSet<String>,
    span: &SourceSpan,
) -> Result<ScriptType, ScriptLangError> {
    match expr {
        ParsedTypeExpr::Primitive(name) => Ok(ScriptType::Primitive { name: name.clone() }),
        ParsedTypeExpr::Array(element_type) => {
            let resolved_element = resolve_type_expr_with_lookup_with_aliases(
                element_type,
                type_decls_map,
                type_aliases,
                resolved,
                visiting,
                span,
            )?;
            Ok(ScriptType::Array {
                element_type: Box::new(resolved_element),
            })
        }
        ParsedTypeExpr::Map(value_type) => {
            let resolved_value = resolve_type_expr_with_lookup_with_aliases(
                value_type,
                type_decls_map,
                type_aliases,
                resolved,
                visiting,
                span,
            )?;
            Ok(ScriptType::Map {
                key_type: "string".to_string(),
                value_type: Box::new(resolved_value),
            })
        }
        ParsedTypeExpr::Custom(name) => {
            match resolve_named_type_with_aliases(
                name,
                type_decls_map,
                type_aliases,
                resolved,
                visiting,
            ) {
                Ok(value) => Ok(value),
                Err(_) => Err(ScriptLangError::with_span(
                    "TYPE_UNKNOWN",
                    format!("Unknown custom type \"{}\".", name),
                    span.clone(),
                )),
            }
        }
    }
}

fn resolve_type_expr(
    expr: &ParsedTypeExpr,
    resolved_types: &BTreeMap<String, ScriptType>,
    span: &SourceSpan,
) -> Result<ScriptType, ScriptLangError> {
    match expr {
        ParsedTypeExpr::Primitive(name) => Ok(ScriptType::Primitive { name: name.clone() }),
        ParsedTypeExpr::Array(element_type) => Ok(ScriptType::Array {
            element_type: Box::new(resolve_type_expr(element_type, resolved_types, span)?),
        }),
        ParsedTypeExpr::Map(value_type) => Ok(ScriptType::Map {
            key_type: "string".to_string(),
            value_type: Box::new(resolve_type_expr(value_type, resolved_types, span)?),
        }),
        ParsedTypeExpr::Custom(name) => match resolved_types.get(name).cloned() {
            Some(value) => Ok(value),
            None => Err(ScriptLangError::with_span(
                "TYPE_UNKNOWN",
                format!("Unknown custom type \"{}\".", name),
                span.clone(),
            )),
        },
    }
}

#[cfg(test)]
fn parse_type_declaration_node(node: &XmlElementNode) -> Result<ParsedTypeDecl, ScriptLangError> {
    parse_type_declaration_node_with_namespace(node, "defs")
}

fn parse_type_declaration_node_with_namespace(
    node: &XmlElementNode,
    namespace: &str,
) -> Result<ParsedTypeDecl, ScriptLangError> {
    let name = get_required_non_empty_attr(node, "name")?;
    assert_name_not_reserved(&name, "type", node.location.clone())?;

    let mut fields = Vec::new();
    let mut seen = HashSet::new();

    for child in element_children(node) {
        if child.name != "field" {
            return Err(ScriptLangError::with_span(
                "XML_TYPE_CHILD_INVALID",
                format!("Unsupported child <{}> under <type>.", child.name),
                child.location.clone(),
            ));
        }

        let field_name = get_required_non_empty_attr(child, "name")?;
        assert_name_not_reserved(&field_name, "type field", child.location.clone())?;
        if !seen.insert(field_name.clone()) {
            return Err(ScriptLangError::with_span(
                "TYPE_FIELD_DUPLICATE",
                format!("Duplicate field \"{}\" in type \"{}\".", field_name, name),
                child.location.clone(),
            ));
        }

        let field_type_raw = get_required_non_empty_attr(child, "type")?;
        let field_type = parse_type_expr(&field_type_raw, &child.location)?;
        fields.push(ParsedTypeFieldDecl {
            name: field_name,
            type_expr: field_type,
            location: child.location.clone(),
        });
    }

    let qualified_name = format!("{}.{}", namespace, name);
    Ok(ParsedTypeDecl {
        name,
        qualified_name,
        fields,
        location: node.location.clone(),
    })
}

#[cfg(test)]
fn parse_function_declaration_node(
    node: &XmlElementNode,
) -> Result<ParsedFunctionDecl, ScriptLangError> {
    parse_function_declaration_node_with_namespace(node, "defs")
}

fn parse_function_declaration_node_with_namespace(
    node: &XmlElementNode,
    namespace: &str,
) -> Result<ParsedFunctionDecl, ScriptLangError> {
    let name = get_required_non_empty_attr(node, "name")?;
    assert_name_not_reserved(&name, "function", node.location.clone())?;

    let params = parse_function_args(node)?;
    let return_binding = parse_function_return(node)?;
    let code = parse_inline_required_no_element_children(node)?;

    let qualified_name = format!("{}.{}", namespace, name);
    Ok(ParsedFunctionDecl {
        name,
        qualified_name,
        params,
        return_binding,
        code,
        location: node.location.clone(),
    })
}

fn compile_script(
    script_path: &str,
    root: &XmlElementNode,
    visible_types: &BTreeMap<String, ScriptType>,
    visible_functions: &BTreeMap<String, FunctionDecl>,
    visible_json_globals: &[String],
    all_json_symbols: &BTreeSet<String>,
) -> Result<ScriptIr, ScriptLangError> {
    if root.name != "script" {
        return Err(ScriptLangError::with_span(
            "XML_ROOT_INVALID",
            "Script file root must be <script>.",
            root.location.clone(),
        ));
    }

    let script_name = get_required_non_empty_attr(root, "name")?;
    assert_name_not_reserved(&script_name, "script", root.location.clone())?;

    let params = parse_script_args(root, visible_types)?;
    validate_reserved_prefix_in_user_var_declarations(root)?;

    let mut reserved_names = params
        .iter()
        .map(|param| param.name.clone())
        .collect::<Vec<_>>();
    reserved_names.sort();

    let expanded_root = expand_script_macros(root, &reserved_names)?;

    let mut builder = GroupBuilder::new(script_path);
    let root_group_id = builder.next_group_id();

    let mut visible_var_types = BTreeMap::new();
    for param in &params {
        visible_var_types.insert(param.name.clone(), param.r#type.clone());
    }

    compile_group(
        &root_group_id,
        None,
        &expanded_root,
        &mut builder,
        visible_types,
        &visible_var_types,
        CompileGroupMode::new(0, false),
    )?;

    let ir = ScriptIr {
        script_path: script_path.to_string(),
        script_name,
        params,
        root_group_id,
        groups: builder.groups,
        visible_json_globals: visible_json_globals.to_vec(),
        visible_functions: visible_functions.clone(),
    };

    validate_json_symbol_visibility(&ir, all_json_symbols)?;
    Ok(ir)
}

fn validate_json_symbol_visibility(
    script_ir: &ScriptIr,
    all_json_symbols: &BTreeSet<String>,
) -> Result<(), ScriptLangError> {
    let visible_json = script_ir
        .visible_json_globals
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let hidden_json = all_json_symbols
        .difference(&visible_json)
        .cloned()
        .collect::<BTreeSet<_>>();

    if hidden_json.is_empty() {
        return Ok(());
    }

    let function_roots = collect_visible_function_roots(&script_ir.visible_functions);
    let mut script_allowed = collect_script_declared_names(script_ir);
    script_allowed.extend(function_roots.iter().cloned());
    script_allowed.insert("random".to_string());

    for group in script_ir.groups.values() {
        for node in &group.nodes {
            match node {
                ScriptNode::Text {
                    value, location, ..
                } => {
                    for expr in extract_text_interpolations(value) {
                        ensure_no_hidden_json_symbol(
                            &expr,
                            &hidden_json,
                            &script_allowed,
                            &script_ir.script_name,
                            "text interpolation",
                            location,
                        )?;
                    }
                }
                ScriptNode::Code { code, location, .. } => {
                    ensure_no_hidden_json_symbol(
                        code,
                        &hidden_json,
                        &script_allowed,
                        &script_ir.script_name,
                        "code block",
                        location,
                    )?;
                }
                ScriptNode::Var { declaration, .. } => {
                    if let Some(expr) = &declaration.initial_value_expr {
                        ensure_no_hidden_json_symbol(
                            expr,
                            &hidden_json,
                            &script_allowed,
                            &script_ir.script_name,
                            "var initializer",
                            &declaration.location,
                        )?;
                    }
                }
                ScriptNode::If {
                    when_expr,
                    location,
                    ..
                } => {
                    ensure_no_hidden_json_symbol(
                        when_expr,
                        &hidden_json,
                        &script_allowed,
                        &script_ir.script_name,
                        "if condition",
                        location,
                    )?;
                }
                ScriptNode::While {
                    when_expr,
                    location,
                    ..
                } => {
                    ensure_no_hidden_json_symbol(
                        when_expr,
                        &hidden_json,
                        &script_allowed,
                        &script_ir.script_name,
                        "while condition",
                        location,
                    )?;
                }
                ScriptNode::Choice { options, .. } => {
                    for option in options {
                        if let Some(when_expr) = &option.when_expr {
                            ensure_no_hidden_json_symbol(
                                when_expr,
                                &hidden_json,
                                &script_allowed,
                                &script_ir.script_name,
                                "choice option condition",
                                &option.location,
                            )?;
                        }
                    }
                }
                ScriptNode::Call { args, location, .. }
                | ScriptNode::Return { args, location, .. } => {
                    for arg in args {
                        ensure_no_hidden_json_symbol(
                            &arg.value_expr,
                            &hidden_json,
                            &script_allowed,
                            &script_ir.script_name,
                            "call/return argument",
                            location,
                        )?;
                    }
                }
                ScriptNode::Input { .. }
                | ScriptNode::Break { .. }
                | ScriptNode::Continue { .. } => {}
            }
        }
    }

    for (name, decl) in &script_ir.visible_functions {
        if !name.contains('.') {
            continue;
        }

        let mut function_allowed = BTreeSet::new();
        function_allowed.extend(function_roots.iter().cloned());
        function_allowed.insert("random".to_string());
        function_allowed.insert(decl.return_binding.name.clone());
        for param in &decl.params {
            function_allowed.insert(param.name.clone());
        }
        function_allowed.extend(extract_local_bindings(&decl.code));

        ensure_no_hidden_json_symbol(
            &decl.code,
            &hidden_json,
            &function_allowed,
            &script_ir.script_name,
            "defs function code",
            &decl.location,
        )?;
    }

    Ok(())
}

fn collect_script_declared_names(script_ir: &ScriptIr) -> BTreeSet<String> {
    let mut out = script_ir
        .params
        .iter()
        .map(|param| param.name.clone())
        .collect::<BTreeSet<_>>();

    for group in script_ir.groups.values() {
        for node in &group.nodes {
            if let ScriptNode::Var { declaration, .. } = node {
                out.insert(declaration.name.clone());
            }
        }
    }
    out
}

fn collect_visible_function_roots(
    visible_functions: &BTreeMap<String, FunctionDecl>,
) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for name in visible_functions.keys() {
        let root = name.split('.').next().unwrap_or(name);
        out.insert(root.to_string());
    }
    out
}

fn extract_text_interpolations(template: &str) -> Vec<String> {
    let interpolation_regex = Regex::new(r"\$\{([^{}]+)\}").expect("template regex must compile");
    interpolation_regex
        .captures_iter(template)
        .filter_map(|caps| caps.get(1).map(|entry| entry.as_str().trim().to_string()))
        .filter(|expr| !expr.is_empty())
        .collect()
}

fn extract_local_bindings(source: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let re = Regex::new(r"\b(?:let|const)\s+([$A-Za-z_][$0-9A-Za-z_]*)").expect("local bind regex");
    for caps in re.captures_iter(source) {
        if let Some(name) = caps.get(1) {
            out.insert(name.as_str().to_string());
        }
    }
    out
}

fn ensure_no_hidden_json_symbol(
    source: &str,
    hidden_json: &BTreeSet<String>,
    allowed_names: &BTreeSet<String>,
    script_name: &str,
    context: &str,
    location: &SourceSpan,
) -> Result<(), ScriptLangError> {
    if let Some(symbol) = find_hidden_json_symbol(source, hidden_json, allowed_names) {
        return Err(ScriptLangError::with_span(
            "JSON_SYMBOL_NOT_VISIBLE",
            format!(
                "JSON symbol \"{}\" is referenced in {} of script \"{}\" but is not visible via include.",
                symbol, context, script_name
            ),
            location.clone(),
        ));
    }
    Ok(())
}

fn find_hidden_json_symbol(
    source: &str,
    hidden_json: &BTreeSet<String>,
    allowed_names: &BTreeSet<String>,
) -> Option<String> {
    let sanitized = sanitize_rhai_source(source);
    hidden_json
        .iter()
        .filter(|symbol| !allowed_names.contains(*symbol))
        .find(|symbol| contains_root_identifier(&sanitized, symbol))
        .cloned()
}

fn sanitize_rhai_source(source: &str) -> String {
    let line_comment_re = Regex::new(r"//[^\n]*").expect("line comment regex");
    let block_comment_re = Regex::new(r"(?s)/\*.*?\*/").expect("block comment regex");
    let double_quote_re = Regex::new(r#""(?:\\.|[^"\\])*""#).expect("double quote regex");
    let single_quote_re = Regex::new(r#"'(?:\\.|[^'\\])*'"#).expect("single quote regex");

    let without_line_comments = line_comment_re.replace_all(source, " ");
    let without_block_comments = block_comment_re.replace_all(&without_line_comments, " ");
    let without_double_quotes = double_quote_re.replace_all(&without_block_comments, " ");
    single_quote_re
        .replace_all(&without_double_quotes, " ")
        .into_owned()
}

fn contains_root_identifier(source: &str, symbol: &str) -> bool {
    let pattern = format!(
        r"(?m)(^|[^.$0-9A-Za-z_]){}([^:$0-9A-Za-z_]|$)",
        regex::escape(symbol)
    );
    Regex::new(&pattern)
        .expect("root identifier regex must compile")
        .is_match(source)
}

#[derive(Debug, Clone, Copy)]
struct CompileGroupMode {
    while_depth: usize,
    allow_option_direct_continue: bool,
}

impl CompileGroupMode {
    fn new(while_depth: usize, allow_option_direct_continue: bool) -> Self {
        Self {
            while_depth,
            allow_option_direct_continue,
        }
    }
}

fn compile_group(
    group_id: &str,
    parent_group_id: Option<&str>,
    container: &XmlElementNode,
    builder: &mut GroupBuilder,
    visible_types: &BTreeMap<String, ScriptType>,
    visible_var_types: &BTreeMap<String, ScriptType>,
    mode: CompileGroupMode,
) -> Result<(), ScriptLangError> {
    let mut local_var_types = visible_var_types.clone();
    let mut nodes = Vec::new();

    macro_rules! compile_child_group {
        ($child_group_id:expr, $child_container:expr, $child_while_depth:expr, $allow_continue:expr) => {
            compile_group(
                $child_group_id,
                Some(group_id),
                $child_container,
                builder,
                visible_types,
                &local_var_types,
                CompileGroupMode::new($child_while_depth, $allow_continue),
            )
        };
    }

    builder.groups.insert(
        group_id.to_string(),
        ImplicitGroup {
            group_id: group_id.to_string(),
            parent_group_id: parent_group_id.map(|value| value.to_string()),
            entry_node_id: None,
            nodes: Vec::new(),
        },
    );

    for child in element_children(container) {
        if has_attr(child, "once") && child.name != "text" {
            return Err(ScriptLangError::with_span(
                "XML_ATTR_NOT_ALLOWED",
                "Attribute \"once\" is only allowed on <text> and <option>.",
                child.location.clone(),
            ));
        }

        let node = match child.name.as_str() {
            "var" => {
                let declaration = parse_var_declaration(child, visible_types)?;
                local_var_types.insert(declaration.name.clone(), declaration.r#type.clone());
                ScriptNode::Var {
                    id: builder.next_node_id("var"),
                    declaration,
                    location: child.location.clone(),
                }
            }
            "text" => ScriptNode::Text {
                id: builder.next_node_id("text"),
                value: parse_inline_required(child)?,
                once: parse_bool_attr(child, "once", false)?,
                location: child.location.clone(),
            },
            "code" => ScriptNode::Code {
                id: builder.next_node_id("code"),
                code: parse_inline_required(child)?,
                location: child.location.clone(),
            },
            "if" => {
                let then_group_id = builder.next_group_id();
                let else_group_id = builder.next_group_id();

                let else_node = element_children(child).find(|candidate| candidate.name == "else");

                let then_container = XmlElementNode {
                    name: child.name.clone(),
                    attributes: child.attributes.clone(),
                    children: child
                        .children
                        .iter()
                        .filter(|entry| {
                            !matches!(entry, XmlNode::Element(element) if element.name == "else")
                        })
                        .cloned()
                        .collect(),
                    location: child.location.clone(),
                };

                compile_child_group!(&then_group_id, &then_container, mode.while_depth, false)?;

                if let Some(else_child) = else_node {
                    compile_child_group!(&else_group_id, else_child, mode.while_depth, false)?;
                } else {
                    builder.groups.insert(
                        else_group_id.clone(),
                        ImplicitGroup {
                            group_id: else_group_id.clone(),
                            parent_group_id: Some(group_id.to_string()),
                            entry_node_id: None,
                            nodes: Vec::new(),
                        },
                    );
                }

                ScriptNode::If {
                    id: builder.next_node_id("if"),
                    when_expr: get_required_non_empty_attr(child, "when")?,
                    then_group_id,
                    else_group_id: Some(else_group_id),
                    location: child.location.clone(),
                }
            }
            "while" => {
                let body_group_id = builder.next_group_id();
                compile_child_group!(&body_group_id, child, mode.while_depth + 1, false)?;
                ScriptNode::While {
                    id: builder.next_node_id("while"),
                    when_expr: get_required_non_empty_attr(child, "when")?,
                    body_group_id,
                    location: child.location.clone(),
                }
            }
            "choice" => {
                let prompt_text = get_required_non_empty_attr(child, "text")?;
                let mut options = Vec::new();
                let mut fall_over_seen = 0usize;

                for option in element_children(child) {
                    if option.name != "option" {
                        return Err(ScriptLangError::with_span(
                            "XML_CHOICE_CHILD_INVALID",
                            format!("Unsupported child <{}> under <choice>.", option.name),
                            option.location.clone(),
                        ));
                    }

                    let once = parse_bool_attr(option, "once", false)?;
                    let fall_over = parse_bool_attr(option, "fall_over", false)?;
                    let when_expr = get_optional_attr(option, "when");
                    if fall_over {
                        fall_over_seen += 1;
                        if when_expr.is_some() {
                            return Err(ScriptLangError::with_span(
                                "XML_OPTION_FALL_OVER_WHEN_FORBIDDEN",
                                "fall_over option cannot declare when.",
                                option.location.clone(),
                            ));
                        }
                    }

                    let option_group_id = builder.next_group_id();
                    compile_child_group!(&option_group_id, option, mode.while_depth, true)?;

                    options.push(ChoiceOption {
                        id: builder.next_choice_id(),
                        text: get_required_non_empty_attr(option, "text")?,
                        when_expr,
                        once,
                        fall_over,
                        group_id: option_group_id,
                        location: option.location.clone(),
                    });
                }

                if fall_over_seen > 1 {
                    return Err(ScriptLangError::with_span(
                        "XML_OPTION_FALL_OVER_DUPLICATE",
                        "At most one fall_over option is allowed per choice.",
                        child.location.clone(),
                    ));
                }

                if let Some(index) = options.iter().position(|option| option.fall_over) {
                    if index != options.len().saturating_sub(1) {
                        return Err(ScriptLangError::with_span(
                            "XML_OPTION_FALL_OVER_NOT_LAST",
                            "fall_over option must be the last option.",
                            child.location.clone(),
                        ));
                    }
                }

                ScriptNode::Choice {
                    id: builder.next_node_id("choice"),
                    prompt_text,
                    options,
                    location: child.location.clone(),
                }
            }
            "input" => {
                if has_attr(child, "default") {
                    return Err(ScriptLangError::with_span(
                        "XML_INPUT_DEFAULT_UNSUPPORTED",
                        "Attribute \"default\" is not supported on <input>.",
                        child.location.clone(),
                    ));
                }
                if has_any_child_content(child) {
                    return Err(ScriptLangError::with_span(
                        "XML_INPUT_CONTENT_FORBIDDEN",
                        "<input> cannot contain child nodes or inline text.",
                        child.location.clone(),
                    ));
                }

                ScriptNode::Input {
                    id: builder.next_node_id("input"),
                    target_var: get_required_non_empty_attr(child, "var")?,
                    prompt_text: get_required_non_empty_attr(child, "text")?,
                    location: child.location.clone(),
                }
            }
            "break" => {
                if mode.while_depth == 0 {
                    return Err(ScriptLangError::with_span(
                        "XML_BREAK_OUTSIDE_WHILE",
                        "<break/> is only valid inside <while>.",
                        child.location.clone(),
                    ));
                }
                ScriptNode::Break {
                    id: builder.next_node_id("break"),
                    location: child.location.clone(),
                }
            }
            "continue" => {
                let target = if mode.while_depth > 0 {
                    ContinueTarget::While
                } else if mode.allow_option_direct_continue {
                    ContinueTarget::Choice
                } else {
                    return Err(ScriptLangError::with_span(
                        "XML_CONTINUE_OUTSIDE_WHILE_OR_OPTION",
                        "<continue/> is only valid inside <while> or as direct child of <option>.",
                        child.location.clone(),
                    ));
                };

                ScriptNode::Continue {
                    id: builder.next_node_id("continue"),
                    target,
                    location: child.location.clone(),
                }
            }
            "call" => ScriptNode::Call {
                id: builder.next_node_id("call"),
                target_script: get_required_non_empty_attr(child, "script")?,
                args: parse_args(get_optional_attr(child, "args"))?,
                location: child.location.clone(),
            },
            "return" => {
                let args = parse_args(get_optional_attr(child, "args"))?;
                if args.iter().any(|arg| arg.is_ref) {
                    return Err(ScriptLangError::with_span(
                        "XML_RETURN_REF_UNSUPPORTED",
                        "Return args do not support ref mode.",
                        child.location.clone(),
                    ));
                }

                let target_script = get_optional_attr(child, "script");
                if !args.is_empty() && target_script.is_none() {
                    return Err(ScriptLangError::with_span(
                        "XML_RETURN_ARGS_REQUIRE_SCRIPT",
                        "Return args require script attribute.",
                        child.location.clone(),
                    ));
                }

                ScriptNode::Return {
                    id: builder.next_node_id("return"),
                    target_script,
                    args,
                    location: child.location.clone(),
                }
            }
            "loop" => {
                return Err(ScriptLangError::with_span(
                    "XML_LOOP_INTERNAL",
                    "<loop> must be expanded before compile phase.",
                    child.location.clone(),
                ))
            }
            "else" => {
                return Err(ScriptLangError::with_span(
                    "XML_ELSE_POSITION",
                    "<else> can only appear inside <if>.",
                    child.location.clone(),
                ))
            }
            removed @ ("vars" | "step" | "set" | "push" | "remove") => {
                return Err(ScriptLangError::with_span(
                    "XML_REMOVED_NODE",
                    format!("<{}> is removed in ScriptLang.", removed),
                    child.location.clone(),
                ))
            }
            _ => {
                return Err(ScriptLangError::with_span(
                    "XML_NODE_UNSUPPORTED",
                    format!("Unsupported node <{}> in <script> body.", child.name),
                    child.location.clone(),
                ))
            }
        };

        let group = builder
            .groups
            .get_mut(group_id)
            .expect("group should exist when compiling nodes");
        if group.entry_node_id.is_none() {
            group.entry_node_id = Some(node_id(&node).to_string());
        }

        nodes.push(node);
    }

    if let Some(group) = builder.groups.get_mut(group_id) {
        group.nodes = nodes;
    }

    Ok(())
}

fn node_id(node: &ScriptNode) -> &str {
    match node {
        ScriptNode::Text { id, .. }
        | ScriptNode::Code { id, .. }
        | ScriptNode::Var { id, .. }
        | ScriptNode::If { id, .. }
        | ScriptNode::While { id, .. }
        | ScriptNode::Choice { id, .. }
        | ScriptNode::Input { id, .. }
        | ScriptNode::Break { id, .. }
        | ScriptNode::Continue { id, .. }
        | ScriptNode::Call { id, .. }
        | ScriptNode::Return { id, .. } => id,
    }
}

fn parse_var_declaration(
    node: &XmlElementNode,
    visible_types: &BTreeMap<String, ScriptType>,
) -> Result<VarDeclaration, ScriptLangError> {
    let name = get_required_non_empty_attr(node, "name")?;

    let type_raw = get_required_non_empty_attr(node, "type")?;
    let ty_expr = parse_type_expr(&type_raw, &node.location)?;
    let ty = resolve_type_expr(&ty_expr, visible_types, &node.location)?;

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

    Ok(VarDeclaration {
        name,
        r#type: ty,
        initial_value_expr,
        location: node.location.clone(),
    })
}

fn parse_script_args(
    root: &XmlElementNode,
    visible_types: &BTreeMap<String, ScriptType>,
) -> Result<Vec<ScriptParam>, ScriptLangError> {
    let Some(raw) = get_optional_attr(root, "args") else {
        return Ok(Vec::new());
    };

    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }

    let segments = split_by_top_level_comma(&raw);
    let mut params = Vec::new();
    let mut names = HashSet::new();

    for segment in segments {
        if segment.is_empty() {
            continue;
        }
        let is_ref = segment.starts_with("ref:");
        let normalized = if is_ref {
            segment.trim_start_matches("ref:").trim()
        } else {
            segment.as_str()
        };

        let Some(separator) = normalized.find(':') else {
            return Err(ScriptLangError::with_span(
                "SCRIPT_ARGS_PARSE_ERROR",
                format!("Invalid script args segment: \"{}\".", segment),
                root.location.clone(),
            ));
        };
        if separator == 0 || separator + 1 >= normalized.len() {
            return Err(ScriptLangError::with_span(
                "SCRIPT_ARGS_PARSE_ERROR",
                format!("Invalid script args segment: \"{}\".", segment),
                root.location.clone(),
            ));
        }

        let type_raw = normalized[..separator].trim();
        let name = normalized[separator + 1..].trim();

        assert_name_not_reserved(name, "script arg", root.location.clone())?;
        if !names.insert(name.to_string()) {
            return Err(ScriptLangError::with_span(
                "SCRIPT_ARGS_DUPLICATE",
                format!("Script arg \"{}\" is declared more than once.", name),
                root.location.clone(),
            ));
        }

        let parsed_type = parse_type_expr(type_raw, &root.location)?;
        let resolved_type = resolve_type_expr(&parsed_type, visible_types, &root.location)?;

        params.push(ScriptParam {
            name: name.to_string(),
            r#type: resolved_type,
            is_ref,
            location: root.location.clone(),
        });
    }

    Ok(params)
}

fn parse_function_args(
    node: &XmlElementNode,
) -> Result<Vec<ParsedFunctionParamDecl>, ScriptLangError> {
    let Some(raw) = get_optional_attr(node, "args") else {
        return Ok(Vec::new());
    };
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }

    let mut params = Vec::new();
    let mut names = HashSet::new();

    for segment in split_by_top_level_comma(&raw) {
        if segment.starts_with("ref:") {
            return Err(ScriptLangError::with_span(
                "XML_FUNCTION_ARGS_REF_UNSUPPORTED",
                format!("Function arg \"{}\" cannot use ref mode.", segment),
                node.location.clone(),
            ));
        }
        let Some(separator) = segment.find(':') else {
            return Err(ScriptLangError::with_span(
                "FUNCTION_ARGS_PARSE_ERROR",
                format!("Invalid function args segment: \"{}\".", segment),
                node.location.clone(),
            ));
        };
        if separator == 0 || separator + 1 >= segment.len() {
            return Err(ScriptLangError::with_span(
                "FUNCTION_ARGS_PARSE_ERROR",
                format!("Invalid function args segment: \"{}\".", segment),
                node.location.clone(),
            ));
        }

        let type_raw = segment[..separator].trim();
        let name = segment[separator + 1..].trim();
        assert_name_not_reserved(name, "function arg", node.location.clone())?;

        if !names.insert(name.to_string()) {
            return Err(ScriptLangError::with_span(
                "FUNCTION_ARGS_DUPLICATE",
                format!("Function arg \"{}\" is declared more than once.", name),
                node.location.clone(),
            ));
        }

        params.push(ParsedFunctionParamDecl {
            name: name.to_string(),
            type_expr: parse_type_expr(type_raw, &node.location)?,
            location: node.location.clone(),
        });
    }

    Ok(params)
}

fn parse_function_return(
    node: &XmlElementNode,
) -> Result<ParsedFunctionParamDecl, ScriptLangError> {
    let raw = get_required_non_empty_attr(node, "return")?;
    if raw.starts_with("ref:") {
        return Err(ScriptLangError::with_span(
            "XML_FUNCTION_RETURN_REF_UNSUPPORTED",
            "Attribute \"return\" on <function> cannot use ref mode.",
            node.location.clone(),
        ));
    }

    let Some(separator) = raw.find(':') else {
        return Err(ScriptLangError::with_span(
            "FUNCTION_RETURN_PARSE_ERROR",
            format!("Invalid function return segment: \"{}\".", raw),
            node.location.clone(),
        ));
    };
    if separator == 0 || separator + 1 >= raw.len() {
        return Err(ScriptLangError::with_span(
            "FUNCTION_RETURN_PARSE_ERROR",
            format!("Invalid function return segment: \"{}\".", raw),
            node.location.clone(),
        ));
    }

    let type_raw = raw[..separator].trim();
    let name = raw[separator + 1..].trim();
    assert_name_not_reserved(name, "function return", node.location.clone())?;

    Ok(ParsedFunctionParamDecl {
        name: name.to_string(),
        type_expr: parse_type_expr(type_raw, &node.location)?,
        location: node.location.clone(),
    })
}

fn parse_type_expr(raw: &str, span: &SourceSpan) -> Result<ParsedTypeExpr, ScriptLangError> {
    let source = raw.trim();
    if source == "int" || source == "float" || source == "string" || source == "boolean" {
        return Ok(ParsedTypeExpr::Primitive(source.to_string()));
    }

    if let Some(stripped) = source.strip_suffix("[]") {
        let element_type = parse_type_expr(stripped, span)?;
        return Ok(ParsedTypeExpr::Array(Box::new(element_type)));
    }

    if let Some(value) = source
        .strip_prefix("#{")
        .and_then(|inner| inner.strip_suffix('}'))
    {
        if value.trim().is_empty() {
            return Err(ScriptLangError::with_span(
                "TYPE_PARSE_ERROR",
                format!("Unsupported type syntax: \"{}\".", raw),
                span.clone(),
            ));
        }
        let value_type = parse_type_expr(value.trim(), span)?;
        return Ok(ParsedTypeExpr::Map(Box::new(value_type)));
    }

    let custom_regex = Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)*$")
        .expect("type regex must compile");
    if custom_regex.is_match(source) {
        return Ok(ParsedTypeExpr::Custom(source.to_string()));
    }

    Err(ScriptLangError::with_span(
        "TYPE_PARSE_ERROR",
        format!("Unsupported type syntax: \"{}\".", raw),
        span.clone(),
    ))
}

fn parse_args(raw: Option<String>) -> Result<Vec<CallArgument>, ScriptLangError> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }

    let mut args = Vec::new();
    for part in split_by_top_level_comma(&raw) {
        let is_ref = part.starts_with("ref:");
        let normalized = if is_ref {
            part.trim_start_matches("ref:").trim()
        } else {
            part.as_str()
        };
        if normalized.is_empty() {
            return Err(ScriptLangError::new(
                "CALL_ARGS_PARSE_ERROR",
                format!("Invalid call arg segment: \"{}\".", part),
            ));
        }

        args.push(CallArgument {
            value_expr: normalized.to_string(),
            is_ref,
        });
    }

    Ok(args)
}

fn parse_inline_required(node: &XmlElementNode) -> Result<String, ScriptLangError> {
    if has_attr(node, "value") {
        return Err(ScriptLangError::with_span(
            "XML_ATTR_NOT_ALLOWED",
            format!(
                "Attribute \"value\" is not allowed on <{}>. Use inline content instead.",
                node.name
            ),
            node.location.clone(),
        ));
    }

    let content = inline_text_content(node);
    if content.trim().is_empty() {
        return Err(ScriptLangError::with_span(
            "XML_EMPTY_NODE_CONTENT",
            format!("<{}> requires non-empty inline content.", node.name),
            node.location.clone(),
        ));
    }

    Ok(content.trim().to_string())
}

fn parse_inline_required_no_element_children(
    node: &XmlElementNode,
) -> Result<String, ScriptLangError> {
    if let Some(element) = element_children(node).next() {
        return Err(ScriptLangError::with_span(
            "XML_FUNCTION_CHILD_NODE_INVALID",
            format!(
                "<{}> cannot contain child elements. Only inline code text is allowed.",
                node.name
            ),
            element.location.clone(),
        ));
    }

    parse_inline_required(node)
}

fn inline_text_content(node: &XmlElementNode) -> String {
    node.children
        .iter()
        .filter_map(|entry| match entry {
            XmlNode::Text(XmlTextNode { value, .. }) => Some(value.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_bool_attr(
    node: &XmlElementNode,
    name: &str,
    default: bool,
) -> Result<bool, ScriptLangError> {
    let Some(value) = get_optional_attr(node, name) else {
        return Ok(default);
    };

    match value.trim() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(ScriptLangError::with_span(
            "XML_ATTR_BOOL_INVALID",
            format!(
                "Attribute \"{}\" on <{}> must be \"true\" or \"false\".",
                name, node.name
            ),
            node.location.clone(),
        )),
    }
}

fn split_by_top_level_comma(raw: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut brace_depth = 0usize;
    let mut quote: Option<char> = None;

    for ch in raw.chars() {
        if let Some(active_quote) = quote {
            current.push(ch);
            if ch == active_quote {
                quote = None;
            }
            continue;
        }

        if ch == '\'' || ch == '"' {
            quote = Some(ch);
            current.push(ch);
            continue;
        }

        match ch {
            '(' => paren_depth += 1,
            ')' if paren_depth > 0 => paren_depth -= 1,
            '[' => bracket_depth += 1,
            ']' if bracket_depth > 0 => bracket_depth -= 1,
            '{' => brace_depth += 1,
            '}' if brace_depth > 0 => brace_depth -= 1,
            ',' if paren_depth == 0 && bracket_depth == 0 && brace_depth == 0 => {
                parts.push(current.trim().to_string());
                current.clear();
                continue;
            }
            _ => {}
        }

        current.push(ch);
    }

    if !current.trim().is_empty() {
        parts.push(current.trim().to_string());
    }

    parts
}

fn assert_name_not_reserved(
    name: &str,
    label: &str,
    span: SourceSpan,
) -> Result<(), ScriptLangError> {
    if !name.trim().starts_with(INTERNAL_RESERVED_NAME_PREFIX) {
        return Ok(());
    }

    Err(ScriptLangError::with_span(
        "NAME_RESERVED_PREFIX",
        format!(
            "Name \"{}\" for {} cannot start with \"{}\" because that prefix is reserved.",
            name, label, INTERNAL_RESERVED_NAME_PREFIX
        ),
        span,
    ))
}

fn element_children(node: &XmlElementNode) -> impl Iterator<Item = &XmlElementNode> {
    node.children.iter().filter_map(|entry| match entry {
        XmlNode::Element(element) => Some(element),
        _ => None,
    })
}

fn has_any_child_content(node: &XmlElementNode) -> bool {
    for entry in &node.children {
        match entry {
            XmlNode::Element(_) => return true,
            XmlNode::Text(text) if !text.value.trim().is_empty() => return true,
            XmlNode::Text(_) => {}
        }
    }
    false
}

fn get_optional_attr(node: &XmlElementNode, name: &str) -> Option<String> {
    node.attributes.get(name).cloned()
}

fn get_required_non_empty_attr(
    node: &XmlElementNode,
    name: &str,
) -> Result<String, ScriptLangError> {
    let Some(raw) = node.attributes.get(name) else {
        return Err(ScriptLangError::with_span(
            "XML_MISSING_ATTR",
            format!(
                "Missing required attribute \"{}\" on <{}>.",
                name, node.name
            ),
            node.location.clone(),
        ));
    };

    if raw.trim().is_empty() {
        return Err(ScriptLangError::with_span(
            "XML_EMPTY_ATTR",
            format!("Attribute \"{}\" on <{}> cannot be empty.", name, node.name),
            node.location.clone(),
        ));
    }

    Ok(raw.to_string())
}

fn has_attr(node: &XmlElementNode, name: &str) -> bool {
    node.attributes.contains_key(name)
}

fn stable_base(script_path: &str) -> String {
    script_path
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '.' || ch == '/' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn expand_script_macros(
    root: &XmlElementNode,
    reserved_var_names: &[String],
) -> Result<XmlElementNode, ScriptLangError> {
    let mut used_var_names = BTreeSet::new();
    for name in reserved_var_names {
        used_var_names.insert(name.clone());
    }
    collect_declared_var_names(root, &mut used_var_names);

    let mut context = MacroExpansionContext {
        used_var_names,
        loop_counter: 0,
    };

    Ok(XmlElementNode {
        name: root.name.clone(),
        attributes: root.attributes.clone(),
        children: expand_children(&root.children, &mut context)?,
        location: root.location.clone(),
    })
}

fn collect_declared_var_names(node: &XmlElementNode, names: &mut BTreeSet<String>) {
    if node.name == "var" {
        if let Some(name) = node.attributes.get("name") {
            if !name.is_empty() {
                names.insert(name.clone());
            }
        }
    }

    for child in element_children(node) {
        collect_declared_var_names(child, names);
    }
}

fn validate_reserved_prefix_in_user_var_declarations(
    node: &XmlElementNode,
) -> Result<(), ScriptLangError> {
    if node.name == "var" {
        if let Some(name) = node.attributes.get("name") {
            if !name.is_empty() {
                assert_name_not_reserved(name, "var", node.location.clone())?;
            }
        }
    }

    for child in element_children(node) {
        validate_reserved_prefix_in_user_var_declarations(child)?;
    }

    Ok(())
}

fn expand_children(
    children: &[XmlNode],
    context: &mut MacroExpansionContext,
) -> Result<Vec<XmlNode>, ScriptLangError> {
    let mut out = Vec::new();
    for child in children {
        match child {
            XmlNode::Text(text) => out.push(XmlNode::Text(text.clone())),
            XmlNode::Element(element) => {
                for expanded in expand_element_with_macros(element, context)? {
                    out.push(XmlNode::Element(expanded));
                }
            }
        }
    }
    Ok(out)
}

fn expand_element_with_macros(
    node: &XmlElementNode,
    context: &mut MacroExpansionContext,
) -> Result<Vec<XmlElementNode>, ScriptLangError> {
    if node.name != "loop" {
        return Ok(vec![XmlElementNode {
            name: node.name.clone(),
            attributes: node.attributes.clone(),
            children: expand_children(&node.children, context)?,
            location: node.location.clone(),
        }]);
    }

    let times_expr = parse_loop_times_expr(node)?;
    let temp_var_name = next_loop_temp_var_name(context);
    let body_children = expand_children(&node.children, context)?;

    let decrement_code = XmlElementNode {
        name: "code".to_string(),
        attributes: BTreeMap::new(),
        children: vec![XmlNode::Text(XmlTextNode {
            value: format!("{} = {} - 1;", temp_var_name, temp_var_name),
            location: node.location.clone(),
        })],
        location: node.location.clone(),
    };

    let mut loop_var_attrs = BTreeMap::new();
    loop_var_attrs.insert("name".to_string(), temp_var_name.clone());
    loop_var_attrs.insert("type".to_string(), "int".to_string());

    let loop_var = XmlElementNode {
        name: "var".to_string(),
        attributes: loop_var_attrs,
        children: vec![XmlNode::Text(XmlTextNode {
            value: times_expr,
            location: node.location.clone(),
        })],
        location: node.location.clone(),
    };

    let mut while_attrs = BTreeMap::new();
    while_attrs.insert("when".to_string(), format!("{} > 0", temp_var_name));

    let mut while_children = Vec::new();
    while_children.push(XmlNode::Element(decrement_code));
    while_children.extend(body_children);

    let loop_while = XmlElementNode {
        name: "while".to_string(),
        attributes: while_attrs,
        children: while_children,
        location: node.location.clone(),
    };

    Ok(vec![loop_var, loop_while])
}

fn parse_loop_times_expr(node: &XmlElementNode) -> Result<String, ScriptLangError> {
    let raw = get_required_non_empty_attr(node, "times")?;
    let trimmed = raw.trim();
    if trimmed.starts_with("${") && trimmed.ends_with('}') {
        return Err(ScriptLangError::with_span(
            "XML_LOOP_TIMES_TEMPLATE_UNSUPPORTED",
            "Attribute \"times\" on <loop> must not use ${...} wrapper.",
            node.location.clone(),
        ));
    }
    Ok(raw)
}

fn next_loop_temp_var_name(context: &mut MacroExpansionContext) -> String {
    loop {
        let candidate = format!("{}{}_remaining", LOOP_TEMP_VAR_PREFIX, context.loop_counter);
        context.loop_counter += 1;
        if context.used_var_names.insert(candidate.clone()) {
            return candidate;
        }
    }
}

fn slvalue_from_json(value: JsonValue) -> SlValue {
    match value {
        JsonValue::Null => SlValue::String("null".to_string()),
        JsonValue::Bool(value) => SlValue::Bool(value),
        JsonValue::Number(value) => SlValue::Number(value.as_f64().unwrap_or(0.0)),
        JsonValue::String(value) => SlValue::String(value),
        JsonValue::Array(values) => {
            SlValue::Array(values.into_iter().map(slvalue_from_json).collect())
        }
        JsonValue::Object(values) => SlValue::Map(
            values
                .into_iter()
                .map(|(key, value)| (key, slvalue_from_json(value)))
                .collect(),
        ),
    }
}

pub fn default_values_from_script_params(params: &[ScriptParam]) -> BTreeMap<String, SlValue> {
    let mut defaults = BTreeMap::new();
    for param in params {
        defaults.insert(param.name.clone(), default_value_from_type(&param.r#type));
    }
    defaults
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn map(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

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
    fn loop_macro_expands_to_var_and_while() {
        let files = map(&[(
            "main.script.xml",
            r#"
<script name="main">
  <var name="i" type="int">0</var>
  <loop times="2">
    <code>i = i + 1;</code>
  </loop>
</script>
"#,
        )]);

        let result = compile_project_bundle_from_xml_map(&files).expect("project should compile");
        let main = result.scripts.get("main").expect("main script");
        let root = main.groups.get(&main.root_group_id).expect("root group");
        assert!(root
            .nodes
            .iter()
            .any(|node| matches!(node, ScriptNode::Var { .. })));
        assert!(root
            .nodes
            .iter()
            .any(|node| matches!(node, ScriptNode::While { .. })));
    }

    fn read_sources_recursive(
        root: &Path,
        current: &Path,
        out: &mut BTreeMap<String, String>,
    ) -> Result<(), std::io::Error> {
        for entry in fs::read_dir(current)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                read_sources_recursive(root, &path, out)?;
                continue;
            }
            let relative = path
                .strip_prefix(root)
                .expect("path should be under root")
                .to_string_lossy()
                .replace('\\', "/");
            let text = fs::read_to_string(&path)?;
            out.insert(relative, text);
        }
        Ok(())
    }

    fn sources_from_example_dir(name: &str) -> BTreeMap<String, String> {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = manifest_dir
            .join("..")
            .join("..")
            .join("examples")
            .join("scripts-rhai")
            .join(name);
        let mut out = BTreeMap::new();
        read_sources_recursive(&root, &root, &mut out).expect("example sources should read");
        out
    }

    #[test]
    fn compile_bundle_supports_all_example_scenarios() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let examples_root = manifest_dir
            .join("..")
            .join("..")
            .join("examples")
            .join("scripts-rhai");

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
    fn default_values_from_script_params_respects_declared_types() {
        let params = vec![
            ScriptParam {
                name: "hp".to_string(),
                r#type: ScriptType::Primitive {
                    name: "int".to_string(),
                },
                is_ref: false,
                location: SourceSpan::synthetic(),
            },
            ScriptParam {
                name: "name".to_string(),
                r#type: ScriptType::Primitive {
                    name: "string".to_string(),
                },
                is_ref: false,
                location: SourceSpan::synthetic(),
            },
        ];
        let defaults = default_values_from_script_params(&params);
        assert_eq!(defaults.get("hp"), Some(&SlValue::Number(0.0)));
        assert_eq!(defaults.get("name"), Some(&SlValue::String(String::new())));
    }

    fn xml_text(value: &str) -> XmlNode {
        XmlNode::Text(XmlTextNode {
            value: value.to_string(),
            location: SourceSpan::synthetic(),
        })
    }

    fn xml_element(name: &str, attrs: &[(&str, &str)], children: Vec<XmlNode>) -> XmlElementNode {
        XmlElementNode {
            name: name.to_string(),
            attributes: attrs
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
            children,
            location: SourceSpan::synthetic(),
        }
    }

    #[test]
    fn source_kind_and_path_helpers_cover_common_cases() {
        assert!(matches!(
            detect_source_kind("a.script.xml"),
            Some(SourceKind::ScriptXml)
        ));
        assert!(matches!(
            detect_source_kind("a.defs.xml"),
            Some(SourceKind::DefsXml)
        ));
        assert!(matches!(
            detect_source_kind("a.json"),
            Some(SourceKind::Json)
        ));
        assert!(detect_source_kind("a.txt").is_none());

        assert_eq!(
            resolve_include_path("nested/main.script.xml", "../shared.defs.xml"),
            "shared.defs.xml"
        );
        assert_eq!(
            resolve_include_path("/", "shared/main.script.xml"),
            "shared/main.script.xml"
        );
        assert_eq!(
            normalize_virtual_path("./a/./b/../c\\d.script.xml"),
            "a/c/d.script.xml"
        );
        assert_eq!(stable_base("a*b?c"), "a_b_c");
    }

    #[test]
    fn parse_json_symbol_and_global_collection_errors_are_reported() {
        assert_eq!(
            parse_json_global_symbol("game.json").expect("symbol"),
            "game"
        );
        let invalid = parse_json_global_symbol("bad-name.json").expect_err("invalid");
        assert_eq!(invalid.code, "JSON_SYMBOL_INVALID");

        let reserved = parse_json_global_symbol("__sl_reserved.json").expect_err("reserved");
        assert_eq!(reserved.code, "NAME_RESERVED_PREFIX");

        let duplicate = compile_project_bundle_from_xml_map(&map(&[
            ("a/x.json", r#"{"v":1}"#),
            ("b/x.json", r#"{"v":2}"#),
            (
                "main.script.xml",
                r#"
<!-- include: a/x.json -->
<!-- include: b/x.json -->
<script name="main"><text>x</text></script>
"#,
            ),
        ]))
        .expect_err("duplicate symbol should fail");
        assert_eq!(duplicate.code, "JSON_SYMBOL_DUPLICATE");

        let missing_sources = BTreeMap::from([(
            "broken.json".to_string(),
            SourceFile {
                kind: SourceKind::Json,
                includes: Vec::new(),
                xml_root: None,
                json_value: None,
            },
        )]);
        let missing = collect_global_json(&missing_sources).expect_err("missing value");
        assert_eq!(missing.code, "JSON_MISSING_VALUE");
    }

    #[test]
    fn json_symbol_visibility_helpers_cover_context_edges() {
        let hidden_json = BTreeSet::from(["game".to_string()]);
        let allowed = BTreeSet::new();

        assert_eq!(
            find_hidden_json_symbol("value = game.hp;", &hidden_json, &allowed),
            Some("game".to_string())
        );
        assert_eq!(
            find_hidden_json_symbol("value = obj.game;", &hidden_json, &allowed),
            None
        );
        assert_eq!(
            find_hidden_json_symbol("value = #{game: 1};", &hidden_json, &allowed),
            None
        );
        assert_eq!(
            find_hidden_json_symbol(r#"value = "game"; // game"#, &hidden_json, &allowed),
            None
        );
        assert_eq!(
            find_hidden_json_symbol("/* game */ value = 1;", &hidden_json, &allowed),
            None
        );

        let locals = extract_local_bindings("let game = 1; const score = game + 1;");
        assert!(locals.contains("game"));
        assert!(locals.contains("score"));

        let allowed_game = BTreeSet::from(["game".to_string()]);
        assert_eq!(
            find_hidden_json_symbol("value = game.hp;", &hidden_json, &allowed_game),
            None
        );
    }

    #[test]
    fn compile_bundle_rejects_hidden_json_usage_without_include_in_script() {
        let files = map(&[
            ("game.json", r#"{ "hp": 5 }"#),
            (
                "main.script.xml",
                r#"<script name="main"><text>${game.hp}</text></script>"#,
            ),
        ]);

        let error = compile_project_bundle_from_xml_map(&files)
            .expect_err("missing json include should fail at compile time");
        assert_eq!(error.code, "JSON_SYMBOL_NOT_VISIBLE");
    }

    #[test]
    fn compile_bundle_rejects_hidden_json_usage_without_include_in_defs() {
        let files = map(&[
            ("game.json", r#"{ "hp": 5 }"#),
            (
                "shared.defs.xml",
                r#"
<defs name="shared">
  <function name="boost" return="int:out">
    out = game.hp;
  </function>
</defs>
"#,
            ),
            (
                "main.script.xml",
                r#"
<!-- include: shared.defs.xml -->
<script name="main">
  <var name="hp" type="int">1</var>
  <code>hp = shared.boost();</code>
</script>
"#,
            ),
        ]);

        let error = compile_project_bundle_from_xml_map(&files)
            .expect_err("defs code should fail when json is not visible");
        assert_eq!(error.code, "JSON_SYMBOL_NOT_VISIBLE");
    }

    #[test]
    fn compile_bundle_allows_visible_or_shadowed_json_symbols() {
        let visible = map(&[
            ("game.json", r#"{ "hp": 5 }"#),
            (
                "main.script.xml",
                r#"
<!-- include: game.json -->
<script name="main">
  <text>${game.hp}</text>
</script>
"#,
            ),
        ]);
        compile_project_bundle_from_xml_map(&visible).expect("visible json symbol should compile");

        let shadowed = map(&[
            ("game.json", r#"{ "hp": 5 }"#),
            (
                "main.script.xml",
                r#"
<script name="main">
  <var name="game" type="int">1</var>
  <code>game = game + 1;</code>
</script>
"#,
            ),
        ]);
        compile_project_bundle_from_xml_map(&shadowed)
            .expect("shadowed local name should compile without include");
    }

    #[test]
    fn json_symbol_visibility_validation_covers_all_script_node_paths() {
        let files = map(&[
            ("game.json", r#"{ "hp": 5 }"#),
            ("secret.json", r#"{ "v": 9 }"#),
            (
                "helpers.defs.xml",
                r#"
<defs name="helpers">
  <function name="boost" args="int:x" return="int:out">
    let local = x + game.hp;
    out = local;
  </function>
</defs>
"#,
            ),
            (
                "next.script.xml",
                r#"
<script name="next" args="int:n">
  <text>${n}</text>
</script>
"#,
            ),
            (
                "main.script.xml",
                r#"
<!-- include: game.json -->
<!-- include: helpers.defs.xml -->
<script name="main">
  <var name="hp" type="int">1</var>
  <var name="name" type="string">&quot;A&quot;</var>
  <if when="hp > 0">
    <text>ok</text>
  </if>
  <while when="hp > 0">
    <code>hp = hp - 1;</code>
    <continue/>
    <break/>
  </while>
  <choice text="c">
    <option text="o1" when="hp >= 0">
      <text>x</text>
    </option>
  </choice>
  <input var="name" text="in"/>
  <code>hp = helpers.boost(hp);</code>
  <call script="next" args="hp"/>
  <return script="next" args="hp"/>
</script>
"#,
            ),
        ]);

        compile_project_bundle_from_xml_map(&files)
            .expect("validation should pass when hidden json is not referenced");
    }

    #[test]
    fn parse_type_and_call_argument_helpers_cover_valid_and_invalid_inputs() {
        let span = SourceSpan::synthetic();
        assert!(matches!(
            parse_type_expr("int", &span).expect("primitive"),
            ParsedTypeExpr::Primitive(_)
        ));
        assert!(matches!(
            parse_type_expr("int[]", &span).expect("array"),
            ParsedTypeExpr::Array(_)
        ));
        assert!(matches!(
            parse_type_expr("#{int}", &span).expect("map"),
            ParsedTypeExpr::Map(_)
        ));
        assert!(matches!(
            parse_type_expr("CustomType", &span).expect("custom"),
            ParsedTypeExpr::Custom(_)
        ));
        let invalid_type = parse_type_expr("Map<int,string>", &span).expect_err("invalid");
        assert_eq!(invalid_type.code, "TYPE_PARSE_ERROR");
        let empty_map_type = parse_type_expr("#{   }", &span).expect_err("empty map type");
        assert_eq!(empty_map_type.code, "TYPE_PARSE_ERROR");

        let args = parse_args(Some("1, ref:hp, a + 1".to_string())).expect("args");
        assert_eq!(args.len(), 3);
        assert!(args[1].is_ref);

        let bad_args = parse_args(Some("ref:   ".to_string())).expect_err("bad args");
        assert_eq!(bad_args.code, "CALL_ARGS_PARSE_ERROR");
    }

    #[test]
    fn parse_var_declaration_rejects_value_attr_and_child_elements() {
        let visible_types = BTreeMap::new();
        let with_value = xml_element(
            "var",
            &[("name", "x"), ("type", "int"), ("value", "1")],
            Vec::new(),
        );
        let value_error =
            parse_var_declaration(&with_value, &visible_types).expect_err("value attr forbidden");
        assert_eq!(value_error.code, "XML_ATTR_NOT_ALLOWED");

        let with_child = xml_element(
            "var",
            &[("name", "x"), ("type", "int")],
            vec![XmlNode::Element(xml_element(
                "text",
                &[],
                vec![xml_text("bad")],
            ))],
        );
        let child_error = parse_var_declaration(&with_child, &visible_types)
            .expect_err("child element should be rejected");
        assert_eq!(child_error.code, "XML_VAR_CHILD_INVALID");
    }

    #[test]
    fn parse_script_args_and_function_decl_helpers_cover_error_paths() {
        let mut visible_types = BTreeMap::new();
        visible_types.insert(
            "Custom".to_string(),
            ScriptType::Object {
                type_name: "Custom".to_string(),
                fields: BTreeMap::new(),
            },
        );
        let root_ok = xml_element("script", &[("args", "int:a,ref:Custom:b")], Vec::new());
        let parsed = parse_script_args(&root_ok, &visible_types).expect("args parse");
        assert_eq!(parsed.len(), 2);
        assert!(parsed[1].is_ref);

        let root_bad = xml_element("script", &[("args", "int")], Vec::new());
        let error = parse_script_args(&root_bad, &visible_types).expect_err("bad args");
        assert_eq!(error.code, "SCRIPT_ARGS_PARSE_ERROR");

        let root_dup = xml_element("script", &[("args", "int:a,int:a")], Vec::new());
        let error = parse_script_args(&root_dup, &visible_types).expect_err("duplicate args");
        assert_eq!(error.code, "SCRIPT_ARGS_DUPLICATE");

        let fn_node = xml_element(
            "function",
            &[("name", "f"), ("args", "ref:int:a"), ("return", "int:r")],
            vec![xml_text("r = a;")],
        );
        let error = parse_function_declaration_node(&fn_node).expect_err("ref arg unsupported");
        assert_eq!(error.code, "XML_FUNCTION_ARGS_REF_UNSUPPORTED");

        let fn_bad_return = xml_element(
            "function",
            &[("name", "f"), ("args", "int:a"), ("return", "ref:int:r")],
            vec![xml_text("r = a;")],
        );
        let error =
            parse_function_declaration_node(&fn_bad_return).expect_err("ref return unsupported");
        assert_eq!(error.code, "XML_FUNCTION_RETURN_REF_UNSUPPORTED");
    }

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
        };

        let reachable = BTreeSet::from(["shared.defs.xml".to_string()]);
        let defs_by_path = BTreeMap::from([("shared.defs.xml".to_string(), defs)]);

        let (types, functions) =
            resolve_visible_defs(&reachable, &defs_by_path).expect("defs should resolve");
        assert!(types.contains_key("Obj"));
        let function = functions.get("make").expect("function should exist");
        assert_eq!(function.params.len(), 1);
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
                },
            ),
        ]);
        let reachable = BTreeSet::from(["a.defs.xml".to_string(), "b.defs.xml".to_string()]);
        let (_types, functions) =
            resolve_visible_defs(&reachable, &defs_by_path).expect("defs should resolve");
        assert!(functions.contains_key("a.doit"));
        assert!(functions.contains_key("b.doit"));
        assert!(!functions.contains_key("doit"));
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
    fn type_resolution_helpers_cover_nested_array_and_map_paths() {
        let span = SourceSpan::synthetic();
        let mut resolved = BTreeMap::new();
        let mut visiting = HashSet::new();
        let type_map = BTreeMap::from([(
            "Obj".to_string(),
            ParsedTypeDecl {
                name: "Obj".to_string(),
                qualified_name: "Obj".to_string(),
                fields: vec![ParsedTypeFieldDecl {
                    name: "n".to_string(),
                    type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                    location: span.clone(),
                }],
                location: span.clone(),
            },
        )]);

        let array = resolve_type_expr_with_lookup(
            &ParsedTypeExpr::Array(Box::new(ParsedTypeExpr::Custom("Obj".to_string()))),
            &type_map,
            &mut resolved,
            &mut visiting,
            &span,
        )
        .expect("array custom type should resolve");
        assert!(matches!(array, ScriptType::Array { .. }));

        let map = resolve_type_expr_with_lookup(
            &ParsedTypeExpr::Map(Box::new(ParsedTypeExpr::Custom("Obj".to_string()))),
            &type_map,
            &mut resolved,
            &mut visiting,
            &span,
        )
        .expect("map custom type should resolve");
        assert!(matches!(map, ScriptType::Map { .. }));

        let array_err = resolve_type_expr_with_lookup(
            &ParsedTypeExpr::Array(Box::new(ParsedTypeExpr::Custom("Missing".to_string()))),
            &type_map,
            &mut resolved,
            &mut visiting,
            &span,
        )
        .expect_err("unknown array element type should fail");
        assert_eq!(array_err.code, "TYPE_UNKNOWN");

        let map_err = resolve_type_expr_with_lookup(
            &ParsedTypeExpr::Map(Box::new(ParsedTypeExpr::Custom("Missing".to_string()))),
            &type_map,
            &mut resolved,
            &mut visiting,
            &span,
        )
        .expect_err("unknown map value type should fail");
        assert_eq!(map_err.code, "TYPE_UNKNOWN");

        let nested = parse_type_expr("#{int[]}", &span).expect("type should parse");
        assert!(matches!(nested, ParsedTypeExpr::Map(_)));

        let type_node = xml_element(
            "type",
            &[("name", "Bag")],
            vec![XmlNode::Element(xml_element(
                "field",
                &[("name", "values"), ("type", "#{int[]}")],
                Vec::new(),
            ))],
        );
        let parsed = parse_type_declaration_node(&type_node).expect("type node should parse");
        assert_eq!(parsed.fields.len(), 1);
    }

    #[test]
    fn parse_function_return_and_type_expr_success_paths_are_covered() {
        let function_node = xml_element(
            "function",
            &[("name", "f"), ("return", "int:out")],
            vec![xml_text("out = 1;")],
        );
        let parsed_return = parse_function_return(&function_node).expect("return should parse");
        assert_eq!(parsed_return.name, "out");

        let span = SourceSpan::synthetic();
        assert!(matches!(
            parse_type_expr("int[]", &span).expect("array should parse"),
            ParsedTypeExpr::Array(_)
        ));
        assert!(matches!(
            parse_type_expr("#{int}", &span).expect("map should parse"),
            ParsedTypeExpr::Map(_)
        ));
    }

    #[test]
    fn compile_group_recurses_for_if_while_and_choice_children() {
        let mut builder = GroupBuilder::new("recursive.script.xml");
        let root_group = builder.next_group_id();
        let container = xml_element(
            "script",
            &[("name", "main")],
            vec![
                XmlNode::Element(xml_element(
                    "if",
                    &[("when", "true")],
                    vec![
                        XmlNode::Element(xml_element("text", &[], vec![xml_text("A")])),
                        XmlNode::Element(xml_element(
                            "else",
                            &[],
                            vec![XmlNode::Element(xml_element(
                                "text",
                                &[],
                                vec![xml_text("B")],
                            ))],
                        )),
                    ],
                )),
                XmlNode::Element(xml_element(
                    "while",
                    &[("when", "false")],
                    vec![XmlNode::Element(xml_element(
                        "text",
                        &[],
                        vec![xml_text("W")],
                    ))],
                )),
                XmlNode::Element(xml_element(
                    "choice",
                    &[("text", "Pick")],
                    vec![XmlNode::Element(xml_element(
                        "option",
                        &[("text", "O")],
                        vec![XmlNode::Element(xml_element(
                            "text",
                            &[],
                            vec![xml_text("X")],
                        ))],
                    ))],
                )),
            ],
        );

        compile_group(
            &root_group,
            None,
            &container,
            &mut builder,
            &BTreeMap::new(),
            &BTreeMap::new(),
            CompileGroupMode::new(0, false),
        )
        .expect("group should compile");

        let group = builder
            .groups
            .get(&root_group)
            .expect("root group should exist");
        assert!(group.entry_node_id.is_some());
        assert_eq!(group.nodes.len(), 3);
    }

    #[test]
    fn inline_bool_and_attr_helpers_cover_errors() {
        let node = xml_element("text", &[("value", "x")], vec![xml_text("ignored")]);
        let error = parse_inline_required(&node).expect_err("value attr forbidden");
        assert_eq!(error.code, "XML_ATTR_NOT_ALLOWED");

        let empty = xml_element("text", &[], vec![xml_text("   ")]);
        let error = parse_inline_required(&empty).expect_err("empty inline forbidden");
        assert_eq!(error.code, "XML_EMPTY_NODE_CONTENT");

        let with_child = xml_element(
            "function",
            &[],
            vec![XmlNode::Element(xml_element("x", &[], Vec::new()))],
        );
        let error = parse_inline_required_no_element_children(&with_child)
            .expect_err("child element forbidden");
        assert_eq!(error.code, "XML_FUNCTION_CHILD_NODE_INVALID");

        let bool_node = xml_element("text", &[("once", "maybe")], vec![xml_text("x")]);
        let error = parse_bool_attr(&bool_node, "once", false).expect_err("invalid bool attr");
        assert_eq!(error.code, "XML_ATTR_BOOL_INVALID");

        let miss_attr = get_required_non_empty_attr(&xml_element("x", &[], vec![]), "name")
            .expect_err("missing attr");
        assert_eq!(miss_attr.code, "XML_MISSING_ATTR");
        let empty_attr =
            get_required_non_empty_attr(&xml_element("x", &[("name", " ")], vec![]), "name")
                .expect_err("empty attr");
        assert_eq!(empty_attr.code, "XML_EMPTY_ATTR");

        assert!(has_any_child_content(&xml_element(
            "x",
            &[],
            vec![xml_text(" t ")]
        )));
        assert!(!has_any_child_content(&xml_element(
            "x",
            &[],
            vec![xml_text("   ")]
        )));
        assert!(split_by_top_level_comma("a, f(1,2), #{int}, #{a:1,b:2}").len() >= 4);
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
    fn compile_group_reports_script_structure_errors() {
        let visible_types = BTreeMap::new();
        let local_var_types = BTreeMap::new();

        let bad_once = xml_element(
            "script",
            &[("name", "main")],
            vec![XmlNode::Element(xml_element(
                "code",
                &[("once", "true")],
                vec![xml_text("x = 1;")],
            ))],
        );
        let mut builder = GroupBuilder::new("main.script.xml");
        let group_id = builder.next_group_id();
        let error = compile_group(
            &group_id,
            None,
            &bad_once,
            &mut builder,
            &visible_types,
            &local_var_types,
            CompileGroupMode::new(0, false),
        )
        .expect_err("once on code should fail");
        assert_eq!(error.code, "XML_ATTR_NOT_ALLOWED");

        let bad_break = xml_element(
            "script",
            &[("name", "main")],
            vec![XmlNode::Element(xml_element("break", &[], Vec::new()))],
        );
        let mut builder = GroupBuilder::new("main.script.xml");
        let group_id = builder.next_group_id();
        let error = compile_group(
            &group_id,
            None,
            &bad_break,
            &mut builder,
            &visible_types,
            &local_var_types,
            CompileGroupMode::new(0, false),
        )
        .expect_err("break outside while should fail");
        assert_eq!(error.code, "XML_BREAK_OUTSIDE_WHILE");

        let bad_continue = xml_element(
            "script",
            &[("name", "main")],
            vec![XmlNode::Element(xml_element("continue", &[], Vec::new()))],
        );
        let mut builder = GroupBuilder::new("main.script.xml");
        let group_id = builder.next_group_id();
        let error = compile_group(
            &group_id,
            None,
            &bad_continue,
            &mut builder,
            &visible_types,
            &local_var_types,
            CompileGroupMode::new(0, false),
        )
        .expect_err("continue outside while/option should fail");
        assert_eq!(error.code, "XML_CONTINUE_OUTSIDE_WHILE_OR_OPTION");

        let bad_return = xml_element(
            "script",
            &[("name", "main")],
            vec![XmlNode::Element(xml_element(
                "return",
                &[("args", "1")],
                Vec::new(),
            ))],
        );
        let mut builder = GroupBuilder::new("main.script.xml");
        let group_id = builder.next_group_id();
        let error = compile_group(
            &group_id,
            None,
            &bad_return,
            &mut builder,
            &visible_types,
            &local_var_types,
            CompileGroupMode::new(0, false),
        )
        .expect_err("return args without script should fail");
        assert_eq!(error.code, "XML_RETURN_ARGS_REQUIRE_SCRIPT");

        let bad_node = xml_element(
            "script",
            &[("name", "main")],
            vec![XmlNode::Element(xml_element("unknown", &[], Vec::new()))],
        );
        let mut builder = GroupBuilder::new("main.script.xml");
        let group_id = builder.next_group_id();
        let error = compile_group(
            &group_id,
            None,
            &bad_node,
            &mut builder,
            &visible_types,
            &local_var_types,
            CompileGroupMode::new(0, false),
        )
        .expect_err("unknown node should fail");
        assert_eq!(error.code, "XML_NODE_UNSUPPORTED");
    }

    #[test]
    fn compiler_error_matrix_covers_more_validation_paths() {
        let cases: Vec<(&str, BTreeMap<String, String>, &str)> = vec![
            (
                "json parse error",
                map(&[
                    ("bad.json", "{"),
                    ("main.script.xml", "<script name=\"main\"><text>x</text></script>"),
                ]),
                "JSON_PARSE_ERROR",
            ),
            (
                "defs child invalid",
                map(&[
                    (
                        "x.defs.xml",
                        "<defs name=\"x\"><unknown/></defs>",
                    ),
                    (
                        "main.script.xml",
                        r#"
<!-- include: x.defs.xml -->
<script name="main"><text>x</text></script>
"#,
                    ),
                ]),
                "XML_DEFS_CHILD_INVALID",
            ),
            (
                "type field child invalid",
                map(&[
                    (
                        "x.defs.xml",
                        "<defs name=\"x\"><type name=\"A\"><bad/></type></defs>",
                    ),
                    (
                        "main.script.xml",
                        r#"
<!-- include: x.defs.xml -->
<script name="main"><text>x</text></script>
"#,
                    ),
                ]),
                "XML_TYPE_CHILD_INVALID",
            ),
            (
                "type field duplicate",
                map(&[
                    (
                        "x.defs.xml",
                        "<defs name=\"x\"><type name=\"A\"><field name=\"v\" type=\"int\"/><field name=\"v\" type=\"int\"/></type></defs>",
                    ),
                    (
                        "main.script.xml",
                        r#"
<!-- include: x.defs.xml -->
<script name="main"><text>x</text></script>
"#,
                    ),
                ]),
                "TYPE_FIELD_DUPLICATE",
            ),
            (
                "function duplicate",
                map(&[
                    (
                        "x.defs.xml",
                        "<defs name=\"x\"><function name=\"f\" return=\"int:r\">r=1;</function><function name=\"f\" return=\"int:r\">r=2;</function></defs>",
                    ),
                    (
                        "main.script.xml",
                        r#"
<!-- include: x.defs.xml -->
<script name="main"><text>x</text></script>
"#,
                    ),
                ]),
                "FUNCTION_DECL_DUPLICATE",
            ),
            (
                "unknown custom type in var",
                map(&[(
                    "main.script.xml",
                    "<script name=\"main\"><var name=\"x\" type=\"Unknown\"/></script>",
                )]),
                "TYPE_UNKNOWN",
            ),
            (
                "choice child invalid",
                map(&[(
                    "main.script.xml",
                    "<script name=\"main\"><choice text=\"c\"><bad/></choice></script>",
                )]),
                "XML_CHOICE_CHILD_INVALID",
            ),
            (
                "choice fall_over with when forbidden",
                map(&[(
                    "main.script.xml",
                    "<script name=\"main\"><choice text=\"c\"><option text=\"a\" fall_over=\"true\" when=\"true\"/></choice></script>",
                )]),
                "XML_OPTION_FALL_OVER_WHEN_FORBIDDEN",
            ),
            (
                "choice fall_over duplicate",
                map(&[(
                    "main.script.xml",
                    "<script name=\"main\"><choice text=\"c\"><option text=\"a\" fall_over=\"true\"/><option text=\"b\" fall_over=\"true\"/></choice></script>",
                )]),
                "XML_OPTION_FALL_OVER_DUPLICATE",
            ),
            (
                "choice fall_over not last",
                map(&[(
                    "main.script.xml",
                    "<script name=\"main\"><choice text=\"c\"><option text=\"a\" fall_over=\"true\"/><option text=\"b\"/></choice></script>",
                )]),
                "XML_OPTION_FALL_OVER_NOT_LAST",
            ),
            (
                "input default unsupported",
                map(&[(
                    "main.script.xml",
                    "<script name=\"main\"><input var=\"x\" text=\"p\" default=\"d\"/></script>",
                )]),
                "XML_INPUT_DEFAULT_UNSUPPORTED",
            ),
            (
                "input content forbidden",
                map(&[(
                    "main.script.xml",
                    "<script name=\"main\"><input var=\"x\" text=\"p\">x</input></script>",
                )]),
                "XML_INPUT_CONTENT_FORBIDDEN",
            ),
            (
                "return ref unsupported",
                map(&[(
                    "main.script.xml",
                    "<script name=\"main\"><return script=\"next\" args=\"ref:x\"/></script>",
                )]),
                "XML_RETURN_REF_UNSUPPORTED",
            ),
            (
                "removed node",
                map(&[(
                    "main.script.xml",
                    "<script name=\"main\"><set/></script>",
                )]),
                "XML_REMOVED_NODE",
            ),
            (
                "else at top level",
                map(&[(
                    "main.script.xml",
                    "<script name=\"main\"><else/></script>",
                )]),
                "XML_ELSE_POSITION",
            ),
            (
                "break outside while",
                map(&[(
                    "main.script.xml",
                    "<script name=\"main\"><break/></script>",
                )]),
                "XML_BREAK_OUTSIDE_WHILE",
            ),
            (
                "continue outside while or option",
                map(&[(
                    "main.script.xml",
                    "<script name=\"main\"><continue/></script>",
                )]),
                "XML_CONTINUE_OUTSIDE_WHILE_OR_OPTION",
            ),
            (
                "call args parse error",
                map(&[(
                    "main.script.xml",
                    "<script name=\"main\"><call script=\"s\" args=\"ref:\"/></script>",
                )]),
                "CALL_ARGS_PARSE_ERROR",
            ),
            (
                "script args reserved prefix",
                map(&[(
                    "main.script.xml",
                    "<script name=\"main\" args=\"int:__sl_x\"><text>x</text></script>",
                )]),
                "NAME_RESERVED_PREFIX",
            ),
            (
                "loop times template unsupported",
                map(&[(
                    "main.script.xml",
                    "<script name=\"main\"><loop times=\"${n}\"><text>x</text></loop></script>",
                )]),
                "XML_LOOP_TIMES_TEMPLATE_UNSUPPORTED",
            ),
        ];

        for (name, files, expected_code) in cases {
            let result = compile_project_bundle_from_xml_map(&files);
            assert!(result.is_err(), "case should fail: {}", name);
            let error = result.expect_err("error should exist");
            assert_eq!(error.code, expected_code, "case: {}", name);
        }
    }

    #[test]
    fn compiler_private_helpers_cover_remaining_paths() {
        assert_eq!(
            resolve_include_path("scripts/main.script.xml", "/shared.defs.xml"),
            "shared.defs.xml"
        );
        let reachable = collect_reachable_files("missing.script.xml", &BTreeMap::new());
        assert!(reachable.contains("missing.script.xml"));

        let visible_empty = collect_visible_json_symbols(
            &BTreeSet::from(["missing.json".to_string()]),
            &BTreeMap::new(),
        )
        .expect("missing reachable entries should be skipped");
        assert!(visible_empty.is_empty());

        let mut sources = BTreeMap::new();
        sources.insert(
            "a/x.json".to_string(),
            SourceFile {
                kind: SourceKind::Json,
                includes: Vec::new(),
                xml_root: None,
                json_value: Some(SlValue::Number(1.0)),
            },
        );
        sources.insert(
            "b/x.json".to_string(),
            SourceFile {
                kind: SourceKind::Json,
                includes: Vec::new(),
                xml_root: None,
                json_value: Some(SlValue::Number(2.0)),
            },
        );
        let duplicate_visible = collect_visible_json_symbols(
            &BTreeSet::from(["a/x.json".to_string(), "b/x.json".to_string()]),
            &sources,
        )
        .expect_err("duplicate visible json symbol should fail");
        assert_eq!(duplicate_visible.code, "JSON_SYMBOL_DUPLICATE");

        let invalid_file_name = parse_json_global_symbol("/").expect_err("invalid file name");
        assert_eq!(invalid_file_name.code, "JSON_SYMBOL_INVALID");

        let span = SourceSpan::synthetic();
        let field = ParsedTypeFieldDecl {
            name: "v".to_string(),
            type_expr: ParsedTypeExpr::Primitive("int".to_string()),
            location: span.clone(),
        };
        let mut type_map = BTreeMap::from([(
            "A".to_string(),
            ParsedTypeDecl {
                name: "A".to_string(),
                qualified_name: "A".to_string(),
                fields: vec![field.clone()],
                location: span.clone(),
            },
        )]);
        let mut resolved = BTreeMap::new();
        let mut visiting = HashSet::new();
        let _ = resolve_named_type("A", &type_map, &mut resolved, &mut visiting).expect("resolve");
        let _ = resolve_named_type("A", &type_map, &mut resolved, &mut visiting)
            .expect("resolved cache should be used");

        let unknown = resolve_named_type(
            "Missing",
            &type_map,
            &mut BTreeMap::new(),
            &mut HashSet::new(),
        )
        .expect_err("unknown type should fail");
        assert_eq!(unknown.code, "TYPE_UNKNOWN");

        type_map.insert(
            "Dup".to_string(),
            ParsedTypeDecl {
                name: "Dup".to_string(),
                qualified_name: "Dup".to_string(),
                fields: vec![field.clone(), field],
                location: span.clone(),
            },
        );
        let duplicate_field =
            resolve_named_type("Dup", &type_map, &mut BTreeMap::new(), &mut HashSet::new())
                .expect_err("duplicate type field should fail");
        assert_eq!(duplicate_field.code, "TYPE_FIELD_DUPLICATE");

        let mut resolved_for_lookup = BTreeMap::new();
        let mut visiting_for_lookup = HashSet::new();
        let array_ty = resolve_type_expr_with_lookup(
            &ParsedTypeExpr::Array(Box::new(ParsedTypeExpr::Primitive("int".to_string()))),
            &BTreeMap::new(),
            &mut resolved_for_lookup,
            &mut visiting_for_lookup,
            &span,
        )
        .expect("array lookup should resolve");
        assert!(matches!(array_ty, ScriptType::Array { .. }));
        let map_ty = resolve_type_expr_with_lookup(
            &ParsedTypeExpr::Map(Box::new(ParsedTypeExpr::Primitive("string".to_string()))),
            &BTreeMap::new(),
            &mut resolved_for_lookup,
            &mut visiting_for_lookup,
            &span,
        )
        .expect("map lookup should resolve");
        assert!(matches!(map_ty, ScriptType::Map { .. }));

        let array = resolve_type_expr(
            &ParsedTypeExpr::Array(Box::new(ParsedTypeExpr::Primitive("int".to_string()))),
            &BTreeMap::new(),
            &span,
        )
        .expect("array should resolve");
        assert!(matches!(array, ScriptType::Array { .. }));
        let map_resolved = resolve_type_expr(
            &ParsedTypeExpr::Map(Box::new(ParsedTypeExpr::Primitive("int".to_string()))),
            &BTreeMap::new(),
            &span,
        )
        .expect("map should resolve");
        assert!(matches!(map_resolved, ScriptType::Map { .. }));

        let non_script_root = xml_element("defs", &[("name", "x")], Vec::new());
        let compile_root_error = compile_script(
            "x.script.xml",
            &non_script_root,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &[],
            &BTreeSet::new(),
        )
        .expect_err("compile_script should require script root");
        assert_eq!(compile_root_error.code, "XML_ROOT_INVALID");

        let rich_script = map(&[(
            "main.script.xml",
            r#"
<script name="main">
  <if when="true">
    <text>A</text>
    <else><text>B</text></else>
  </if>
  <while when="false">
    <text>W</text>
  </while>
  <choice text="Pick">
    <option text="O"><text>X</text></option>
  </choice>
</script>
"#,
        )]);
        let compiled =
            compile_project_bundle_from_xml_map(&rich_script).expect("compile should pass");
        let main = compiled.scripts.get("main").expect("main script");
        let root_group = main.groups.get(&main.root_group_id).expect("root group");
        assert!(root_group
            .nodes
            .iter()
            .any(|node| matches!(node, ScriptNode::While { .. })));
        assert!(root_group
            .nodes
            .iter()
            .any(|node| matches!(node, ScriptNode::Input { .. })
                || matches!(node, ScriptNode::Call { .. })
                || matches!(node, ScriptNode::If { .. })));

        let defs_resolution = map(&[
            (
                "shared.defs.xml",
                r##"
<defs name="shared">
  <type name="Obj">
    <field name="values" type="#{int[]}"/>
  </type>
  <function name="build" return="Obj:r">
    r = #{values: #{a: [1]}};
  </function>
</defs>
"##,
            ),
            (
                "main.script.xml",
                r#"
<!-- include: shared.defs.xml -->
<script name="main">
  <var name="x" type="Obj"/>
</script>
"#,
            ),
        ]);
        let _ = compile_project_bundle_from_xml_map(&defs_resolution)
            .expect("defs return/field type resolution should pass");

        let mut builder_ok = GroupBuilder::new("manual.script.xml");
        let root_ok = builder_ok.next_group_id();
        let complex_container = xml_element(
            "script",
            &[("name", "main")],
            vec![
                XmlNode::Element(xml_element(
                    "if",
                    &[("when", "true")],
                    vec![
                        XmlNode::Element(xml_element("text", &[], vec![xml_text("A")])),
                        XmlNode::Element(xml_element(
                            "else",
                            &[],
                            vec![XmlNode::Element(xml_element(
                                "text",
                                &[],
                                vec![xml_text("B")],
                            ))],
                        )),
                    ],
                )),
                XmlNode::Element(xml_element(
                    "while",
                    &[("when", "false")],
                    vec![XmlNode::Element(xml_element(
                        "text",
                        &[],
                        vec![xml_text("W")],
                    ))],
                )),
                XmlNode::Element(xml_element(
                    "choice",
                    &[("text", "Pick")],
                    vec![XmlNode::Element(xml_element(
                        "option",
                        &[("text", "O")],
                        vec![XmlNode::Element(xml_element(
                            "text",
                            &[],
                            vec![xml_text("X")],
                        ))],
                    ))],
                )),
            ],
        );
        compile_group(
            &root_ok,
            None,
            &complex_container,
            &mut builder_ok,
            &BTreeMap::new(),
            &BTreeMap::new(),
            CompileGroupMode::new(0, false),
        )
        .expect("manual complex compile_group should pass");

        let mut loop_builder = GroupBuilder::new("loop.script.xml");
        let loop_group = loop_builder.next_group_id();
        let loop_error = compile_group(
            &loop_group,
            None,
            &xml_element(
                "script",
                &[("name", "main")],
                vec![XmlNode::Element(xml_element(
                    "loop",
                    &[("times", "2")],
                    vec![XmlNode::Element(xml_element(
                        "text",
                        &[],
                        vec![xml_text("x")],
                    ))],
                ))],
            ),
            &mut loop_builder,
            &BTreeMap::new(),
            &BTreeMap::new(),
            CompileGroupMode::new(0, false),
        )
        .expect_err("loop should have been expanded");
        assert_eq!(loop_error.code, "XML_LOOP_INTERNAL");

        let while_node = ScriptNode::While {
            id: "w1".to_string(),
            when_expr: "true".to_string(),
            body_group_id: "g".to_string(),
            location: SourceSpan::synthetic(),
        };
        let while_id = node_id(&while_node);
        assert_eq!(while_id, "w1");
        let input_node = ScriptNode::Input {
            id: "i1".to_string(),
            target_var: "name".to_string(),
            prompt_text: "p".to_string(),
            location: SourceSpan::synthetic(),
        };
        let input_id = node_id(&input_node);
        assert_eq!(input_id, "i1");
        let call_node = ScriptNode::Call {
            id: "c1".to_string(),
            target_script: "main".to_string(),
            args: Vec::new(),
            location: SourceSpan::synthetic(),
        };
        let call_id = node_id(&call_node);
        assert_eq!(call_id, "c1");

        let empty_args = parse_script_args(
            &xml_element("script", &[("args", "   ")], Vec::new()),
            &BTreeMap::new(),
        )
        .expect("empty script args should be accepted");
        assert!(empty_args.is_empty());
        let args_with_empty_segment = parse_script_args(
            &xml_element("script", &[("args", "int:a,,int:b")], Vec::new()),
            &BTreeMap::new(),
        )
        .expect("empty arg segment should be ignored");
        assert_eq!(args_with_empty_segment.len(), 2);
        let args_bad_start = parse_script_args(
            &xml_element("script", &[("args", ":a")], Vec::new()),
            &BTreeMap::new(),
        )
        .expect_err("bad args should fail");
        assert_eq!(args_bad_start.code, "SCRIPT_ARGS_PARSE_ERROR");
        let args_bad_end = parse_script_args(
            &xml_element("script", &[("args", "int:")], Vec::new()),
            &BTreeMap::new(),
        )
        .expect_err("bad args should fail");
        assert_eq!(args_bad_end.code, "SCRIPT_ARGS_PARSE_ERROR");
        let args_empty_name = parse_script_args(
            &xml_element("script", &[("args", "int:   ")], Vec::new()),
            &BTreeMap::new(),
        )
        .expect_err("empty script arg name should fail");
        assert_eq!(args_empty_name.code, "SCRIPT_ARGS_PARSE_ERROR");

        let empty_fn_args = parse_function_args(&xml_element(
            "function",
            &[("name", "f"), ("args", "   "), ("return", "int:r")],
            vec![xml_text("r = 1;")],
        ))
        .expect("empty function args should be accepted");
        assert!(empty_fn_args.is_empty());
        let fn_args_bad_start = parse_function_args(&xml_element(
            "function",
            &[("name", "f"), ("args", ":a"), ("return", "int:r")],
            vec![xml_text("r = 1;")],
        ))
        .expect_err("bad function args should fail");
        assert_eq!(fn_args_bad_start.code, "FUNCTION_ARGS_PARSE_ERROR");
        let fn_args_bad_end = parse_function_args(&xml_element(
            "function",
            &[("name", "f"), ("args", "int:"), ("return", "int:r")],
            vec![xml_text("r = 1;")],
        ))
        .expect_err("bad function args should fail");
        assert_eq!(fn_args_bad_end.code, "FUNCTION_ARGS_PARSE_ERROR");
        let fn_args_dup = parse_function_args(&xml_element(
            "function",
            &[("name", "f"), ("args", "int:a,int:a"), ("return", "int:r")],
            vec![xml_text("r = 1;")],
        ))
        .expect_err("duplicate function args should fail");
        assert_eq!(fn_args_dup.code, "FUNCTION_ARGS_DUPLICATE");
        let fn_args_no_colon = parse_function_args(&xml_element(
            "function",
            &[("name", "f"), ("args", "int"), ("return", "int:r")],
            vec![xml_text("r = 1;")],
        ))
        .expect_err("function arg without colon should fail");
        assert_eq!(fn_args_no_colon.code, "FUNCTION_ARGS_PARSE_ERROR");

        let ret_no_colon = parse_function_return(&xml_element(
            "function",
            &[("name", "f"), ("return", "int")],
            vec![xml_text("x")],
        ))
        .expect_err("return parse should fail");
        assert_eq!(ret_no_colon.code, "FUNCTION_RETURN_PARSE_ERROR");
        let ret_bad_edge = parse_function_return(&xml_element(
            "function",
            &[("name", "f"), ("return", "int:")],
            vec![xml_text("x")],
        ))
        .expect_err("return parse should fail");
        assert_eq!(ret_bad_edge.code, "FUNCTION_RETURN_PARSE_ERROR");

        let empty_call_args = parse_args(Some("   ".to_string())).expect("empty call args");
        assert!(empty_call_args.is_empty());
        let _ = parse_type_expr("int[]", &SourceSpan::synthetic()).expect("array parse");
        let _ = parse_type_expr("#{int}", &SourceSpan::synthetic()).expect("map parse");
        let _ =
            parse_type_expr("#{int[]}", &SourceSpan::synthetic()).expect("nested map/array parse");

        let inline = inline_text_content(&xml_element(
            "x",
            &[],
            vec![XmlNode::Element(xml_element("y", &[], Vec::new()))],
        ));
        assert!(inline.is_empty());

        let split = split_by_top_level_comma("'a,b',[1,2],{k:1}");
        assert_eq!(split.len(), 3);

        assert!(has_any_child_content(&xml_element(
            "x",
            &[],
            vec![XmlNode::Element(xml_element("y", &[], Vec::new()))]
        )));

        let mut declared = BTreeSet::new();
        collect_declared_var_names(
            &xml_element("var", &[("name", "")], Vec::new()),
            &mut declared,
        );
        assert!(declared.is_empty());
        collect_declared_var_names(&xml_element("var", &[], Vec::new()), &mut declared);
        assert!(declared.is_empty());
        validate_reserved_prefix_in_user_var_declarations(&xml_element(
            "var",
            &[("name", "")],
            Vec::new(),
        ))
        .expect("empty var name should be ignored");
        validate_reserved_prefix_in_user_var_declarations(&xml_element("var", &[], Vec::new()))
            .expect("var without name should be ignored");

        let mut context = MacroExpansionContext {
            used_var_names: BTreeSet::from([format!("{}{}_remaining", LOOP_TEMP_VAR_PREFIX, 0)]),
            loop_counter: 0,
        };
        let generated = next_loop_temp_var_name(&mut context);
        assert!(generated.ends_with("_remaining"));

        assert_eq!(
            slvalue_from_json(JsonValue::Null),
            SlValue::String("null".to_string())
        );
    }
}
