use crate::*;

pub(crate) fn parse_type_expr(
    raw: &str,
    span: &SourceSpan,
) -> Result<ParsedTypeExpr, ScriptLangError> {
    let source = raw.trim();
    if source == "script" {
        return Ok(ParsedTypeExpr::Script);
    }
    if source == "function" {
        return Ok(ParsedTypeExpr::Function);
    }

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
        if let Some((raw_key_type, raw_value_type)) = split_map_type_key_value(value) {
            let key_type = parse_type_expr(raw_key_type, span)?;
            let value_type = parse_type_expr(raw_value_type, span)?;
            return Ok(ParsedTypeExpr::Map {
                key_type: Box::new(key_type),
                value_type: Box::new(value_type),
            });
        }
        let value_type = parse_type_expr(value.trim(), span)?;
        return Ok(ParsedTypeExpr::Map {
            key_type: Box::new(ParsedTypeExpr::Primitive("string".to_string())),
            value_type: Box::new(value_type),
        });
    }

    if type_name_regex().is_match(source) {
        return Ok(ParsedTypeExpr::Custom(source.to_string()));
    }

    Err(ScriptLangError::with_span(
        "TYPE_PARSE_ERROR",
        format!("Unsupported type syntax: \"{}\".", raw),
        span.clone(),
    ))
}

fn split_map_type_key_value(raw: &str) -> Option<(&str, &str)> {
    let chars = raw.char_indices().collect::<Vec<_>>();
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut brace_depth = 0usize;
    let mut quote: Option<char> = None;
    let mut idx = 0usize;

    while idx < chars.len() {
        let (_, ch) = chars[idx];
        if let Some(active_quote) = quote {
            if ch == '\\' {
                idx += 2;
                continue;
            }
            if ch == active_quote {
                quote = None;
            }
            idx += 1;
            continue;
        }

        if ch == '\'' || ch == '"' {
            quote = Some(ch);
            idx += 1;
            continue;
        }

        match ch {
            '(' => paren_depth += 1,
            ')' if paren_depth > 0 => paren_depth -= 1,
            '[' => bracket_depth += 1,
            ']' if bracket_depth > 0 => bracket_depth -= 1,
            '{' => brace_depth += 1,
            '}' if brace_depth > 0 => brace_depth -= 1,
            '=' if paren_depth == 0
                && bracket_depth == 0
                && brace_depth == 0
                && chars.get(idx + 1).is_some_and(|(_, next)| *next == '>') =>
            {
                let (start, _) = chars[idx];
                let end = chars[idx + 1].0 + '>'.len_utf8();
                let left = raw[..start].trim();
                let right = raw[end..].trim();
                if left.is_empty() || right.is_empty() {
                    return None;
                }
                return Some((left, right));
            }
            _ => {}
        }
        idx += 1;
    }

    None
}

pub(crate) fn type_name_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)*$")
            .expect("type regex must compile")
    })
}

