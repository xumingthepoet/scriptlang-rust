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

    if type_name_regex().is_match(source) {
        return Ok(ParsedTypeExpr::Custom(source.to_string()));
    }

    Err(ScriptLangError::with_span(
        "TYPE_PARSE_ERROR",
        format!("Unsupported type syntax: \"{}\".", raw),
        span.clone(),
    ))
}

fn type_name_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)*$")
            .expect("type regex must compile")
    })
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

#[cfg(test)]
mod xml_utils_tests {
    use super::*;
    use crate::compiler_test_support::*;

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
        assert_eq!(split_by_top_level_comma("a,b"), vec!["a".to_string(), "b".to_string()]);
    }

}
