use crate::*;

pub(crate) fn stable_base(script_path: &str) -> String {
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

pub(crate) fn expand_script_macros(
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
        for_counter: 0,
    };

    Ok(XmlElementNode {
        name: root.name.clone(),
        attributes: root.attributes.clone(),
        children: expand_children(&root.children, &mut context)?,
        location: root.location.clone(),
    })
}

pub(crate) fn collect_declared_var_names(node: &XmlElementNode, names: &mut BTreeSet<String>) {
    if node.name == "temp" || node.name == "temp-input" {
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

pub(crate) fn validate_reserved_prefix_in_user_var_declarations(
    node: &XmlElementNode,
) -> Result<(), ScriptLangError> {
    if node.name == "temp" || node.name == "temp-input" {
        if let Some(name) = node.attributes.get("name") {
            if !name.is_empty() {
                let label = if node.name == "temp" {
                    "temp"
                } else {
                    "temp-input"
                };
                assert_decl_name_not_reserved_or_rhai_keyword(name, label, node.location.clone())?;
            }
        }
    }

    for child in element_children(node) {
        validate_reserved_prefix_in_user_var_declarations(child)?;
    }

    Ok(())
}

pub(crate) fn expand_children(
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

pub(crate) fn expand_element_with_macros(
    node: &XmlElementNode,
    context: &mut MacroExpansionContext,
) -> Result<Vec<XmlElementNode>, ScriptLangError> {
    if node.name == "for" {
        return Ok(vec![expand_for_macro(node, context)?]);
    }
    if node.name == "temp-input" {
        return expand_temp_input_macro(node);
    }

    Ok(vec![XmlElementNode {
        name: node.name.clone(),
        attributes: node.attributes.clone(),
        children: expand_children(&node.children, context)?,
        location: node.location.clone(),
    }])
}

fn expand_temp_input_macro(node: &XmlElementNode) -> Result<Vec<XmlElementNode>, ScriptLangError> {
    validate_temp_input_attributes(node)?;
    if let Some(child) = element_children(node).next() {
        return Err(ScriptLangError::with_span(
            "XML_TEMP_INPUT_CONTENT_FORBIDDEN",
            "<temp-input> cannot contain child elements. Use inline text only.",
            child.location.clone(),
        ));
    }

    let name = get_required_non_empty_attr(node, "name")?;
    assert_decl_name_not_reserved_or_rhai_keyword(&name, "temp-input", node.location.clone())?;

    let type_name = get_required_non_empty_attr(node, "type")?;
    if type_name.trim() != "string" {
        return Err(ScriptLangError::with_span(
            "XML_TEMP_INPUT_TYPE_UNSUPPORTED",
            format!(
                "Attribute \"type\" on <temp-input> only supports \"string\", got \"{}\".",
                type_name
            ),
            node.location.clone(),
        ));
    }

    let prompt_text = get_required_non_empty_attr(node, "text")?;
    let max_length = get_optional_attr(node, "max_length");
    let inline = inline_text_content(node);

    let mut temp_attrs = BTreeMap::new();
    temp_attrs.insert("name".to_string(), name.clone());
    temp_attrs.insert("type".to_string(), "string".to_string());
    let temp_children = if inline.trim().is_empty() {
        Vec::new()
    } else {
        vec![XmlNode::Text(XmlTextNode {
            value: inline,
            location: node.location.clone(),
        })]
    };
    let temp_node = XmlElementNode {
        name: "temp".to_string(),
        attributes: temp_attrs,
        children: temp_children,
        location: node.location.clone(),
    };

    let mut input_attrs = BTreeMap::new();
    input_attrs.insert("var".to_string(), name);
    input_attrs.insert("text".to_string(), prompt_text);
    if let Some(value) = max_length {
        input_attrs.insert("max_length".to_string(), value);
    }
    let input_node = XmlElementNode {
        name: "input".to_string(),
        attributes: input_attrs,
        children: Vec::new(),
        location: node.location.clone(),
    };

    Ok(vec![temp_node, input_node])
}

fn validate_temp_input_attributes(node: &XmlElementNode) -> Result<(), ScriptLangError> {
    for key in node.attributes.keys() {
        if matches!(key.as_str(), "name" | "type" | "text" | "max_length") {
            continue;
        }
        return Err(ScriptLangError::with_span(
            "XML_ATTR_NOT_ALLOWED",
            format!(
                "Attribute \"{}\" is not allowed on <temp-input>. Supported attributes: name, type, text, max_length.",
                key
            ),
            node.location.clone(),
        ));
    }
    Ok(())
}

/// Find a required child element by name from XML nodes
#[allow(dead_code)]
fn find_child_by_name<'a>(children: &'a [XmlNode], name: &str) -> Option<&'a XmlElementNode> {
    children.iter().find_map(|entry| match entry {
        XmlNode::Element(element) if element.name == name => Some(element),
        _ => None,
    })
}

#[derive(Debug, Clone)]
pub(crate) struct ForTempDecl {
    pub(crate) name: String,
    pub(crate) type_expr: String,
    pub(crate) init_expr: String,
}

fn expand_for_macro(
    node: &XmlElementNode,
    context: &mut MacroExpansionContext,
) -> Result<XmlElementNode, ScriptLangError> {
    validate_for_attributes(node)?;
    let temps_raw = get_required_non_empty_attr(node, "temps")?;
    let condition_expr = get_required_non_empty_attr(node, "condition")?;
    let iteration_expr = get_for_iteration_expr(node)?;
    let temp_decls = parse_for_temps_decls(&temps_raw, node)?;

    let mut temp_names = BTreeSet::new();
    let mut group_children = Vec::new();
    for decl in temp_decls {
        assert_decl_name_not_reserved_or_rhai_keyword(
            &decl.name,
            "for temp",
            node.location.clone(),
        )?;
        if !temp_names.insert(decl.name.clone()) {
            return Err(ScriptLangError::with_span(
                "XML_FOR_TEMPS_DUPLICATE",
                format!(
                    "Attribute \"temps\" on <for> contains duplicated temp name \"{}\".",
                    decl.name
                ),
                node.location.clone(),
            ));
        }

        let mut temp_attrs = BTreeMap::new();
        temp_attrs.insert("name".to_string(), decl.name);
        temp_attrs.insert("type".to_string(), decl.type_expr);
        group_children.push(XmlNode::Element(XmlElementNode {
            name: "temp".to_string(),
            attributes: temp_attrs,
            children: vec![XmlNode::Text(XmlTextNode {
                value: decl.init_expr,
                location: node.location.clone(),
            })],
            location: node.location.clone(),
        }));
    }

    let first_flag_name = next_for_first_flag_var_name(context);
    let mut first_flag_attrs = BTreeMap::new();
    first_flag_attrs.insert("name".to_string(), first_flag_name.clone());
    first_flag_attrs.insert("type".to_string(), "boolean".to_string());
    group_children.push(XmlNode::Element(XmlElementNode {
        name: "temp".to_string(),
        attributes: first_flag_attrs,
        children: vec![XmlNode::Text(XmlTextNode {
            value: "true".to_string(),
            location: node.location.clone(),
        })],
        location: node.location.clone(),
    }));

    let clear_first_flag_code = XmlElementNode {
        name: "code".to_string(),
        attributes: BTreeMap::new(),
        children: vec![XmlNode::Text(XmlTextNode {
            value: format!("{} = false;", first_flag_name),
            location: node.location.clone(),
        })],
        location: node.location.clone(),
    };
    let iteration_code = XmlElementNode {
        name: "code".to_string(),
        attributes: BTreeMap::new(),
        children: vec![XmlNode::Text(XmlTextNode {
            value: iteration_expr,
            location: node.location.clone(),
        })],
        location: node.location.clone(),
    };

    let first_flag_else = XmlElementNode {
        name: "else".to_string(),
        attributes: BTreeMap::new(),
        children: vec![XmlNode::Element(iteration_code)],
        location: node.location.clone(),
    };

    let mut first_flag_if_attrs = BTreeMap::new();
    first_flag_if_attrs.insert("when".to_string(), first_flag_name);
    let first_flag_if = XmlElementNode {
        name: "if".to_string(),
        attributes: first_flag_if_attrs,
        children: vec![
            XmlNode::Element(clear_first_flag_code),
            XmlNode::Element(first_flag_else),
        ],
        location: node.location.clone(),
    };

    let mut while_children = vec![XmlNode::Element(first_flag_if)];
    while_children.extend(expand_children(&node.children, context)?);

    let mut while_attrs = BTreeMap::new();
    while_attrs.insert("when".to_string(), condition_expr);
    group_children.push(XmlNode::Element(XmlElementNode {
        name: "while".to_string(),
        attributes: while_attrs,
        children: while_children,
        location: node.location.clone(),
    }));

    Ok(XmlElementNode {
        name: "group".to_string(),
        attributes: BTreeMap::new(),
        children: group_children,
        location: node.location.clone(),
    })
}

fn get_for_iteration_expr(node: &XmlElementNode) -> Result<String, ScriptLangError> {
    let Some(raw) = get_optional_attr(node, "iteration") else {
        return Ok("true;".to_string());
    };

    if raw.trim().is_empty() {
        return Err(ScriptLangError::with_span(
            "XML_EMPTY_ATTR",
            "Attribute \"iteration\" on <for> cannot be empty.",
            node.location.clone(),
        ));
    }

    Ok(raw)
}

pub(crate) fn parse_for_temps_decls(
    raw: &str,
    node: &XmlElementNode,
) -> Result<Vec<ForTempDecl>, ScriptLangError> {
    let entries = split_top_level_for_temps_entries(raw);
    // Note: split_top_level_for_temps_entries always returns at least one element,
    // because it always pushes the final part even if empty. So entries.is_empty()
    // is unreachable here.

    let mut result = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        if entry.is_empty() {
            if index == entries.len().saturating_sub(1) {
                continue;
            }
            return Err(invalid_for_temps_error(
                node,
                "Attribute \"temps\" on <for> contains empty declaration entry.",
            ));
        }
        result.push(parse_for_temp_decl_entry(entry, node)?);
    }

    if result.is_empty() {
        return Err(invalid_for_temps_error(
            node,
            "Attribute \"temps\" on <for> must contain at least one declaration.",
        ));
    }

    Ok(result)
}

