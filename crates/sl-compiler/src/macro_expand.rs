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

