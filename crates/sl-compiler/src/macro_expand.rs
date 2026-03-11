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
        loop_counter: 0,
    };

    Ok(XmlElementNode {
        name: root.name.clone(),
        attributes: root.attributes.clone(),
        children: expand_children(&root.children, &mut context)?,
        location: root.location.clone(),
    })
}

pub(crate) fn collect_declared_var_names(node: &XmlElementNode, names: &mut BTreeSet<String>) {
    if node.name == "temp" {
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
    if node.name == "temp" {
        if let Some(name) = node.attributes.get("name") {
            if !name.is_empty() {
                assert_decl_name_not_reserved_or_rhai_keyword(name, "temp", node.location.clone())?;
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
        name: "temp".to_string(),
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

pub(crate) fn parse_loop_times_expr(node: &XmlElementNode) -> Result<String, ScriptLangError> {
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

pub(crate) fn next_loop_temp_var_name(context: &mut MacroExpansionContext) -> String {
    loop {
        let candidate = format!("{}{}_remaining", LOOP_TEMP_VAR_PREFIX, context.loop_counter);
        context.loop_counter += 1;
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
    fn loop_macro_expands_to_var_and_while() {
        let files = map(&[(
            "main.xml",
            r#"
    <module name="main" export="script:main">
    <script name="main">
      <temp name="i" type="int">0</temp>
      <loop times="2">
        <code>i = i + 1;</code>
      </loop>
    </script>
    </module>
    "#,
        )]);

        let result = compile_project_bundle_from_xml_map(&files).expect("project should compile");
        let main = result.scripts.get("main.main").expect("main script");
        let root = main.groups.get(&main.root_group_id).expect("root group");
        let var_count = root
            .nodes
            .iter()
            .filter(|node| matches!(node, ScriptNode::Var { .. }))
            .count();
        let while_count = root
            .nodes
            .iter()
            .filter(|node| matches!(node, ScriptNode::While { .. }))
            .count();
        assert!(var_count > 0);
        assert_eq!(while_count, 1);
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

        let bad_loop = xml_element(
            "loop",
            &[("times", "${n}")],
            vec![XmlNode::Element(xml_element(
                "text",
                &[],
                vec![xml_text("x")],
            ))],
        );
        let error = parse_loop_times_expr(&bad_loop).expect_err("template times should fail");
        assert_eq!(error.code, "XML_LOOP_TIMES_TEMPLATE_UNSUPPORTED");

        let missing_times = xml_element("loop", &[], vec![xml_text("x")]);
        let error = parse_loop_times_expr(&missing_times).expect_err("times is required");
        assert_eq!(error.code, "XML_MISSING_ATTR");

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
            loop_counter: 0,
        };
        let expanded = expand_element_with_macros(&plain, &mut context).expect("expand plain node");
        assert_eq!(expanded.len(), 1);
        assert_eq!(expanded[0].name, "text");

        let plain_with_bad_child = xml_element(
            "text",
            &[],
            vec![XmlNode::Element(xml_element(
                "loop",
                &[("times", "${n}")],
                vec![xml_text("x")],
            ))],
        );
        let error = expand_element_with_macros(&plain_with_bad_child, &mut context)
            .expect_err("non-loop node child expansion should propagate errors");
        assert_eq!(error.code, "XML_LOOP_TIMES_TEMPLATE_UNSUPPORTED");

        let loop_with_bad_child = xml_element(
            "loop",
            &[("times", "2")],
            vec![XmlNode::Element(xml_element(
                "loop",
                &[("times", "${n}")],
                vec![xml_text("x")],
            ))],
        );
        let error = expand_element_with_macros(&loop_with_bad_child, &mut context)
            .expect_err("loop body child expansion should propagate errors");
        assert_eq!(error.code, "XML_LOOP_TIMES_TEMPLATE_UNSUPPORTED");

        let chosen = next_loop_temp_var_name(&mut context);
        assert!(chosen.starts_with(LOOP_TEMP_VAR_PREFIX));
    }
}