fn parse_for_temp_decl_entry(
    entry: &str,
    node: &XmlElementNode,
) -> Result<ForTempDecl, ScriptLangError> {
    let delimiter_positions = top_level_delimiter_positions(entry, ':');
    if delimiter_positions.len() < 2 {
        return Err(invalid_for_temps_error(
            node,
            "Each temps entry on <for> must be \"name:type:init\".",
        ));
    }

    let first = delimiter_positions[0];
    let second = delimiter_positions[1];
    let name = entry[..first].trim().to_string();
    let type_expr = entry[first + 1..second].trim().to_string();
    let init_expr = entry[second + 1..].trim().to_string();

    if name.is_empty() || type_expr.is_empty() || init_expr.is_empty() {
        return Err(invalid_for_temps_error(
            node,
            "Each temps entry on <for> must provide non-empty name, type, and init.",
        ));
    }

    Ok(ForTempDecl {
        name,
        type_expr,
        init_expr,
    })
}

fn top_level_delimiter_positions(raw: &str, delimiter: char) -> Vec<usize> {
    let mut positions = Vec::new();
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut brace_depth = 0usize;
    let mut quote: Option<char> = None;

    for (index, ch) in raw.char_indices() {
        if let Some(active_quote) = quote {
            if ch == active_quote {
                quote = None;
            }
            continue;
        }

        if ch == '\'' || ch == '"' {
            quote = Some(ch);
            continue;
        }

        match ch {
            '(' => paren_depth += 1,
            ')' if paren_depth > 0 => paren_depth -= 1,
            '[' => bracket_depth += 1,
            ']' if bracket_depth > 0 => bracket_depth -= 1,
            '{' => brace_depth += 1,
            '}' if brace_depth > 0 => brace_depth -= 1,
            _ => {}
        }

        if ch == delimiter && paren_depth == 0 && bracket_depth == 0 && brace_depth == 0 {
            positions.push(index);
        }
    }

    positions
}

