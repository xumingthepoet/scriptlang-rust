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

