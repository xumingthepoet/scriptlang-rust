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

    let mut scripts = BTreeMap::new();

    for (file_path, source) in &sources {
        if !matches!(source.kind, SourceKind::ScriptXml) {
            continue;
        }

        let Some(script_root) = &source.xml_root else {
            continue;
        };

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
    let parent = Path::new(current_path)
        .parent()
        .unwrap_or_else(|| Path::new(""));
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

        if let Some(source) = sources.get(node) {
            for include in &source.includes {
                dfs(include, sources, states, stack)?;
            }
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

        let Some(root) = &source.xml_root else {
            continue;
        };

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
                "type" => type_decls.push(parse_type_declaration_node(child)?),
                "function" => function_decls.push(parse_function_declaration_node(child)?),
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
        let value = source
            .json_value
            .clone()
            .ok_or_else(|| ScriptLangError::new("JSON_MISSING_VALUE", "Missing JSON value."))?;
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
    let mut type_decls_map = BTreeMap::new();

    for path in reachable {
        let Some(defs) = defs_by_path.get(path) else {
            continue;
        };
        for decl in &defs.type_decls {
            if type_decls_map.contains_key(&decl.name) {
                return Err(ScriptLangError::with_span(
                    "TYPE_DECL_DUPLICATE",
                    format!("Duplicate type declaration \"{}\".", decl.name),
                    decl.location.clone(),
                ));
            }
            type_decls_map.insert(decl.name.clone(), decl.clone());
        }
    }

    let mut resolved_types: BTreeMap<String, ScriptType> = BTreeMap::new();
    let mut visiting = HashSet::new();

    for type_name in type_decls_map.keys() {
        resolve_named_type(
            type_name,
            &type_decls_map,
            &mut resolved_types,
            &mut visiting,
        )?;
    }

    let mut functions = BTreeMap::new();

    for path in reachable {
        let Some(defs) = defs_by_path.get(path) else {
            continue;
        };

        for decl in &defs.function_decls {
            if functions.contains_key(&decl.name) {
                return Err(ScriptLangError::with_span(
                    "FUNCTION_DECL_DUPLICATE",
                    format!("Duplicate function declaration \"{}\".", decl.name),
                    decl.location.clone(),
                ));
            }

            let mut params = Vec::new();
            for param in &decl.params {
                params.push(FunctionParam {
                    name: param.name.clone(),
                    r#type: resolve_type_expr(&param.type_expr, &resolved_types, &param.location)?,
                    location: param.location.clone(),
                });
            }

            let return_type = resolve_type_expr(
                &decl.return_binding.type_expr,
                &resolved_types,
                &decl.return_binding.location,
            )?;

            functions.insert(
                decl.name.clone(),
                FunctionDecl {
                    name: decl.name.clone(),
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
        }
    }

    Ok((resolved_types, functions))
}

fn resolve_named_type(
    name: &str,
    type_decls_map: &BTreeMap<String, ParsedTypeDecl>,
    resolved: &mut BTreeMap<String, ScriptType>,
    visiting: &mut HashSet<String>,
) -> Result<ScriptType, ScriptLangError> {
    if let Some(found) = resolved.get(name) {
        return Ok(found.clone());
    }

    if !visiting.insert(name.to_string()) {
        return Err(ScriptLangError::new(
            "TYPE_DECL_RECURSIVE",
            format!("Recursive type declaration detected for \"{}\".", name),
        ));
    }

    let Some(decl) = type_decls_map.get(name) else {
        visiting.remove(name);
        return Err(ScriptLangError::new(
            "TYPE_UNKNOWN",
            format!("Unknown type \"{}\".", name),
        ));
    };

    let mut fields = BTreeMap::new();
    for field in &decl.fields {
        if fields.contains_key(&field.name) {
            visiting.remove(name);
            return Err(ScriptLangError::with_span(
                "TYPE_FIELD_DUPLICATE",
                format!("Duplicate field \"{}\" in type \"{}\".", field.name, name),
                field.location.clone(),
            ));
        }
        let field_type = resolve_type_expr_with_lookup(
            &field.type_expr,
            type_decls_map,
            resolved,
            visiting,
            &field.location,
        )?;
        fields.insert(field.name.clone(), field_type);
    }

    visiting.remove(name);

    let resolved_type = ScriptType::Object {
        type_name: name.to_string(),
        fields,
    };
    resolved.insert(name.to_string(), resolved_type.clone());
    Ok(resolved_type)
}

fn resolve_type_expr_with_lookup(
    expr: &ParsedTypeExpr,
    type_decls_map: &BTreeMap<String, ParsedTypeDecl>,
    resolved: &mut BTreeMap<String, ScriptType>,
    visiting: &mut HashSet<String>,
    span: &SourceSpan,
) -> Result<ScriptType, ScriptLangError> {
    match expr {
        ParsedTypeExpr::Primitive(name) => Ok(ScriptType::Primitive { name: name.clone() }),
        ParsedTypeExpr::Array(element_type) => Ok(ScriptType::Array {
            element_type: Box::new(resolve_type_expr_with_lookup(
                element_type,
                type_decls_map,
                resolved,
                visiting,
                span,
            )?),
        }),
        ParsedTypeExpr::Map(value_type) => Ok(ScriptType::Map {
            key_type: "string".to_string(),
            value_type: Box::new(resolve_type_expr_with_lookup(
                value_type,
                type_decls_map,
                resolved,
                visiting,
                span,
            )?),
        }),
        ParsedTypeExpr::Custom(name) => {
            resolve_named_type(name, type_decls_map, resolved, visiting).map_err(|_| {
                ScriptLangError::with_span(
                    "TYPE_UNKNOWN",
                    format!("Unknown custom type \"{}\".", name),
                    span.clone(),
                )
            })
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
        ParsedTypeExpr::Custom(name) => resolved_types.get(name).cloned().ok_or_else(|| {
            ScriptLangError::with_span(
                "TYPE_UNKNOWN",
                format!("Unknown custom type \"{}\".", name),
                span.clone(),
            )
        }),
    }
}

fn parse_type_declaration_node(node: &XmlElementNode) -> Result<ParsedTypeDecl, ScriptLangError> {
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

        let field_type = parse_type_expr(
            &get_required_non_empty_attr(child, "type")?,
            &child.location,
        )?;
        fields.push(ParsedTypeFieldDecl {
            name: field_name,
            type_expr: field_type,
            location: child.location.clone(),
        });
    }

    Ok(ParsedTypeDecl {
        name,
        fields,
        location: node.location.clone(),
    })
}

fn parse_function_declaration_node(
    node: &XmlElementNode,
) -> Result<ParsedFunctionDecl, ScriptLangError> {
    let name = get_required_non_empty_attr(node, "name")?;
    assert_name_not_reserved(&name, "function", node.location.clone())?;

    let params = parse_function_args(node)?;
    let return_binding = parse_function_return(node)?;
    let code = parse_inline_required_no_element_children(node)?;

    Ok(ParsedFunctionDecl {
        name,
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
        0,
        false,
    )?;

    Ok(ScriptIr {
        script_path: script_path.to_string(),
        script_name,
        params,
        root_group_id,
        groups: builder.groups,
        visible_json_globals: visible_json_globals.to_vec(),
        visible_functions: visible_functions.clone(),
    })
}

#[allow(clippy::too_many_arguments)]
fn compile_group(
    group_id: &str,
    parent_group_id: Option<&str>,
    container: &XmlElementNode,
    builder: &mut GroupBuilder,
    visible_types: &BTreeMap<String, ScriptType>,
    visible_var_types: &BTreeMap<String, ScriptType>,
    while_depth: usize,
    allow_option_direct_continue: bool,
) -> Result<(), ScriptLangError> {
    let mut local_var_types = visible_var_types.clone();
    let mut nodes = Vec::new();

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

                compile_group(
                    &then_group_id,
                    Some(group_id),
                    &then_container,
                    builder,
                    visible_types,
                    &local_var_types,
                    while_depth,
                    false,
                )?;

                if let Some(else_child) = else_node {
                    compile_group(
                        &else_group_id,
                        Some(group_id),
                        else_child,
                        builder,
                        visible_types,
                        &local_var_types,
                        while_depth,
                        false,
                    )?;
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
                compile_group(
                    &body_group_id,
                    Some(group_id),
                    child,
                    builder,
                    visible_types,
                    &local_var_types,
                    while_depth + 1,
                    false,
                )?;
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
                    compile_group(
                        &option_group_id,
                        Some(group_id),
                        option,
                        builder,
                        visible_types,
                        &local_var_types,
                        while_depth,
                        true,
                    )?;

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
                if while_depth == 0 {
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
                let target = if while_depth > 0 {
                    ContinueTarget::While
                } else if allow_option_direct_continue {
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

        if let Some(group) = builder.groups.get_mut(group_id) {
            if group.entry_node_id.is_none() {
                group.entry_node_id = Some(node_id(&node).to_string());
            }
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

    Ok(VarDeclaration {
        name,
        r#type: ty,
        initial_value_expr: get_optional_attr(node, "value"),
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
        if name.is_empty() {
            return Err(ScriptLangError::with_span(
                "SCRIPT_ARGS_PARSE_ERROR",
                format!("Invalid script args segment: \"{}\".", segment),
                root.location.clone(),
            ));
        }

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
    if source == "number" || source == "string" || source == "boolean" {
        return Ok(ParsedTypeExpr::Primitive(source.to_string()));
    }

    if let Some(stripped) = source.strip_suffix("[]") {
        return Ok(ParsedTypeExpr::Array(Box::new(parse_type_expr(
            stripped, span,
        )?)));
    }

    if let Some(inner) = source.strip_prefix("Map<string,") {
        if let Some(value) = inner.strip_suffix('>') {
            return Ok(ParsedTypeExpr::Map(Box::new(parse_type_expr(
                value.trim(),
                span,
            )?)));
        }
    }

    let custom_regex = Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*$").expect("type regex must compile");
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
    let mut angle_depth = 0usize;
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
            '<' => angle_depth += 1,
            '>' if angle_depth > 0 => angle_depth -= 1,
            '(' => paren_depth += 1,
            ')' if paren_depth > 0 => paren_depth -= 1,
            '[' => bracket_depth += 1,
            ']' if bracket_depth > 0 => bracket_depth -= 1,
            '{' => brace_depth += 1,
            '}' if brace_depth > 0 => brace_depth -= 1,
            ',' if angle_depth == 0
                && paren_depth == 0
                && bracket_depth == 0
                && brace_depth == 0 =>
            {
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
    node.children.iter().any(|entry| match entry {
        XmlNode::Element(_) => true,
        XmlNode::Text(text) => !text.value.trim().is_empty(),
    })
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
    loop_var_attrs.insert("type".to_string(), "number".to_string());
    loop_var_attrs.insert("value".to_string(), times_expr);

    let loop_var = XmlElementNode {
        name: "var".to_string(),
        attributes: loop_var_attrs,
        children: Vec::new(),
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
    params
        .iter()
        .map(|param| (param.name.clone(), default_value_from_type(&param.r#type)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
  <var name="i" type="number" value="0"/>
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
}