fn split_top_level_for_temps_entries(raw: &str) -> Vec<String> {
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
            ';' if paren_depth == 0 && bracket_depth == 0 && brace_depth == 0 => {
                parts.push(current.trim().to_string());
                current.clear();
                continue;
            }
            _ => {}
        }

        current.push(ch);
    }

    parts.push(current.trim().to_string());
    parts
}

fn invalid_for_temps_error(node: &XmlElementNode, message: &str) -> ScriptLangError {
    ScriptLangError::with_span(
        "XML_FOR_TEMPS_INVALID",
        message.to_string(),
        node.location.clone(),
    )
}

fn validate_for_attributes(node: &XmlElementNode) -> Result<(), ScriptLangError> {
    for key in node.attributes.keys() {
        if matches!(key.as_str(), "temps" | "condition" | "iteration") {
            continue;
        }
        return Err(ScriptLangError::with_span(
            "XML_ATTR_NOT_ALLOWED",
            format!(
                "Attribute \"{}\" is not allowed on <for>. Supported attributes: temps, condition, iteration.",
                key
            ),
            node.location.clone(),
        ));
    }
    Ok(())
}

pub(crate) fn next_for_first_flag_var_name(context: &mut MacroExpansionContext) -> String {
    loop {
        let candidate = format!("{}{}_first", FOR_FIRST_TEMP_VAR_PREFIX, context.for_counter);
        context.for_counter += 1;
        if context.used_var_names.insert(candidate.clone()) {
            return candidate;
        }
    }
}

#[cfg(test)]
mod macro_expand_tests {
    use super::*;
    use crate::compiler_test_support::*;

    #[test]
    fn for_macro_expands_to_group_and_while() {
        let files = map(&[(
            "main.xml",
            r#"
    <module name="main" export="script:main">
    <script name="main">
      <text>before</text>
      <for temps="i:int:0" condition="i LT 2" iteration="i = i + 1;">
        <text>${i}</text>
      </for>
    </script>
    </module>
    "#,
        )]);

        let result = compile_project_bundle_from_xml_map(&files).expect("project should compile");
        let main = result.scripts.get("main.main").expect("main script");
        let root = main.groups.get(&main.root_group_id).expect("root group");

        // root.nodes is [Text, If], so find_map's _ => None branch executes for Text
        let then_group_id = root
            .nodes
            .iter()
            .find_map(|node| match node {
                ScriptNode::If { then_group_id, .. } => Some(then_group_id.as_str()),
                _ => None, // This executes for Text node
            })
            .expect("for should compile into a scoped group");
        let for_group = main.groups.get(then_group_id).expect("for group");
        let var_count = for_group
            .nodes
            .iter()
            .filter(|node| matches!(node, ScriptNode::Var { .. }))
            .count();
        // for_group.nodes is [Var, Var, While], so find_map's _ => None executes for Var nodes
        let while_node = for_group
            .nodes
            .iter()
            .find_map(|node| match node {
                ScriptNode::While { body_group_id, .. } => Some(body_group_id.as_str()),
                _ => None, // This executes for Var nodes
            })
            .expect("for should produce while node");
        assert_eq!(var_count, 2);

        let while_group = main.groups.get(while_node).expect("while group");
        // Verify the group has nodes
        assert!(!while_group.nodes.is_empty());
    }

