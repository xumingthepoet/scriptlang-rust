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
    let iteration_expr = get_required_non_empty_attr(node, "iteration")?;
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

pub(crate) fn parse_for_temps_decls(
    raw: &str,
    node: &XmlElementNode,
) -> Result<Vec<ForTempDecl>, ScriptLangError> {
    let entries = split_top_level_for_temps_entries(raw);
    if entries.is_empty() {
        return Err(invalid_for_temps_error(
            node,
            "Attribute \"temps\" on <for> must contain at least one declaration.",
        ));
    }

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
      <for temps="i:int:0" condition="i &lt; 2" iteration="i = i + 1;">
        <text>${i}</text>
      </for>
    </script>
    </module>
    "#,
        )]);

        let result = compile_project_bundle_from_xml_map(&files).expect("project should compile");
        let main = result.scripts.get("main.main").expect("main script");
        let root = main.groups.get(&main.root_group_id).expect("root group");

        let ScriptNode::If { then_group_id, .. } = root.nodes.first().expect("group node") else {
            panic!("for should compile into a scoped group");
        };
        let for_group = main.groups.get(then_group_id).expect("for group");
        let var_count = for_group
            .nodes
            .iter()
            .filter(|node| matches!(node, ScriptNode::Var { .. }))
            .count();
        let while_node = for_group
            .nodes
            .iter()
            .find_map(|node| match node {
                ScriptNode::While { body_group_id, .. } => Some(body_group_id.as_str()),
                _ => None,
            })
            .expect("for should produce while node");
        assert_eq!(var_count, 2);

        let while_group = main.groups.get(while_node).expect("while group");
        assert!(matches!(
            while_group.nodes.first(),
            Some(ScriptNode::If { .. })
        ));
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

        let while_node = group
            .children
            .iter()
            .find_map(|entry| match entry {
                XmlNode::Element(element) if element.name == "while" => Some(element),
                _ => None,
            })
            .expect("for should contain while");
        let first_child = while_node.children.first().expect("while first child");
        let XmlNode::Element(if_node) = first_child else {
            panic!("while first child must be if");
        };
        assert_eq!(if_node.name, "if");
        let else_node = if_node
            .children
            .iter()
            .find_map(|entry| match entry {
                XmlNode::Element(element) if element.name == "else" => Some(element),
                _ => None,
            })
            .expect("if should contain else branch");
        let iteration_code = else_node.children.first().expect("else first child");
        let XmlNode::Element(code_node) = iteration_code else {
            panic!("else first child must be code");
        };
        assert_eq!(code_node.name, "code");
        let iteration_text = inline_text_content(code_node);
        assert_eq!(iteration_text.trim(), "i = i + 1;");
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

        let missing_iteration = xml_element(
            "for",
            &[("temps", "i:int:0"), ("condition", "true")],
            Vec::new(),
        );
        let error = expand_element_with_macros(
            &missing_iteration,
            &mut MacroExpansionContext {
                used_var_names: BTreeSet::new(),
                for_counter: 0,
            },
        )
        .expect_err("missing required attr should fail");
        assert_eq!(error.code, "XML_MISSING_ATTR");

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
}
