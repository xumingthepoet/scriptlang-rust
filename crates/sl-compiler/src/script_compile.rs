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

    builder.groups.insert(
        group_id.to_string(),
        ImplicitGroup {
            group_id: group_id.to_string(),
            parent_group_id: parent_group_id.map(|value| value.to_string()),
            entry_node_id: None,
            nodes: Vec::new(),
        },
    );

    compile_group_nodes(
        group_id,
        container,
        builder,
        visible_types,
        &mut local_var_types,
        mode,
        &mut nodes,
    )?;

    let entry_node_id = nodes.first().map(|node| node_id(node).to_string());
    if let Some(group) = builder.groups.get_mut(group_id) {
        group.entry_node_id = entry_node_id;
        group.nodes = nodes;
    }

    Ok(())
}

fn compile_group_nodes(
    group_id: &str,
    container: &XmlElementNode,
    builder: &mut GroupBuilder,
    visible_types: &BTreeMap<String, ScriptType>,
    local_var_types: &mut BTreeMap<String, ScriptType>,
    mode: CompileGroupMode,
    nodes: &mut Vec<ScriptNode>,
) -> Result<(), ScriptLangError> {
    for child in element_children(container) {
        if has_attr(child, "once") && child.name != "text" {
            return Err(ScriptLangError::with_span(
                "XML_ATTR_NOT_ALLOWED",
                "Attribute \"once\" is only allowed on <text> and <option>.",
                child.location.clone(),
            ));
        }

        let node = match child.name.as_str() {
            "group" => {
                let body_group_id = builder.next_group_id();
                let else_group_id = builder.next_group_id();

                compile_group(
                    &body_group_id,
                    Some(group_id),
                    child,
                    builder,
                    visible_types,
                    local_var_types,
                    CompileGroupMode::new(mode.while_depth, false),
                )?;

                builder.groups.insert(
                    else_group_id.clone(),
                    ImplicitGroup {
                        group_id: else_group_id.clone(),
                        parent_group_id: Some(group_id.to_string()),
                        entry_node_id: None,
                        nodes: Vec::new(),
                    },
                );

                ScriptNode::If {
                    id: builder.next_node_id("if"),
                    when_expr: "true".to_string(),
                    then_group_id: body_group_id,
                    else_group_id: Some(else_group_id),
                    location: child.location.clone(),
                }
            }
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
                    local_var_types,
                    CompileGroupMode::new(mode.while_depth, false),
                )?;

                if let Some(else_child) = else_node {
                    compile_group(
                        &else_group_id,
                        Some(group_id),
                        else_child,
                        builder,
                        visible_types,
                        local_var_types,
                        CompileGroupMode::new(mode.while_depth, false),
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
                    local_var_types,
                    CompileGroupMode::new(mode.while_depth + 1, false),
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
                        local_var_types,
                        CompileGroupMode::new(mode.while_depth, true),
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

        nodes.push(node);
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

fn parse_type_name_segment<'a>(
    segment: &'a str,
    parse_error_code: &'static str,
    parse_error_label: &'static str,
    span: &SourceSpan,
) -> Result<(&'a str, &'a str), ScriptLangError> {
    let Some(separator) = segment.find(':') else {
        return Err(ScriptLangError::with_span(
            parse_error_code,
            format!("Invalid {} segment: \"{}\".", parse_error_label, segment),
            span.clone(),
        ));
    };
    if separator == 0 || separator + 1 >= segment.len() {
        return Err(ScriptLangError::with_span(
            parse_error_code,
            format!("Invalid {} segment: \"{}\".", parse_error_label, segment),
            span.clone(),
        ));
    }

    let type_raw = segment[..separator].trim();
    let name = segment[separator + 1..].trim();
    Ok((type_raw, name))
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
        let (type_raw, name) = parse_type_name_segment(
            normalized,
            "SCRIPT_ARGS_PARSE_ERROR",
            "script args",
            &root.location,
        )?;

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
        let (type_raw, name) = parse_type_name_segment(
            &segment,
            "FUNCTION_ARGS_PARSE_ERROR",
            "function args",
            &node.location,
        )?;
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
    let (type_raw, name) = parse_type_name_segment(
        &raw,
        "FUNCTION_RETURN_PARSE_ERROR",
        "function return",
        &node.location,
    )?;
    assert_name_not_reserved(name, "function return", node.location.clone())?;

    Ok(ParsedFunctionParamDecl {
        name: name.to_string(),
        type_expr: parse_type_expr(type_raw, &node.location)?,
        location: node.location.clone(),
    })
}

#[cfg(test)]
mod script_compile_tests {
    use super::*;
    use crate::compiler_test_support::*;

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
    fn compile_group_creates_scoped_child_group_node() {
        let mut builder = GroupBuilder::new("group.script.xml");
        let root_group = builder.next_group_id();
        let container = xml_element(
            "script",
            &[("name", "main")],
            vec![
                XmlNode::Element(xml_element(
                    "group",
                    &[],
                    vec![
                        XmlNode::Element(xml_element(
                            "var",
                            &[("name", "name"), ("type", "string")],
                            vec![xml_text("\"Rin\"")],
                        )),
                        XmlNode::Element(xml_element("text", &[], vec![xml_text("in-group")])),
                    ],
                )),
                XmlNode::Element(xml_element(
                    "input",
                    &[("var", "name"), ("text", "prompt")],
                    Vec::new(),
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
        .expect("group container should compile");

        let group = builder
            .groups
            .get(&root_group)
            .expect("root group should exist");
        assert_eq!(group.nodes.len(), 2);
        assert!(matches!(group.nodes[1], ScriptNode::Input { .. }));

        let ScriptNode::If { then_group_id, .. } = &group.nodes[0] else {
            panic!("group node should compile into an if wrapper");
        };
        let scoped_group = builder
            .groups
            .get(then_group_id)
            .expect("group child should exist");
        assert_eq!(scoped_group.nodes.len(), 2);
        assert!(matches!(scoped_group.nodes[0], ScriptNode::Var { .. }));
        assert!(matches!(scoped_group.nodes[1], ScriptNode::Text { .. }));
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
            vec![XmlNode::Element(xml_element(
                "group",
                &[],
                vec![XmlNode::Element(xml_element("unknown", &[], Vec::new()))],
            ))],
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