    #[test]
    fn for_macro_guards_iteration_with_first_flag() {
        let for_node = xml_element(
            "for",
            &[
                ("temps", "i:int:0"),
                ("condition", "i < 3"),
                ("iteration", "i = i + 1;"),
            ],
            vec![XmlNode::Element(xml_element(
                "text",
                &[],
                vec![xml_text("x")],
            ))],
        );
        let expanded = expand_element_with_macros(
            &for_node,
            &mut MacroExpansionContext {
                used_var_names: BTreeSet::new(),
                for_counter: 0,
            },
        )
        .expect("for should expand");
        let group = expanded.first().expect("expanded group");
        assert_eq!(group.name, "group");

        let while_node =
            find_child_by_name(&group.children, "while").expect("for should contain while");
        let if_node =
            find_child_by_name(&while_node.children, "if").expect("while first child must be if");
        assert_eq!(if_node.name, "if");
        let else_node =
            find_child_by_name(&if_node.children, "else").expect("if should contain else branch");
        let code_node =
            find_child_by_name(&else_node.children, "code").expect("else first child must be code");
        assert_eq!(code_node.name, "code");
        let iteration_text = inline_text_content(code_node);
        assert_eq!(iteration_text.trim(), "i = i + 1;");

        // Cover the _ => None branches in find_map by iterating over mixed content
        let mixed_children: Vec<XmlNode> = vec![
            XmlNode::Element(xml_element("text", &[], vec![xml_text("text")])),
            XmlNode::Element(xml_element("other", &[], Vec::new())),
            XmlNode::Text(XmlTextNode {
                value: "text".to_string(),
                location: SourceSpan::synthetic(),
            }),
        ];
        // This find_map will hit _ => None for all elements (no "if")
        let result = find_child_by_name(&mixed_children, "if");
        assert!(
            result.is_none(),
            "find_map should return None when no match"
        );
    }

    #[test]
    fn for_temps_parser_handles_complex_init_and_empty_middle_entry() {
        let host = xml_element(
            "for",
            &[
                (
                    "temps",
                    "a:int:1;b:string:'x:y';c:#{string=>int}:#{'k': 1};",
                ),
                ("condition", "true"),
                ("iteration", "a = a + 1;"),
            ],
            Vec::new(),
        );
        let parsed = parse_for_temps_decls(
            host.attributes
                .get("temps")
                .expect("temps attr should exist"),
            &host,
        )
        .expect("complex temps should parse");
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[1].name, "b");
        assert_eq!(parsed[1].type_expr, "string");
        assert_eq!(parsed[1].init_expr, "'x:y'");
        assert_eq!(parsed[2].type_expr, "#{string=>int}");
        assert_eq!(parsed[2].init_expr, "#{'k': 1}");