pub(crate) fn parse_args(raw: Option<String>) -> Result<Vec<CallArgument>, ScriptLangError> {
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

pub(crate) fn parse_inline_required(node: &XmlElementNode) -> Result<String, ScriptLangError> {
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

pub(crate) fn parse_inline_required_no_element_children(
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

pub(crate) fn inline_text_content(node: &XmlElementNode) -> String {
    node.children
        .iter()
        .filter_map(|entry| match entry {
            XmlNode::Text(XmlTextNode { value, .. }) => Some(value.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn parse_bool_attr(
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

pub(crate) fn parse_access_attr(
    node: &XmlElementNode,
    attr_name: &str,
    default: AccessLevel,
) -> Result<AccessLevel, ScriptLangError> {
    let Some(raw) = get_optional_attr(node, attr_name) else {
        return Ok(default);
    };
    match raw.trim() {
        "public" => Ok(AccessLevel::Public),
        "private" => Ok(AccessLevel::Private),
        _ => Err(ScriptLangError::with_span(
            "XML_ACCESS_INVALID",
            format!(
                "Attribute \"{}\" on <{}> must be \"public\" or \"private\".",
                attr_name, node.name
            ),
            node.location.clone(),
        )),
    }
}

pub(crate) fn split_by_top_level_comma(raw: &str) -> Vec<String> {
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

pub(crate) fn assert_name_not_reserved(
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

pub(crate) fn assert_decl_name_not_reserved_or_rhai_keyword(
    name: &str,
    label: &str,
    span: SourceSpan,
) -> Result<(), ScriptLangError> {
    assert_name_not_reserved(name, label, span.clone())?;
    if !rhai_decl_name_conflicts_keyword(name) {
        return Ok(());
    }

    Err(ScriptLangError::with_span(
        "NAME_RHAI_KEYWORD_RESERVED",
        format!(
            "Name \"{}\" for {} conflicts with Rhai keyword or reserved identifier.",
            name, label
        ),
        span,
    ))
}

fn rhai_decl_name_conflicts_keyword(name: &str) -> bool {
    name.split('.').any(is_rhai_reserved_keyword)
}

fn is_rhai_reserved_keyword(name: &str) -> bool {
    matches!(
        name,
        "_" | "Fn"
            | "as"
            | "async"
            | "await"
            | "break"
            | "call"
            | "case"
            | "catch"
            | "const"
            | "continue"
            | "curry"
            | "debug"
            | "default"
            | "do"
            | "else"
            | "eval"
            | "export"
            | "false"
            | "fn"
            | "for"
            | "go"
            | "goto"
            | "if"
            | "import"
            | "in"
            | "is"
            | "is_def_fn"
            | "is_def_var"
            | "is_shared"
            | "let"
            | "loop"
            | "match"
            | "module"
            | "new"
            | "nil"
            | "null"
            | "package"
            | "print"
            | "private"
            | "protected"
            | "public"
            | "return"
            | "shared"
            | "spawn"
            | "static"
            | "super"
            | "switch"
            | "sync"
            | "this"
            | "thread"
            | "throw"
            | "true"
            | "try"
            | "type_of"
            | "until"
            | "use"
            | "var"
            | "void"
            | "while"
            | "with"
            | "yield"
    )
}

pub(crate) fn element_children(node: &XmlElementNode) -> impl Iterator<Item = &XmlElementNode> {
    node.children.iter().filter_map(|entry| match entry {
        XmlNode::Element(element) => Some(element),
        _ => None,
    })
}

pub(crate) fn has_any_child_content(node: &XmlElementNode) -> bool {
    for entry in &node.children {
        match entry {
            XmlNode::Element(_) => return true,
            XmlNode::Text(text) if !text.value.trim().is_empty() => return true,
            XmlNode::Text(_) => {}
        }
    }
    false
}

pub(crate) fn get_optional_attr(node: &XmlElementNode, name: &str) -> Option<String> {
    node.attributes.get(name).cloned()
}

pub(crate) fn get_required_non_empty_attr(
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

pub(crate) fn has_attr(node: &XmlElementNode, name: &str) -> bool {
    node.attributes.contains_key(name)
}

fn enum_template_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"\$\{([^{}]+)\}").expect("enum template regex must compile"))
}

fn is_expr_ident_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_'
}

fn is_expr_ident_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_' || ch == '.'
}

fn prev_non_whitespace_char(chars: &[char], index: usize) -> Option<char> {
    if index == 0 {
        return None;
    }
    let mut cursor = index;
    while cursor > 0 {
        cursor -= 1;
        let ch = chars[cursor];
        if !ch.is_whitespace() {
            return Some(ch);
        }
    }
    None
}

fn next_non_whitespace_char(chars: &[char], mut index: usize) -> Option<char> {
    while index < chars.len() {
        let ch = chars[index];
        if !ch.is_whitespace() {
            return Some(ch);
        }
        index += 1;
    }
    None
}

pub(crate) fn build_module_symbol_alias_rewrite_map(
    visible_module_vars: &BTreeMap<String, ModuleVarDecl>,
    visible_module_consts: &BTreeMap<String, ModuleConstDecl>,
    blocked_names: &BTreeSet<String>,
) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();

    for (alias, decl) in visible_module_vars {
        if alias == &decl.qualified_name || blocked_names.contains(alias) {
            continue;
        }
        map.entry(alias.clone())
            .or_insert_with(|| decl.qualified_name.clone());
    }

    for (alias, decl) in visible_module_consts {
        if alias == &decl.qualified_name || blocked_names.contains(alias) {
            continue;
        }
        map.entry(alias.clone())
            .or_insert_with(|| decl.qualified_name.clone());
    }

    map
}

pub(crate) fn rewrite_module_symbol_aliases_in_expression(
    expr: &str,
    alias_to_qualified: &BTreeMap<String, String>,
) -> String {
    if alias_to_qualified.is_empty() {
        return expr.to_string();
    }

    let chars = expr.chars().collect::<Vec<_>>();
    let mut out = String::with_capacity(expr.len());
    let mut index = 0usize;
    let mut brace_is_map_stack: Vec<bool> = Vec::new();

    while index < chars.len() {
        let ch = chars[index];
        if ch == '"' || ch == '\'' {
            out.push(ch);
            index += 1;
            while index < chars.len() {
                let inner = chars[index];
                out.push(inner);
                index += 1;
                if inner == '\\' && index < chars.len() {
                    out.push(chars[index]);
                    index += 1;
                    continue;
                }
                if inner == ch {
                    break;
                }
            }
            continue;
        }

        if ch == '{' {
            let is_map_literal = prev_non_whitespace_char(&chars, index) == Some('#');
            brace_is_map_stack.push(is_map_literal);
            out.push(ch);
            index += 1;
            continue;
        }
        if ch == '}' {
            if !brace_is_map_stack.is_empty() {
                brace_is_map_stack.pop();
            }
            out.push(ch);
            index += 1;
            continue;
        }

        if is_expr_ident_start(ch) {
            let start = index;
            index += 1;
            while index < chars.len() && is_expr_ident_char(chars[index]) {
                index += 1;
            }
            let token = chars[start..index].iter().collect::<String>();
            let prev_char = prev_non_whitespace_char(&chars, start);
            let next_char = next_non_whitespace_char(&chars, index);
            let is_map_key = brace_is_map_stack.last().copied().unwrap_or(false)
                && next_char == Some(':')
                && matches!(prev_char, Some('{') | Some(','));
            let is_script_or_function_literal = matches!(prev_char, Some('@') | Some('*'));
            if !is_map_key && !is_script_or_function_literal {
                if let Some(rewrite) = alias_to_qualified.get(&token) {
                    out.push_str(rewrite);
                    continue;
                }
                if let Some((root, rest)) = token.split_once('.') {
                    if let Some(root_rewrite) = alias_to_qualified.get(root) {
                        out.push_str(root_rewrite);
                        out.push('.');
                        out.push_str(rest);
                        continue;
                    }
                }
            }

            out.push_str(&token);
            continue;
        }

        out.push(ch);
        index += 1;
    }

    out
}

pub(crate) fn rewrite_module_symbol_aliases_in_template(
    template: &str,
    alias_to_qualified: &BTreeMap<String, String>,
) -> String {
    if alias_to_qualified.is_empty() {
        return template.to_string();
    }
    let mut out = String::new();
    let mut last_index = 0usize;
    for captures in enum_template_regex().captures_iter(template) {
        let full = captures
            .get(0)
            .expect("capture group 0 must exist for each template capture");
        let expr = captures
            .get(1)
            .expect("capture group 1 must exist for each template capture");
        out.push_str(&template[last_index..full.start()]);
        let rewritten =
            rewrite_module_symbol_aliases_in_expression(expr.as_str(), alias_to_qualified);
        out.push_str("${");
        out.push_str(&rewritten);
        out.push('}');
        last_index = full.end();
    }
    out.push_str(&template[last_index..]);
    out
}

pub(crate) fn rewrite_and_validate_enum_literals_in_expression(
    expr: &str,
    visible_types: &BTreeMap<String, ScriptType>,
    span: &SourceSpan,
) -> Result<String, ScriptLangError> {
    rewrite_and_validate_enum_literals_with_quote(expr, visible_types, span, '"')
}

pub(crate) fn rewrite_and_validate_enum_literals_in_attr_expression(
    expr: &str,
    visible_types: &BTreeMap<String, ScriptType>,
    span: &SourceSpan,
) -> Result<String, ScriptLangError> {
    rewrite_and_validate_enum_literals_with_quote(expr, visible_types, span, '\'')
}

fn rewrite_and_validate_enum_literals_with_quote(
    expr: &str,
    visible_types: &BTreeMap<String, ScriptType>,
    span: &SourceSpan,
    quote: char,
) -> Result<String, ScriptLangError> {
    let enum_lookup = visible_types
        .iter()
        .filter_map(|(name, ty)| match ty {
            ScriptType::Enum { members, .. } => Some((name.as_str(), members)),
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    if enum_lookup.is_empty() {
        return Ok(expr.to_string());
    }

    let chars = expr.chars().collect::<Vec<_>>();
    let mut out = String::with_capacity(expr.len());
    let mut index = 0usize;

    while index < chars.len() {
        let ch = chars[index];
        if ch == '"' || ch == '\'' {
            out.push(ch);
            index += 1;
            while index < chars.len() {
                let inner = chars[index];
                out.push(inner);
                index += 1;
                if inner == '\\' && index < chars.len() {
                    out.push(chars[index]);
                    index += 1;
                    continue;
                }
                if inner == ch {
                    break;
                }
            }
            continue;
        }

        if ch.is_ascii_alphabetic() || ch == '_' {
            let start = index;
            index += 1;
            while index < chars.len() {
                let c = chars[index];
                if c.is_ascii_alphanumeric() || c == '_' || c == '.' {
                    index += 1;
                } else {
                    break;
                }
            }
            let token = chars[start..index].iter().collect::<String>();
            if let Some((type_name, member_name)) = token.rsplit_once('.') {
                if let Some(members) = enum_lookup.get(type_name) {
                    if !members.iter().any(|member| member == member_name) {
                        return Err(ScriptLangError::with_span(
                            "ENUM_LITERAL_MEMBER_UNKNOWN",
                            format!(
                                "Unknown enum member \"{}\" for type \"{}\".",
                                member_name, type_name
                            ),
                            span.clone(),
                        ));
                    }
                    out.push(quote);
                    out.push_str(member_name);
                    out.push(quote);
                    continue;
                }
            }
            out.push_str(&token);
            continue;
        }

        out.push(ch);
        index += 1;
    }

    Ok(out)
}

pub(crate) fn rewrite_and_validate_enum_literals_in_template(
    template: &str,
    visible_types: &BTreeMap<String, ScriptType>,
    span: &SourceSpan,
) -> Result<String, ScriptLangError> {
    let mut out = String::new();
    let mut last_index = 0usize;
    for captures in enum_template_regex().captures_iter(template) {
        let full = captures
            .get(0)
            .expect("capture group 0 must exist for each template capture");
        let expr = captures
            .get(1)
            .expect("capture group 1 must exist for each template capture");
        out.push_str(&template[last_index..full.start()]);
        let rewritten =
            rewrite_and_validate_enum_literals_in_expression(expr.as_str(), visible_types, span)?;
        out.push_str("${");
        out.push_str(&rewritten);
        out.push('}');
        last_index = full.end();
    }
    out.push_str(&template[last_index..]);
    Ok(out)
}

pub(crate) fn parse_enum_literal_initializer(
    expr: &str,
    enum_type_name: &str,
    enum_members: &[String],
    visible_types: &BTreeMap<String, ScriptType>,
    span: &SourceSpan,
) -> Result<String, ScriptLangError> {
    let trimmed = expr.trim();
    if trimmed.starts_with('"') || trimmed.starts_with('\'') {
        return Err(ScriptLangError::with_span(
            "ENUM_LITERAL_REQUIRED",
            format!(
                "Enum \"{}\" initializer must use Type.Member literal, not string literal.",
                enum_type_name
            ),
            span.clone(),
        ));
    }

    let Some((type_name, member_name)) = trimmed.rsplit_once('.') else {
        return Err(ScriptLangError::with_span(
            "ENUM_LITERAL_REQUIRED",
            format!(
                "Enum \"{}\" initializer must use Type.Member literal.",
                enum_type_name
            ),
            span.clone(),
        ));
    };

    if !enum_members.iter().any(|member| member == member_name) {
        return Err(ScriptLangError::with_span(
            "ENUM_LITERAL_MEMBER_UNKNOWN",
            format!(
                "Unknown enum member \"{}\" for type \"{}\".",
                member_name, type_name
            ),
            span.clone(),
        ));
    }

    let type_matches = visible_types.iter().any(|(name, ty)| {
        name == type_name
            && matches!(
                ty,
                ScriptType::Enum {
                    type_name: declared_type_name,
                    members,
                } if declared_type_name == enum_type_name && members == enum_members
            )
    });
    if !type_matches {
        return Err(ScriptLangError::with_span(
            "ENUM_LITERAL_REQUIRED",
            format!(
                "Enum \"{}\" initializer must use Type.Member literal of the same enum type.",
                enum_type_name
            ),
            span.clone(),
        ));
    }

    Ok(member_name.to_string())
}

pub(crate) fn validate_enum_map_initializer_keys_if_static(
    expr: &str,
    enum_type_name: &str,
    enum_members: &[String],
    span: &SourceSpan,
) -> Result<(), ScriptLangError> {
    let trimmed = expr.trim();
    let Some(inner) = trimmed
        .strip_prefix("#{")
        .and_then(|content| content.strip_suffix('}'))
    else {
        return Ok(());
    };
    if inner.trim().is_empty() {
        return Ok(());
    }

    for entry in split_by_top_level_comma(inner) {
        let Some(key_raw) = extract_map_literal_key_expr(&entry) else {
            continue;
        };
        let Some(key) = decode_static_map_key(key_raw) else {
            continue;
        };
        if !enum_members.iter().any(|member| member == &key) {
            return Err(ScriptLangError::with_span(
                "ENUM_MAP_KEY_UNKNOWN",
                format!(
                    "Unknown map key \"{}\" for enum key type \"{}\".",
                    key, enum_type_name
                ),
                span.clone(),
            ));
        }
    }

    Ok(())
}

fn extract_map_literal_key_expr(entry: &str) -> Option<&str> {
    let chars = entry.char_indices().collect::<Vec<_>>();
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut brace_depth = 0usize;
    let mut quote: Option<char> = None;
    let mut idx = 0usize;

    while idx < chars.len() {
        let (offset, ch) = chars[idx];
        if let Some(active_quote) = quote {
            if ch == '\\' {
                idx += 2;
                continue;
            }
            if ch == active_quote {
                quote = None;
            }
            idx += 1;
            continue;
        }

        if ch == '\'' || ch == '"' {
            quote = Some(ch);
            idx += 1;
            continue;
        }

        match ch {
            '(' => paren_depth += 1,
            ')' if paren_depth > 0 => paren_depth -= 1,
            '[' => bracket_depth += 1,
            ']' if bracket_depth > 0 => bracket_depth -= 1,
            '{' => brace_depth += 1,
            '}' if brace_depth > 0 => brace_depth -= 1,
            ':' if paren_depth == 0 && bracket_depth == 0 && brace_depth == 0 => {
                return Some(entry[..offset].trim());
            }
            _ => {}
        }
        idx += 1;
    }
    None
}

fn decode_static_map_key(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if type_name_regex().is_match(trimmed) && !trimmed.contains('.') {
        return Some(trimmed.to_string());
    }

    let mut chars = trimmed.chars();
    // SAFETY: trimmed is non-empty and not a valid type name, so it must have at least one char
    let first = chars.next().expect("trimmed should not be empty here");
    if (first == '"' || first == '\'') && trimmed.ends_with(first) && trimmed.len() >= 2 {
        return Some(trimmed[1..trimmed.len() - 1].to_string());
    }

    None
}

#[cfg(test)]
mod xml_utils_tests {
    use super::*;
    use crate::compiler_test_support::*;

    fn parsed_type_kind(expr: ParsedTypeExpr) -> &'static str {
        match expr {
            ParsedTypeExpr::Primitive(_) => "primitive",
            ParsedTypeExpr::Script => "script",
            ParsedTypeExpr::Function => "function",
            ParsedTypeExpr::Array(_) => "array",
            ParsedTypeExpr::Map { .. } => "map",
            ParsedTypeExpr::Custom(_) => "custom",
        }
    }

    #[test]
    fn parse_type_and_call_argument_helpers_cover_valid_and_invalid_inputs() {
        let span = SourceSpan::synthetic();
        let primitive = parse_type_expr("int", &span).expect("primitive");
        let script = parse_type_expr("script", &span).expect("script");
        let function = parse_type_expr("function", &span).expect("function");
        let array = parse_type_expr("int[]", &span).expect("array");
        let map = parse_type_expr("#{int}", &span).expect("map");
        let map_with_key = parse_type_expr("#{State=>int}", &span).expect("map with key");
        let custom = parse_type_expr("CustomType", &span).expect("custom");
        assert_eq!(parsed_type_kind(primitive), "primitive");
        assert_eq!(parsed_type_kind(script), "script");
        assert_eq!(parsed_type_kind(function), "function");
        assert_eq!(parsed_type_kind(array), "array");
        assert_eq!(parsed_type_kind(map), "map");
        assert_eq!(parsed_type_kind(map_with_key), "map");
        assert_eq!(parsed_type_kind(custom), "custom");
        let invalid_type = parse_type_expr("Map<int,string>", &span).expect_err("invalid");
        assert_eq!(invalid_type.code, "TYPE_PARSE_ERROR");
        let empty_map_type = parse_type_expr("#{   }", &span).expect_err("empty map type");
        assert_eq!(empty_map_type.code, "TYPE_PARSE_ERROR");
        let bad_array_elem = parse_type_expr("[]", &span).expect_err("invalid nested array type");
        assert_eq!(bad_array_elem.code, "TYPE_PARSE_ERROR");
        let bad_map_value = parse_type_expr("#{[]}", &span).expect_err("invalid map value type");
        assert_eq!(bad_map_value.code, "TYPE_PARSE_ERROR");

        // Test empty key in map type - covers split_map_type_key_value returning None (line 103)
        let empty_key = parse_type_expr("#{=>int}", &span).expect_err("empty key");
        assert_eq!(empty_key.code, "TYPE_PARSE_ERROR");

        // Test empty value in map type - covers split_map_type_key_value returning None
        let empty_value = parse_type_expr("#{State=>}", &span).expect_err("empty value");
        assert_eq!(empty_value.code, "TYPE_PARSE_ERROR");

        // Test invalid key type - covers parse_type_expr error propagation for key type (line 33-34)
        // When split_map_type_key_value returns Some but parsing key type fails
        let invalid_key_type =
            parse_type_expr("#{{invalid}=>int}", &span).expect_err("invalid key type");
        assert_eq!(invalid_key_type.code, "TYPE_PARSE_ERROR");

        // Test invalid value type - covers parse_type_expr error propagation for value type (line 34)
        // When split_map_type_key_value returns Some but parsing value type fails
        let invalid_value_type =
            parse_type_expr("#{State=>[invalid]}", &span).expect_err("invalid value type");
        assert_eq!(invalid_value_type.code, "TYPE_PARSE_ERROR");

        let args = parse_args(Some("1, ref:hp, a + 1".to_string())).expect("args");
        assert_eq!(args.len(), 3);
        assert!(args[1].is_ref);

        let bad_args = parse_args(Some("ref:   ".to_string())).expect_err("bad args");
        assert_eq!(bad_args.code, "CALL_ARGS_PARSE_ERROR");
    }

    #[test]
    fn split_map_type_key_value_handles_nested_quotes_and_invalid_edges() {
        assert_eq!(
            split_map_type_key_value("string=>int"),
            Some(("string", "int"))
        );
        assert_eq!(
            split_map_type_key_value("string=>\"x=>y\""),
            Some(("string", "\"x=>y\""))
        );
        assert_eq!(
            split_map_type_key_value("string=>#{int=>int}"),
            Some(("string", "#{int=>int}"))
        );
        assert_eq!(split_map_type_key_value("=>int"), None);
        assert_eq!(split_map_type_key_value("string=>"), None);
        // Test with parentheses BEFORE => in key part - covers depth handling in lines 90-91
        // Key has balanced parentheses: (a)=>value
        assert_eq!(
            split_map_type_key_value("(a)=>value"),
            Some(("(a)", "value"))
        );
        // Test escaped backslash in KEY's quoted string - the \\ in "a\\b" triggers the escape branch (lines 72-74)
        // Input: "a\b"=>value (where \ is escaped in Rust string, so actual chars are: ", a, \, b, ", =>, value)
        let result = split_map_type_key_value("\"a\\b\"=>value");
        assert!(result.is_some());
        let (left, right) = result.unwrap();
        assert_eq!(right, "value");
        // left should be the quoted key "a\b" (the function preserves quotes in the key)
        assert!(left.starts_with('"'));
        assert!(left.ends_with('"'));

        // Test with escaped quote inside string to cover line 76 (quote = None)
        let result2 = split_map_type_key_value(r#"string=>"test\"value""#);
        assert_eq!(result2, Some(("string", r#""test\"value""#)));

        // Test single quotes to cover line 83-86
        let result3 = split_map_type_key_value("'key'=>'value'");
        assert_eq!(result3, Some(("'key'", "'value'")));

        // Test with nested brackets inside quotes (lines 89-95)
        let result4 = split_map_type_key_value(r#"string=>"a[b]c""#);
        assert_eq!(result4, Some(("string", r#""a[b]c""#)));

        // Test with nested braces inside quotes
        let result5 = split_map_type_key_value(r#"string=>"a{b}c""#);
        assert_eq!(result5, Some(("string", r#""a{b}c""#)));

        // Test with nested parens inside quotes
        let result6 = split_map_type_key_value(r#"string=>"a(b)c""#);
        assert_eq!(result6, Some(("string", r#""a(b)c""#)));

        // Test backslash escape at end of quoted string (to cover idx += 2 at line 73)
        let result7 = split_map_type_key_value(r#"string=>"test\\""#);
        assert_eq!(result7, Some(("string", r#""test\\""#)));

        // Test backslash followed by regular char inside quotes
        let result8 = split_map_type_key_value(r#"string=>"a\nb""#);
        assert_eq!(result8, Some(("string", r#""a\nb""#)));

        // Test parentheses outside quotes to cover line 91 guard (paren_depth > 0)
        let result9 = split_map_type_key_value("string=>(a)");
        assert_eq!(result9, Some(("string", "(a)")));

        // Test brackets outside quotes to cover line 93 guard
        let result10 = split_map_type_key_value("string=>[a]");
        assert_eq!(result10, Some(("string", "[a]")));

        // Test braces outside quotes to cover line 95 guard
        let result11 = split_map_type_key_value("string=>{a}");
        assert_eq!(result11, Some(("string", "{a}")));

        // Test unbalanced paren at start (should return None because => not found)
        let result12 = split_map_type_key_value("string=>a)");
        assert_eq!(result12, Some(("string", "a)")));

        // Test unbalanced bracket at start
        let result13 = split_map_type_key_value("string=>a]");
        assert_eq!(result13, Some(("string", "a]")));
    }

    #[test]
    fn split_by_top_level_comma_covers_trailing_part() {
        // This specifically tests the code path at line 196
        // that handles the last part after all commas are processed
        let result = split_by_top_level_comma("a,b,c");
        assert_eq!(
            result,
            vec!["a".to_string(), "b".to_string(), "c".to_string()]
        );

        // Edge case: single element (no comma)
        let single = split_by_top_level_comma("only");
        assert_eq!(single, vec!["only".to_string()]);

        // Edge case: empty string
        let empty = split_by_top_level_comma("");
        assert!(empty.is_empty());
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
        assert_eq!(
            split_by_top_level_comma("a,b"),
            vec!["a".to_string(), "b".to_string()]
        );
        // Test case without trailing comma - covers line 196
        assert_eq!(
            split_by_top_level_comma("a,b,c"),
            vec!["a".to_string(), "b".to_string(), "c".to_string()]
        );
    }

    #[test]
    fn parse_bool_attr_default_and_false_paths_are_covered() {
        let node = xml_element("text", &[], vec![xml_text("x")]);
        assert!(parse_bool_attr(&node, "once", true).expect("default should apply"));

        let false_node = xml_element("text", &[("once", "false")], vec![xml_text("x")]);
        assert!(!parse_bool_attr(&false_node, "once", true).expect("false attr"));
    }

    #[test]
    fn module_symbol_alias_rewrite_helpers_cover_expression_and_template_paths() {
        let hp_decl = ModuleVarDecl {
            namespace: "main".to_string(),
            name: "hp".to_string(),
            qualified_name: "main.hp".to_string(),
            access: AccessLevel::Public,
            r#type: ScriptType::Primitive {
                name: "int".to_string(),
            },
            initial_value_expr: None,
            location: SourceSpan::synthetic(),
        };
        let base_decl = ModuleConstDecl {
            namespace: "main".to_string(),
            name: "BASE".to_string(),
            qualified_name: "main.BASE".to_string(),
            access: AccessLevel::Public,
            r#type: ScriptType::Primitive {
                name: "int".to_string(),
            },
            initial_value_expr: None,
            location: SourceSpan::synthetic(),
        };
        let visible_module_vars = BTreeMap::from([
            ("hp".to_string(), hp_decl.clone()),
            ("main.hp".to_string(), hp_decl),
            (
                "wallet".to_string(),
                ModuleVarDecl {
                    namespace: "main".to_string(),
                    name: "wallet".to_string(),
                    qualified_name: "main.wallet".to_string(),
                    access: AccessLevel::Public,
                    r#type: ScriptType::Map {
                        key_type: MapKeyType::String,
                        value_type: Box::new(ScriptType::Primitive {
                            name: "int".to_string(),
                        }),
                    },
                    initial_value_expr: None,
                    location: SourceSpan::synthetic(),
                },
            ),
        ]);
        let visible_module_consts = BTreeMap::from([
            ("base".to_string(), base_decl.clone()),
            ("main.BASE".to_string(), base_decl),
        ]);
        let map = build_module_symbol_alias_rewrite_map(
            &visible_module_vars,
            &visible_module_consts,
            &BTreeSet::new(),
        );

        let rewritten = rewrite_module_symbol_aliases_in_expression(
            r#"hp + base + #{hp: hp, val: base} + "hp" + @main.next + *main.fn"#,
            &map,
        );
        assert_eq!(
            rewritten,
            r#"main.hp + main.BASE + #{hp: main.hp, val: main.BASE} + "hp" + @main.next + *main.fn"#
        );
        let dotted = rewrite_module_symbol_aliases_in_expression("wallet.gold + base", &map);
        assert_eq!(dotted, "main.wallet.gold + main.BASE");

        let template = rewrite_module_symbol_aliases_in_template("hp=${hp},base=${base}", &map);
        assert_eq!(template, "hp=${main.hp},base=${main.BASE}");
    }

    #[test]
    fn rewrite_enum_literals_covers_empty_visible_types() {
        // Test when visible_types is empty (enum_lookup.is_empty() branch)
        let span = SourceSpan::synthetic();
        let empty_types: BTreeMap<String, ScriptType> = BTreeMap::new();
        let result =
            rewrite_and_validate_enum_literals_in_expression("Color.Red", &empty_types, &span)
                .expect("should succeed with empty types");
        assert_eq!(result, "Color.Red");
    }

    #[test]
    fn rewrite_enum_literals_covers_non_enum_types() {
        // Test when visible_types contains non-enum types (the _ => None branch)
        let span = SourceSpan::synthetic();
        let mut types = BTreeMap::new();
        types.insert(
            "MyInt".to_string(),
            ScriptType::Primitive {
                name: "int".to_string(),
            },
        );
        let result = rewrite_and_validate_enum_literals_in_expression("Color.Red", &types, &span)
            .expect("should succeed with primitive types");
        assert_eq!(result, "Color.Red");
    }

    #[test]
    fn rewrite_enum_literals_covers_valid_enum_member() {
        // Test valid enum member access (Color.Red -> "Red")
        let span = SourceSpan::synthetic();
        let mut types = BTreeMap::new();
        types.insert(
            "Color".to_string(),
            ScriptType::Enum {
                type_name: "Color".to_string(),
                members: vec!["Red".to_string(), "Green".to_string(), "Blue".to_string()],
            },
        );
        let result = rewrite_and_validate_enum_literals_in_expression("Color.Red", &types, &span)
            .expect("should succeed with valid enum member");
        assert_eq!(result, "\"Red\"");
    }

    #[test]
    fn rewrite_enum_literals_in_attr_expression_uses_single_quote() {
        let span = SourceSpan::synthetic();
        let mut types = BTreeMap::new();
        types.insert(
            "Color".to_string(),
            ScriptType::Enum {
                type_name: "Color".to_string(),
                members: vec!["Red".to_string()],
            },
        );
        let result = rewrite_and_validate_enum_literals_in_attr_expression(
            "Color.Red == 'Red'",
            &types,
            &span,
        )
        .expect("attr expression rewrite should pass");
        assert_eq!(result, "'Red' == 'Red'");
    }

    #[test]
    fn rewrite_enum_literals_covers_invalid_enum_member() {
        // Test invalid enum member access (error case)
        let span = SourceSpan::synthetic();
        let mut types = BTreeMap::new();
        types.insert(
            "Color".to_string(),
            ScriptType::Enum {
                type_name: "Color".to_string(),
                members: vec!["Red".to_string(), "Green".to_string()],
            },
        );
        let err = rewrite_and_validate_enum_literals_in_expression("Color.Invalid", &types, &span)
            .expect_err("should fail with invalid enum member");
        assert_eq!(err.code, "ENUM_LITERAL_MEMBER_UNKNOWN");
    }

    #[test]
    fn rewrite_enum_literals_covers_string_literals() {
        // Test string literal handling (single and double quotes)
        let span = SourceSpan::synthetic();
        let mut types = BTreeMap::new();
        types.insert(
            "Color".to_string(),
            ScriptType::Enum {
                type_name: "Color".to_string(),
                members: vec!["Red".to_string()],
            },
        );
        // String literal should not be rewritten
        let result =
            rewrite_and_validate_enum_literals_in_expression("\"Color.Red\"", &types, &span)
                .expect("should preserve string literal");
        assert_eq!(result, "\"Color.Red\"");

        // Single quote string
        let result = rewrite_and_validate_enum_literals_in_expression("'Color.Red'", &types, &span)
            .expect("should preserve single quoted string");
        assert_eq!(result, "'Color.Red'");
    }

    #[test]
    fn rewrite_enum_literals_covers_escape_characters() {
        // Test escape character handling in strings
        let span = SourceSpan::synthetic();
        let mut types = BTreeMap::new();
        types.insert(
            "Status".to_string(),
            ScriptType::Enum {
                type_name: "Status".to_string(),
                members: vec!["OK".to_string()],
            },
        );
        // String with escape sequence
        let result =
            rewrite_and_validate_enum_literals_in_expression("\"hello\\\"world\"", &types, &span)
                .expect("should handle escaped quotes");
        assert_eq!(result, "\"hello\\\"world\"");

        // String with backslash
        let result =
            rewrite_and_validate_enum_literals_in_expression("a + \"test\\\\path\"", &types, &span)
                .expect("should handle backslash in string");
        assert_eq!(result, "a + \"test\\\\path\"");
    }

    #[test]
    fn rewrite_enum_literals_covers_mixed_content() {
        // Test mixed content: string + enum reference
        let span = SourceSpan::synthetic();
        let mut types = BTreeMap::new();
        types.insert(
            "Color".to_string(),
            ScriptType::Enum {
                type_name: "Color".to_string(),
                members: vec!["Red".to_string(), "Blue".to_string()],
            },
        );
        // Expression with both string and enum
        let result = rewrite_and_validate_enum_literals_in_expression(
            "x + \"prefix\" + Color.Red + \"suffix\"",
            &types,
            &span,
        )
        .expect("should handle mixed content");
        assert_eq!(result, "x + \"prefix\" + \"Red\" + \"suffix\"");
    }

    #[test]
    fn rewrite_enum_literals_covers_identifier_with_underscore() {
        // Test identifier with underscore
        let span = SourceSpan::synthetic();
        let mut types = BTreeMap::new();
        types.insert(
            "MyEnum".to_string(),
            ScriptType::Enum {
                type_name: "MyEnum".to_string(),
                members: vec!["Member_One".to_string()],
            },
        );
        let result =
            rewrite_and_validate_enum_literals_in_expression("MyEnum.Member_One", &types, &span)
                .expect("should handle underscore in identifiers");
        assert_eq!(result, "\"Member_One\"");
    }

    #[test]
    fn rewrite_enum_literals_template_covers_basic_usage() {
        // Test rewrite_and_validate_enum_literals_in_template
        let span = SourceSpan::synthetic();
        let mut types = BTreeMap::new();
        types.insert(
            "Color".to_string(),
            ScriptType::Enum {
                type_name: "Color".to_string(),
                members: vec!["Red".to_string(), "Blue".to_string()],
            },
        );
        let result = rewrite_and_validate_enum_literals_in_template(
            "The color is ${Color.Red} and ${Color.Blue}",
            &types,
            &span,
        )
        .expect("should handle template with enum literals");
        assert_eq!(result, "The color is ${\"Red\"} and ${\"Blue\"}");
    }

    #[test]
    fn rewrite_enum_literals_template_covers_no_enum_in_template() {
        // Test template without enum references
        let span = SourceSpan::synthetic();
        let mut types = BTreeMap::new();
        types.insert(
            "Color".to_string(),
            ScriptType::Enum {
                type_name: "Color".to_string(),
                members: vec!["Red".to_string()],
            },
        );
        let result =
            rewrite_and_validate_enum_literals_in_template("Just a regular string", &types, &span)
                .expect("should handle template without enum");
        assert_eq!(result, "Just a regular string");
    }

    #[test]
    fn rewrite_enum_literals_template_covers_multiple_captures() {
        // Test template with multiple enum references
        let span = SourceSpan::synthetic();
        let mut types = BTreeMap::new();
        types.insert(
            "Status".to_string(),
            ScriptType::Enum {
                type_name: "Status".to_string(),
                members: vec!["Pending".to_string(), "Done".to_string()],
            },
        );
        let result = rewrite_and_validate_enum_literals_in_template(
            "${Status.Pending} -> ${Status.Done}",
            &types,
            &span,
        )
        .expect("should handle multiple enum refs");
        assert_eq!(result, "${\"Pending\"} -> ${\"Done\"}");
    }

    #[test]
    fn parse_enum_literal_covers_string_literal_error() {
        // Test error when initializer is a string literal instead of Type.Member
        let span = SourceSpan::synthetic();
        let members = vec!["Red".to_string(), "Green".to_string()];
        let types = BTreeMap::new();

        // Double-quoted string
        let err = parse_enum_literal_initializer("\"Red\"", "Color", &members, &types, &span)
            .expect_err("should fail with string literal");
        assert_eq!(err.code, "ENUM_LITERAL_REQUIRED");
        assert!(err.message.contains("not string literal"));

        // Single-quoted string
        let err = parse_enum_literal_initializer("'Red'", "Color", &members, &types, &span)
            .expect_err("should fail with single-quoted string");
        assert_eq!(err.code, "ENUM_LITERAL_REQUIRED");
    }

    #[test]
    fn parse_enum_literal_covers_missing_dot_error() {
        // Test error when initializer doesn't have a dot (Type.Member)
        let span = SourceSpan::synthetic();
        let members = vec!["Red".to_string()];
        let types = BTreeMap::new();

        let err = parse_enum_literal_initializer("Red", "Color", &members, &types, &span)
            .expect_err("should fail without dot");
        assert_eq!(err.code, "ENUM_LITERAL_REQUIRED");
        assert!(err.message.contains("Type.Member"));
    }

    #[test]
    fn parse_enum_literal_covers_unknown_member_error() {
        // Test error when member name is not in the enum
        let span = SourceSpan::synthetic();
        let members = vec!["Red".to_string(), "Green".to_string()];
        let types = BTreeMap::new();

        let err = parse_enum_literal_initializer("Color.Invalid", "Color", &members, &types, &span)
            .expect_err("should fail with unknown member");
        assert_eq!(err.code, "ENUM_LITERAL_MEMBER_UNKNOWN");
        assert!(err.message.contains("Invalid"));
    }

    #[test]
    fn parse_enum_literal_covers_type_mismatch_error() {
        // Test error when type name doesn't match the enum type
        // The code checks:
        // 1. Not a string literal
        // 2. Has dot separator
        // 3. Member exists in enum_members
        // 4. Type name matches the declared enum_type_name
        let span = SourceSpan::synthetic();
        let members = vec!["Red".to_string()];
        let mut types = BTreeMap::new();
        // Add a different enum type with the same member name
        types.insert(
            "OtherColor".to_string(),
            ScriptType::Enum {
                type_name: "OtherColor".to_string(),
                members: vec!["Red".to_string()],
            },
        );

        // This should fail at step 4 (type_matches) because:
        // - "OtherColor.Red" has dot separator
        // - "Red" is in our members list
        // - But "OtherColor" != "Color" (enum_type_name)
        let err = parse_enum_literal_initializer(
            "OtherColor.Red",
            "Color",  // Different type name than "OtherColor"
            &members, // "Red" is in this list
            &types,
            &span,
        )
        .expect_err("should fail with type mismatch");
        assert_eq!(err.code, "ENUM_LITERAL_REQUIRED");
        assert!(err.message.contains("same enum type"));
    }

    #[test]
    fn parse_enum_literal_covers_valid_initializer() {
        // Test valid enum literal initializer
        let span = SourceSpan::synthetic();
        let members = vec!["Red".to_string(), "Green".to_string()];
        let mut types = BTreeMap::new();
        types.insert(
            "Color".to_string(),
            ScriptType::Enum {
                type_name: "Color".to_string(),
                members: members.clone(),
            },
        );

        let result = parse_enum_literal_initializer("Color.Red", "Color", &members, &types, &span)
            .expect("should succeed with valid initializer");
        assert_eq!(result, "Red");
    }

    #[test]
    fn rewrite_enum_literals_covers_type_not_in_lookup() {
        // Test when token has '.' but type_name is not in enum_lookup
        // This covers line 377 (the else branch of if let Some(members))
        let span = SourceSpan::synthetic();
        let mut types = BTreeMap::new();
        types.insert(
            "Color".to_string(),
            ScriptType::Enum {
                type_name: "Color".to_string(),
                members: vec!["Red".to_string()],
            },
        );
        // Expression with a type that's NOT in the enum_lookup
        let result = rewrite_and_validate_enum_literals_in_expression(
            "UnknownType.Member + Color.Red",
            &types,
            &span,
        )
        .expect("should succeed - UnknownType is passed through");
        // UnknownType.Member should pass through unchanged
        assert_eq!(result, "UnknownType.Member + \"Red\"");
    }

    #[test]
    fn rewrite_enum_template_covers_error_propagation() {
        // Test when template expression causes an error (covers line 406 ? operator)
        let span = SourceSpan::synthetic();
        let mut types = BTreeMap::new();
        types.insert(
            "Color".to_string(),
            ScriptType::Enum {
                type_name: "Color".to_string(),
                members: vec!["Red".to_string()], // Only Red is valid
            },
        );
        // Template with invalid enum member should propagate error
        let err = rewrite_and_validate_enum_literals_in_template(
            "Color is ${Color.Invalid}",
            &types,
            &span,
        )
        .expect_err("should fail with invalid enum member in template");
        assert_eq!(err.code, "ENUM_LITERAL_MEMBER_UNKNOWN");
    }

    #[test]
    fn enum_map_initializer_key_validation_covers_valid_and_invalid_keys() {
        let span = SourceSpan::synthetic();
        let members = vec!["A".to_string(), "B".to_string()];
        validate_enum_map_initializer_keys_if_static(
            "#{A: 1, B: 2}",
            "ids.LocationId",
            &members,
            &span,
        )
        .expect("valid map keys should pass");

        let error = validate_enum_map_initializer_keys_if_static(
            "#{A: 1, X: 2}",
            "ids.LocationId",
            &members,
            &span,
        )
        .expect_err("invalid key should fail");
        assert_eq!(error.code, "ENUM_MAP_KEY_UNKNOWN");

        validate_enum_map_initializer_keys_if_static(
            "make_map()",
            "ids.LocationId",
            &members,
            &span,
        )
        .expect("non-static expression should skip compile-time validation");

        // Test quoted keys - covers decode_static_map_key quote handling (line 752-753)
        validate_enum_map_initializer_keys_if_static(
            "#{\"A\": 1, \"B\": 2}",
            "ids.LocationId",
            &members,
            &span,
        )
        .expect("valid quoted map keys should pass");

        // Test single-quoted keys - covers single quote handling
        validate_enum_map_initializer_keys_if_static(
            "#{'A': 1, 'B': 2}",
            "ids.LocationId",
            &members,
            &span,
        )
        .expect("valid single-quoted map keys should pass");

        // Test map with colon in value (not key) - covers bracket/brace depth handling
        validate_enum_map_initializer_keys_if_static(
            "#{A: #{X: 1}}",
            "ids.LocationId",
            &members,
            &span,
        )
        .expect("nested map should pass validation");

        // Test map with array value containing colon - covers bracket depth
        validate_enum_map_initializer_keys_if_static(
            "#{A: [1, 2]}",
            "ids.LocationId",
            &members,
            &span,
        )
        .expect("array value should pass");

        // Test empty map literal - covers empty check at line 744
        validate_enum_map_initializer_keys_if_static("#{}", "ids.LocationId", &members, &span)
            .expect("empty map should pass");

        // Test key with colon in parentheses - covers paren depth at line 725-726
        validate_enum_map_initializer_keys_if_static(
            "#{A: (x: 1)}",
            "ids.LocationId",
            &members,
            &span,
        )
        .expect("parenthesized value should pass");

        // Test key with quoted string - covers quote handling in extract_map_literal_key_expr (lines 724-727)
        // Key is a quoted string, not the value
        validate_enum_map_initializer_keys_if_static(
            "#{\"A\": 1}",
            "ids.LocationId",
            &members,
            &span,
        )
        .expect("quoted key should pass");

        // Test entry without colon (no key:value pair) - covers continue at line 679 when extract returns None
        validate_enum_map_initializer_keys_if_static(
            "#{A: 1, some_expr}",
            "ids.LocationId",
            &members,
            &span,
        )
        .expect("non-key-value expression should be skipped");

        // Test key with escaped backslash inside quoted string - covers escape character handling in extract_map_literal_key_expr (lines 710-712)
        // This exercises the ch == '\\' branch when processing escaped characters inside quotes
        // Using a key that doesn't exist in enum members to test the path
        validate_enum_map_initializer_keys_if_static(
            r#"#{"A\\C": 1}"#,
            "ids.LocationId",
            &members,
            &span,
        )
        .expect_err("non-existent key should fail");

        // Test numeric key - covers decode_static_map_key returning None (line 679)
        // "123" is extracted as key but decode_static_map_key returns None for non-identifier strings
        validate_enum_map_initializer_keys_if_static(
            "#{123: 1}",
            "ids.LocationId",
            &members,
            &span,
        )
        .expect("numeric key should be skipped");

        // Test key with only whitespace - covers decode_static_map_key returning None at line 744
        // when trimmed is empty
        validate_enum_map_initializer_keys_if_static("#{  : 1}", "ids.LocationId", &members, &span)
            .expect("whitespace-only key should be skipped");

        // Test key with dot (qualified name) - should be skipped by decode_static_map_key
        // as it returns early when type_name_regex matches but contains '.'
        validate_enum_map_initializer_keys_if_static(
            "#{A.B: 1}",
            "ids.LocationId",
            &members,
            &span,
        )
        .expect("qualified name key should be skipped");

        // Test key with parentheses - covers paren depth handling in extract_map_literal_key_expr (lines 728-729)
        // This is a valid identifier so it will pass validation
        validate_enum_map_initializer_keys_if_static(
            "#{(foo): 1}",
            "ids.LocationId",
            &members,
            &span,
        )
        .expect("parenthesized key should pass");

        // Test key with brackets - covers bracket depth handling in extract_map_literal_key_expr (lines 730-731)
        validate_enum_map_initializer_keys_if_static(
            "#{[foo]: 1}",
            "ids.LocationId",
            &members,
            &span,
        )
        .expect("bracketed key should pass");

        // Test key with braces - covers brace depth handling in extract_map_literal_key_expr (lines 732-733)
        validate_enum_map_initializer_keys_if_static(
            "#{{foo}: 1}",
            "ids.LocationId",
            &members,
            &span,
        )
        .expect("braced key should pass");
    }

    #[test]
    fn declaration_name_keyword_guard_is_case_sensitive() {
        let span = SourceSpan::synthetic();
        let keyword = assert_decl_name_not_reserved_or_rhai_keyword("shared", "var", span.clone())
            .expect_err("shared should be rejected");
        assert_eq!(keyword.code, "NAME_RHAI_KEYWORD_RESERVED");

        assert_decl_name_not_reserved_or_rhai_keyword("Shared", "var", span)
            .expect("capitalized variant should pass");
    }
}
