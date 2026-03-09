use crate::*;

pub(crate) fn parse_type_expr(
    raw: &str,
    span: &SourceSpan,
) -> Result<ParsedTypeExpr, ScriptLangError> {
    let source = raw.trim();
    if source == "script" {
        return Ok(ParsedTypeExpr::Script);
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
        let value_type = parse_type_expr(value.trim(), span)?;
        return Ok(ParsedTypeExpr::Map(Box::new(value_type)));
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

pub(crate) fn rewrite_and_validate_enum_literals_in_expression(
    expr: &str,
    visible_types: &BTreeMap<String, ScriptType>,
    span: &SourceSpan,
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
                    out.push('"');
                    out.push_str(member_name);
                    out.push('"');
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

#[cfg(test)]
mod xml_utils_tests {
    use super::*;
    use crate::compiler_test_support::*;

    fn parsed_type_kind(expr: ParsedTypeExpr) -> &'static str {
        match expr {
            ParsedTypeExpr::Primitive(_) => "primitive",
            ParsedTypeExpr::Script => "script",
            ParsedTypeExpr::Array(_) => "array",
            ParsedTypeExpr::Map(_) => "map",
            ParsedTypeExpr::Custom(_) => "custom",
        }
    }

    #[test]
    fn parse_type_and_call_argument_helpers_cover_valid_and_invalid_inputs() {
        let span = SourceSpan::synthetic();
        let primitive = parse_type_expr("int", &span).expect("primitive");
        let script = parse_type_expr("script", &span).expect("script");
        let array = parse_type_expr("int[]", &span).expect("array");
        let map = parse_type_expr("#{int}", &span).expect("map");
        let custom = parse_type_expr("CustomType", &span).expect("custom");
        assert_eq!(parsed_type_kind(primitive), "primitive");
        assert_eq!(parsed_type_kind(script), "script");
        assert_eq!(parsed_type_kind(array), "array");
        assert_eq!(parsed_type_kind(map), "map");
        assert_eq!(parsed_type_kind(custom), "custom");
        let invalid_type = parse_type_expr("Map<int,string>", &span).expect_err("invalid");
        assert_eq!(invalid_type.code, "TYPE_PARSE_ERROR");
        let empty_map_type = parse_type_expr("#{   }", &span).expect_err("empty map type");
        assert_eq!(empty_map_type.code, "TYPE_PARSE_ERROR");
        let bad_array_elem = parse_type_expr("[]", &span).expect_err("invalid nested array type");
        assert_eq!(bad_array_elem.code, "TYPE_PARSE_ERROR");
        let bad_map_value = parse_type_expr("#{[]}", &span).expect_err("invalid map value type");
        assert_eq!(bad_map_value.code, "TYPE_PARSE_ERROR");

        let args = parse_args(Some("1, ref:hp, a + 1".to_string())).expect("args");
        assert_eq!(args.len(), 3);
        assert!(args[1].is_ref);

        let bad_args = parse_args(Some("ref:   ".to_string())).expect_err("bad args");
        assert_eq!(bad_args.code, "CALL_ARGS_PARSE_ERROR");
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
}