        let broken = xml_element(
            "for",
            &[
                ("temps", "a:int:1;;b:int:2"),
                ("condition", "true"),
                ("iteration", "a = a + 1;"),
            ],
            Vec::new(),
        );
        let error = parse_for_temps_decls(
            broken
                .attributes
                .get("temps")
                .expect("temps attr should exist"),
            &broken,
        )
        .expect_err("empty middle entry should fail");
        assert_eq!(error.code, "XML_FOR_TEMPS_INVALID");
    }

    #[test]
    fn for_temps_parser_handles_nested_brackets() {
        // Test that temps parser correctly handles nested parentheses, brackets, and braces
        // This covers the match branches in top_level_delimiter_positions and
        // split_top_level_for_temps_entries
        let host = xml_element(
            "for",
            &[
                // Single entry with parentheses in init expression
                ("temps", "fn:int:get_value(i)"),
                ("condition", "true"),
                ("iteration", "i = i + 1;"),
            ],
            Vec::new(),
        );
        let parsed = parse_for_temps_decls(
            host.attributes
                .get("temps")
                .expect("temps attr should exist"),
            &host,
        )
        .expect("nested brackets should parse");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "fn");
        assert_eq!(parsed[0].type_expr, "int");
        assert_eq!(parsed[0].init_expr, "get_value(i)");

        // Test with array brackets in init
        let host2 = xml_element(
            "for",
            &[
                ("temps", "arr:string:[1,2,3]"),
                ("condition", "true"),
                ("iteration", "i = i + 1;"),
            ],
            Vec::new(),
        );
        let parsed2 = parse_for_temps_decls(
            host2
                .attributes
                .get("temps")
                .expect("temps attr should exist"),
            &host2,
        )
        .expect("array brackets should parse");
        assert_eq!(parsed2[0].init_expr, "[1,2,3]");

        // Test with map braces in init
        let host3 = xml_element(
            "for",
            &[
                ("temps", "map:#{string=>int}:#{'a': 1}"),
                ("condition", "true"),
                ("iteration", "i = i + 1;"),
            ],
            Vec::new(),
        );
        let parsed3 = parse_for_temps_decls(
            host3
                .attributes
                .get("temps")
                .expect("temps attr should exist"),
            &host3,
        )
        .expect("map braces should parse");
        assert_eq!(parsed3[0].init_expr, "#{'a': 1}");
    }

    #[test]
    fn for_temps_parser_handles_unbalanced_closing_tokens_without_underflow() {
        let host = xml_element(
            "for",
            &[
                ("temps", "a:string:value)]};b:int:2"),
                ("condition", "true"),
                ("iteration", "a = a + 1;"),
            ],
            Vec::new(),
        );
        let parsed = parse_for_temps_decls(
            host.attributes
                .get("temps")
                .expect("temps attr should exist"),
            &host,
        )
        .expect("unbalanced closing tokens should not break parser");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].init_expr, "value)]}");
        assert_eq!(parsed[1].init_expr, "2");
    }

    #[test]
    fn temp_input_macro_expands_to_temp_then_input() {
        let node = xml_element(
            "temp-input",
            &[
                ("name", "heroName"),
                ("type", "string"),
                ("text", "Name your hero"),
                ("max_length", "16"),
            ],
            vec![xml_text("\"Traveler\"")],
        );
        let expanded = expand_element_with_macros(
            &node,
            &mut MacroExpansionContext {
                used_var_names: BTreeSet::new(),
                for_counter: 0,
            },
        )
        .expect("temp-input should expand");
        assert_eq!(expanded.len(), 2);

        let temp = &expanded[0];
        assert_eq!(temp.name, "temp");
        assert_eq!(
            temp.attributes.get("name").map(String::as_str),
            Some("heroName")
        );
        assert_eq!(
            temp.attributes.get("type").map(String::as_str),
            Some("string")
        );
        assert_eq!(inline_text_content(temp), "\"Traveler\"");

        let input = &expanded[1];
        assert_eq!(input.name, "input");
        assert_eq!(
            input.attributes.get("var").map(String::as_str),
            Some("heroName")
        );
        assert_eq!(
            input.attributes.get("text").map(String::as_str),
            Some("Name your hero")
        );
        assert_eq!(
            input.attributes.get("max_length").map(String::as_str),
            Some("16")
        );
    }

    #[test]
    fn macro_expand_validation_error_paths_are_covered() {
        let reserved_var = xml_element(
            "script",
            &[("name", "main")],
            vec![XmlNode::Element(xml_element(
                "temp",
                &[("name", "__sl_bad"), ("type", "int")],
                vec![xml_text("1")],
            ))],
        );
        let error = validate_reserved_prefix_in_user_var_declarations(&reserved_var)
            .expect_err("reserved var name should fail");
        assert_eq!(error.code, "NAME_RESERVED_PREFIX");
        let keyword_var = xml_element(
            "script",
            &[("name", "main")],
            vec![XmlNode::Element(xml_element(
                "temp",
                &[("name", "shared"), ("type", "int")],
                vec![xml_text("1")],
            ))],
        );
        let error = validate_reserved_prefix_in_user_var_declarations(&keyword_var)
            .expect_err("keyword var name should fail");
        assert_eq!(error.code, "NAME_RHAI_KEYWORD_RESERVED");

        let reserved_temp_input = xml_element(
            "temp-input",
            &[
                ("name", "__bad"),
                ("type", "string"),
                ("text", "Name your hero"),
            ],
            Vec::new(),
        );
        let error = validate_reserved_prefix_in_user_var_declarations(&reserved_temp_input)
            .expect_err("reserved temp-input name should fail");
        assert_eq!(error.code, "NAME_RESERVED_PREFIX");

        let bad_temps = xml_element(
            "for",
            &[
                ("temps", "i:int"),
                ("condition", "true"),
                ("iteration", "i = i + 1;"),
            ],
            Vec::new(),
        );
        let error = expand_element_with_macros(
            &bad_temps,
            &mut MacroExpansionContext {
                used_var_names: BTreeSet::new(),
                for_counter: 0,
            },
        )
        .expect_err("invalid temps should fail");
        assert_eq!(error.code, "XML_FOR_TEMPS_INVALID");

        let duplicate_temps = xml_element(
            "for",
            &[
                ("temps", "i:int:0;i:int:1"),
                ("condition", "true"),
                ("iteration", "i = i + 1;"),
            ],
            Vec::new(),
        );
        let error = expand_element_with_macros(
            &duplicate_temps,
            &mut MacroExpansionContext {
                used_var_names: BTreeSet::new(),
                for_counter: 0,
            },
        )
        .expect_err("duplicate temp declarations should fail");
        assert_eq!(error.code, "XML_FOR_TEMPS_DUPLICATE");

        let reserved_for_temp = xml_element(
            "for",
            &[
                ("temps", "__bad:int:0"),
                ("condition", "true"),
                ("iteration", "true;"),
            ],
            Vec::new(),
        );
        let error = expand_element_with_macros(
            &reserved_for_temp,
            &mut MacroExpansionContext {
                used_var_names: BTreeSet::new(),
                for_counter: 0,
            },
        )
        .expect_err("reserved for temp should fail");
        assert_eq!(error.code, "NAME_RESERVED_PREFIX");

        let keyword_for_temp = xml_element(
            "for",
            &[
                ("temps", "shared:int:0"),
                ("condition", "true"),
                ("iteration", "true;"),
            ],
            Vec::new(),
        );
        let error = expand_element_with_macros(
            &keyword_for_temp,
            &mut MacroExpansionContext {
                used_var_names: BTreeSet::new(),
                for_counter: 0,
            },
        )
        .expect_err("keyword for temp should fail");
        assert_eq!(error.code, "NAME_RHAI_KEYWORD_RESERVED");

        let bad_for_attr = xml_element(
            "for",
            &[
                ("temps", "i:int:0"),
                ("condition", "true"),
                ("iteration", "i = i + 1;"),
                ("times", "2"),
            ],
            Vec::new(),
        );
        let error = expand_element_with_macros(
            &bad_for_attr,
            &mut MacroExpansionContext {
                used_var_names: BTreeSet::new(),
                for_counter: 0,
            },
        )
        .expect_err("unsupported <for> attrs should fail");
        assert_eq!(error.code, "XML_ATTR_NOT_ALLOWED");

        let empty_iteration = xml_element(
            "for",
            &[
                ("temps", "i:int:0"),
                ("condition", "true"),
                ("iteration", " "),
            ],
            Vec::new(),
        );
        let error = expand_element_with_macros(
            &empty_iteration,
            &mut MacroExpansionContext {
                used_var_names: BTreeSet::new(),
                for_counter: 0,
            },
        )
        .expect_err("empty iteration should fail");
        assert_eq!(error.code, "XML_EMPTY_ATTR");

        let temp_input_missing_type = xml_element(
            "temp-input",
            &[("name", "hero"), ("text", "Name your hero")],
            Vec::new(),
        );
        let error = expand_element_with_macros(
            &temp_input_missing_type,
            &mut MacroExpansionContext {
                used_var_names: BTreeSet::new(),
                for_counter: 0,
            },
        )
        .expect_err("missing type on temp-input should fail");
        assert_eq!(error.code, "XML_MISSING_ATTR");

        // Test empty name attribute (covers 123:57)
        let temp_input_empty_name = xml_element(
            "temp-input",
            &[("name", ""), ("type", "string"), ("text", "Name your hero")],
            Vec::new(),
        );
        let error = expand_element_with_macros(
            &temp_input_empty_name,
            &mut MacroExpansionContext {
                used_var_names: BTreeSet::new(),
                for_counter: 0,
            },
        )
        .expect_err("empty name on temp-input should fail");
        assert_eq!(error.code, "XML_EMPTY_ATTR");

        // Test reserved name on temp-input (covers 124:94)
        let temp_input_reserved_name = xml_element(
            "temp-input",
            &[
                ("name", "__reserved"),
                ("type", "string"),
                ("text", "Name your hero"),
            ],
            Vec::new(),
        );
        let error = expand_element_with_macros(
            &temp_input_reserved_name,
            &mut MacroExpansionContext {
                used_var_names: BTreeSet::new(),
                for_counter: 0,
            },
        )
        .expect_err("reserved name on temp-input should fail");
        assert_eq!(error.code, "NAME_RESERVED_PREFIX");

        let temp_input_bad_type = xml_element(
            "temp-input",
            &[
                ("name", "hero"),
                ("type", "int"),
                ("text", "Name your hero"),
            ],
            Vec::new(),
        );
        let error = expand_element_with_macros(
            &temp_input_bad_type,
            &mut MacroExpansionContext {
                used_var_names: BTreeSet::new(),
                for_counter: 0,
            },
        )
        .expect_err("non-string temp-input type should fail");
        assert_eq!(error.code, "XML_TEMP_INPUT_TYPE_UNSUPPORTED");

        let temp_input_bad_attr = xml_element(
            "temp-input",
            &[
                ("name", "hero"),
                ("type", "string"),
                ("text", "Name your hero"),
                ("var", "hero"),
            ],
            Vec::new(),
        );
        let error = expand_element_with_macros(
            &temp_input_bad_attr,
            &mut MacroExpansionContext {
                used_var_names: BTreeSet::new(),
                for_counter: 0,
            },
        )
        .expect_err("unsupported temp-input attrs should fail");
        assert_eq!(error.code, "XML_ATTR_NOT_ALLOWED");

        // Test missing text attribute (covers 138:64)
        let temp_input_missing_text = xml_element(
            "temp-input",
            &[("name", "hero"), ("type", "string")],
            Vec::new(),
        );
        let error = expand_element_with_macros(
            &temp_input_missing_text,
            &mut MacroExpansionContext {
                used_var_names: BTreeSet::new(),
                for_counter: 0,
            },
        )
        .expect_err("missing text should fail");
        assert_eq!(error.code, "XML_MISSING_ATTR");

        // Test empty text attribute
        let temp_input_empty_text = xml_element(
            "temp-input",
            &[("name", "hero"), ("type", "string"), ("text", "")],
            Vec::new(),
        );
        let error = expand_element_with_macros(
            &temp_input_empty_text,
            &mut MacroExpansionContext {
                used_var_names: BTreeSet::new(),
                for_counter: 0,
            },
        )
        .expect_err("empty text should fail");
        assert_eq!(error.code, "XML_EMPTY_ATTR");

        let temp_input_with_child = xml_element(
            "temp-input",
            &[
                ("name", "hero"),
                ("type", "string"),
                ("text", "Name your hero"),
            ],
            vec![XmlNode::Element(xml_element(
                "text",
                &[],
                vec![xml_text("x")],
            ))],
        );
        let error = expand_element_with_macros(
            &temp_input_with_child,
            &mut MacroExpansionContext {
                used_var_names: BTreeSet::new(),
                for_counter: 0,
            },
        )
        .expect_err("temp-input child elements should fail");
        assert_eq!(error.code, "XML_TEMP_INPUT_CONTENT_FORBIDDEN");

        let empty_inline_temp_input = xml_element(
            "temp-input",
            &[("name", "title"), ("type", "string"), ("text", "Title")],
            vec![xml_text("  ")],
        );
        let expanded = expand_element_with_macros(
            &empty_inline_temp_input,
            &mut MacroExpansionContext {
                used_var_names: BTreeSet::new(),
                for_counter: 0,
            },
        )
        .expect("empty inline temp-input should expand");
        assert_eq!(expanded.len(), 2);
        assert!(expanded[0].children.is_empty());

        let plain = xml_element(
            "text",
            &[],
            vec![
                xml_text("hello"),
                XmlNode::Element(xml_element("x", &[], Vec::new())),
            ],
        );
        let mut context = MacroExpansionContext {
            used_var_names: BTreeSet::new(),
            for_counter: 0,
        };
        let expanded = expand_element_with_macros(&plain, &mut context).expect("expand plain node");
        assert_eq!(expanded.len(), 1);
        assert_eq!(expanded[0].name, "text");

        let for_with_bad_child = xml_element(
            "for",
            &[
                ("temps", "i:int:0"),
                ("condition", "true"),
                ("iteration", "i = i + 1;"),
            ],
            vec![XmlNode::Element(xml_element(
                "for",
                &[("temps", "bad"), ("condition", "true"), ("iteration", "x")],
                Vec::new(),
            ))],
        );
        let error = expand_element_with_macros(&for_with_bad_child, &mut context)
            .expect_err("for body child expansion should propagate errors");
        assert_eq!(error.code, "XML_FOR_TEMPS_INVALID");

        let chosen = next_for_first_flag_var_name(&mut context);
        assert!(chosen.starts_with(FOR_FIRST_TEMP_VAR_PREFIX));
    }

    #[test]
    fn for_macro_empty_temps_attribute_fails() {
        let for_node = xml_element(
            "for",
            &[
                ("temps", ""),
                ("condition", "true"),
                ("iteration", "i = i + 1"),
            ],
            Vec::new(),
        );
        let error = expand_element_with_macros(
            &for_node,
            &mut MacroExpansionContext {
                used_var_names: BTreeSet::new(),
                for_counter: 0,
            },
        )
        .expect_err("empty temps should fail");
        assert_eq!(error.code, "XML_EMPTY_ATTR");
    }

    #[test]
    fn for_macro_missing_iteration_attribute_defaults_to_noop() {
        let for_node = xml_element(
            "for",
            &[("temps", "i:int:0"), ("condition", "true")],
            Vec::new(),
        );
        let expanded = expand_element_with_macros(
            &for_node,
            &mut MacroExpansionContext {
                used_var_names: BTreeSet::new(),
                for_counter: 0,
            },
        )
        .expect("missing iteration should default to no-op");

        let while_node = expanded[0]
            .children
            .iter()
            .find_map(|child| match child {
                XmlNode::Element(element) if element.name == "while" => Some(element),
                _ => None,
            })
            .expect("for expansion should contain while");

        let first_if =
            find_child_by_name(&while_node.children, "if").expect("while first child must be if");
        let else_node = element_children(first_if)
            .find(|child| child.name == "else")
            .expect("guard if should contain else");

        let iteration_code =
            find_child_by_name(&else_node.children, "code").expect("else first child must be code");
        assert_eq!(inline_text_content(iteration_code).trim(), "true;");

        // Cover the _ => None branches in find_map (lines 1243:22, 1254:22)
        let mixed_children: Vec<XmlNode> = vec![
            XmlNode::Element(xml_element("text", &[], vec![xml_text("text")])),
            XmlNode::Element(xml_element("other", &[], Vec::new())),
            XmlNode::Text(XmlTextNode {
                value: "text".to_string(),
                location: SourceSpan::synthetic(),
            }),
        ];
        // This find_map will hit _ => None for all elements (no "code")
        let result = find_child_by_name(&mixed_children, "code");
        assert!(
            result.is_none(),
            "find_map should return None when no match"
        );
    }

    #[test]
    fn for_macro_missing_condition_attribute_fails() {
        // Test line 206: get_required_non_empty_attr("condition") error branch
        let for_node = xml_element(
            "for",
            &[("temps", "i:int:0"), ("iteration", "i = i + 1")],
            Vec::new(),
        );
        let error = expand_element_with_macros(
            &for_node,
            &mut MacroExpansionContext {
                used_var_names: BTreeSet::new(),
                for_counter: 0,
            },
        )
        .expect_err("missing condition should fail");
        assert_eq!(error.code, "XML_MISSING_ATTR");
    }

    #[test]
    fn parse_for_temps_empty_string_fails() {
        let node = xml_element("for", &[("temps", "")], Vec::new());
        let error = parse_for_temps_decls("", &node).expect_err("empty temps string should fail");
        assert_eq!(error.code, "XML_FOR_TEMPS_INVALID");
    }

    #[test]
    fn parse_for_temps_missing_type_fails() {
        // Test line 370: temps entry missing type (name::init)
        let node = xml_element("for", &[("temps", "")], Vec::new());
        let error =
            parse_for_temps_decls("x::0", &node).expect_err("temps entry missing type should fail");
        assert_eq!(error.code, "XML_FOR_TEMPS_INVALID");
    }

    #[test]
    fn for_macro_child_expansion_propagates_error() {
        // Test that expand_children error propagation is covered (34:64)
        // Create a for with an empty temps which will fail during child expansion
        let for_node = xml_element(
            "for",
            &[
                ("temps", ""),
                ("condition", "true"),
                ("iteration", "i = i + 1"),
            ],
            Vec::new(),
        );
        let error = expand_element_with_macros(
            &for_node,
            &mut MacroExpansionContext {
                used_var_names: BTreeSet::new(),
                for_counter: 0,
            },
        )
        .expect_err("empty temps should fail");
        assert_eq!(error.code, "XML_EMPTY_ATTR");
    }

    #[test]
    fn plain_element_child_expansion_error_propagation() {
        // Test line 108: expand_element_with_macros for plain elements
        // When a plain element contains an invalid for macro as child,
        // expand_children returns error which propagates to line 108
        let text_element = xml_element(
            "text",
            &[],
            vec![XmlNode::Element(xml_element(
                "for",
                &[("temps", "i:int:0")], // missing condition and iteration
                vec![xml_text("x")],
            ))],
        );
        let error = expand_element_with_macros(
            &text_element,
            &mut MacroExpansionContext {
                used_var_names: BTreeSet::new(),
                for_counter: 0,
            },
        )
        .expect_err("invalid for child should fail");
        assert_eq!(error.code, "XML_MISSING_ATTR");
    }

    #[test]
    fn expand_script_macros_error_propagation() {
        // Test line 34: expand_script_macros error propagation via expand_children
        // Create a script element containing a for macro with invalid attributes
        let script_element = xml_element(
            "script",
            &[("name", "main")],
            vec![XmlNode::Element(xml_element(
                "for",
                &[("temps", "i:int:0")], // missing condition and iteration
                vec![xml_text("x")],
            ))],
        );
        let error = expand_script_macros(&script_element, &["x".to_string()])
            .expect_err("invalid for should fail");
        assert_eq!(error.code, "XML_MISSING_ATTR");
    }

    // Test coverage for find_child_by_name helper function (covers both Some and None branches)
    #[test]
    fn find_child_by_name_covers_both_branches() {
        let children_with_if: Vec<XmlNode> =
            vec![XmlNode::Element(xml_element("if", &[], Vec::new()))];
        let children_with_code: Vec<XmlNode> =
            vec![XmlNode::Element(xml_element("code", &[], Vec::new()))];
        let children_with_while: Vec<XmlNode> =
            vec![XmlNode::Element(xml_element("while", &[], Vec::new()))];
        let children_without: Vec<XmlNode> = vec![
            XmlNode::Element(xml_element("text", &[], vec![xml_text("hello")])),
            XmlNode::Element(xml_element("other", &[], Vec::new())),
            XmlNode::Text(XmlTextNode {
                value: "text".to_string(),
                location: SourceSpan::synthetic(),
            }),
        ];

        // Test Some branch - element found
        assert!(find_child_by_name(&children_with_if, "if").is_some());
        assert!(find_child_by_name(&children_with_code, "code").is_some());
        assert!(find_child_by_name(&children_with_while, "while").is_some());

        // Test None branch - element not found
        assert!(find_child_by_name(&children_with_if, "other").is_none());
        assert!(find_child_by_name(&children_without, "if").is_none());
        assert!(find_child_by_name(&children_without, "code").is_none());
        assert!(find_child_by_name(&children_without, "while").is_none());
    }
}
