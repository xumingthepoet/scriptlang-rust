use crate::*;

#[derive(Debug, Clone, Copy)]
pub(crate) struct CompileGroupMode {
    while_depth: usize,
    allow_option_direct_continue: bool,
}

impl CompileGroupMode {
    pub(crate) fn new(while_depth: usize, allow_option_direct_continue: bool) -> Self {
        Self {
            while_depth,
            allow_option_direct_continue,
        }
    }
}

pub(crate) fn compile_script(
    options: CompileScriptOptions<'_>,
) -> Result<ScriptIr, ScriptLangError> {
    let CompileScriptOptions {
        script_path,
        root,
        qualified_script_name,
        module_name,
        visible_types,
        visible_functions,
        visible_defs_globals,
    } = options;
    if root.name != "script" {
        return Err(ScriptLangError::with_span(
            "XML_ROOT_INVALID",
            "Script file root must be <script>.",
            root.location.clone(),
        ));
    }

    let local_script_name = get_required_non_empty_attr(root, "name")?;
    assert_name_not_reserved(&local_script_name, "script", root.location.clone())?;
    let script_name = qualified_script_name
        .unwrap_or(&local_script_name)
        .to_string();

    let params = parse_script_args(root, visible_types)?;
    validate_reserved_prefix_in_user_var_declarations(root)?;

    let mut reserved_names = params
        .iter()
        .map(|param| param.name.clone())
        .collect::<Vec<_>>();
    reserved_names.sort();

    let expanded_root = expand_script_macros(root, &reserved_names)?;

    let mut builder = GroupBuilder::new(format!("{}::{}", script_path, script_name));
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
        CompileGroupMode::new(0, false),
    )?;

    qualify_local_script_targets_for_module(&mut builder.groups, module_name);

    Ok(ScriptIr {
        script_path: script_path.to_string(),
        script_name,
        module_name: module_name.map(|value| value.to_string()),
        local_script_name: module_name.map(|_| local_script_name.clone()),
        params,
        root_group_id,
        groups: builder.groups,
        visible_json_globals: Vec::new(),
        visible_functions: visible_functions.clone(),
        visible_defs_globals: visible_defs_globals.clone(),
    })
}

fn qualify_local_script_targets(groups: &mut BTreeMap<String, ImplicitGroup>, module_name: &str) {
    for group in groups.values_mut() {
        for node in &mut group.nodes {
            match node {
                ScriptNode::Call { target_script, .. } => {
                    qualify_static_script_target(target_script, module_name);
                }
                ScriptNode::Return {
                    target_script: Some(target_script),
                    ..
                } => {
                    qualify_static_script_target(target_script, module_name);
                }
                _ => {}
            }
        }
    }
}

fn qualify_local_script_targets_for_module(
    groups: &mut BTreeMap<String, ImplicitGroup>,
    module_name: Option<&str>,
) {
    if let Some(module_name) = module_name {
        qualify_local_script_targets(groups, module_name);
    }
}

fn qualify_static_script_target(target_script: &mut String, module_name: &str) {
    if target_script.contains('.')
        || target_script.contains("${")
        || target_script.trim().is_empty()
    {
        return;
    }
    *target_script = format!("{}.{}", module_name, target_script);
}

pub(crate) fn compile_group(
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
    let group = builder.groups.get_mut(group_id).expect("group must exist");
    group.entry_node_id = entry_node_id;
    group.nodes = nodes;

    Ok(())
}

fn compile_child_group(
    parent_group_id: &str,
    child_group_id: &str,
    child_container: &XmlElementNode,
    builder: &mut GroupBuilder,
    visible_types: &BTreeMap<String, ScriptType>,
    local_var_types: &mut BTreeMap<String, ScriptType>,
    mode: CompileGroupMode,
) -> Result<(), ScriptLangError> {
    compile_group(
        child_group_id,
        Some(parent_group_id),
        child_container,
        builder,
        visible_types,
        local_var_types,
        mode,
    )
}

pub(crate) fn compile_group_nodes(
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
                tag: get_optional_attr(child, "tag")
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty()),
                once: parse_bool_attr(child, "once", false)?,
                location: child.location.clone(),
            },
            "debug" => {
                if !child.attributes.is_empty() {
                    return Err(ScriptLangError::with_span(
                        "XML_ATTR_NOT_ALLOWED",
                        "<debug> does not support attributes. Use inline content only.",
                        child.location.clone(),
                    ));
                }
                ScriptNode::Debug {
                    id: builder.next_node_id("debug"),
                    value: parse_inline_required(child)?,
                    location: child.location.clone(),
                }
            }
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

                let group_mode = CompileGroupMode::new(mode.while_depth, false);
                let then_result = compile_child_group(
                    group_id,
                    &then_group_id,
                    &then_container,
                    builder,
                    visible_types,
                    local_var_types,
                    group_mode,
                );
                then_result?;

                if let Some(else_child) = else_node {
                    let else_result = compile_child_group(
                        group_id,
                        &else_group_id,
                        else_child,
                        builder,
                        visible_types,
                        local_var_types,
                        group_mode,
                    );
                    else_result?;
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
                let while_mode = CompileGroupMode::new(mode.while_depth + 1, false);
                let while_result = compile_child_group(
                    group_id,
                    &body_group_id,
                    child,
                    builder,
                    visible_types,
                    local_var_types,
                    while_mode,
                );
                while_result?;
                ScriptNode::While {
                    id: builder.next_node_id("while"),
                    when_expr: get_required_non_empty_attr(child, "when")?,
                    body_group_id,
                    location: child.location.clone(),
                }
            }
            "choice" => {
                let prompt_text = get_required_non_empty_attr(child, "text")?;
                let mut entries = Vec::new();
                let mut fall_over_seen = 0usize;
                let mut fall_over_entry_index = None;

                for choice_child in element_children(child) {
                    match choice_child.name.as_str() {
                        "option" => {
                            let once = parse_bool_attr(choice_child, "once", false)?;
                            let fall_over = parse_bool_attr(choice_child, "fall_over", false)?;
                            let when_expr = get_optional_attr(choice_child, "when");
                            if fall_over {
                                fall_over_seen += 1;
                                fall_over_entry_index = Some(entries.len());
                                if when_expr.is_some() {
                                    return Err(ScriptLangError::with_span(
                                        "XML_OPTION_FALL_OVER_WHEN_FORBIDDEN",
                                        "fall_over option cannot declare when.",
                                        choice_child.location.clone(),
                                    ));
                                }
                            }

                            let option_group_id = builder.next_group_id();
                            let option_mode = CompileGroupMode::new(mode.while_depth, true);
                            let option_result = compile_child_group(
                                group_id,
                                &option_group_id,
                                choice_child,
                                builder,
                                visible_types,
                                local_var_types,
                                option_mode,
                            );
                            option_result?;

                            entries.push(ChoiceEntry::Static {
                                option: ChoiceOption {
                                    id: builder.next_choice_id(),
                                    text: get_required_non_empty_attr(choice_child, "text")?,
                                    when_expr,
                                    once,
                                    fall_over,
                                    group_id: option_group_id,
                                    location: choice_child.location.clone(),
                                },
                            });
                        }
                        "dynamic-options" => {
                            let array_expr = get_required_non_empty_attr(choice_child, "array")?;
                            let item_name = get_required_non_empty_attr(choice_child, "item")?;
                            let index_name = get_optional_attr(choice_child, "index");
                            assert_name_not_reserved(
                                &item_name,
                                "dynamic-options item",
                                choice_child.location.clone(),
                            )?;
                            if let Some(index_name_value) = &index_name {
                                assert_name_not_reserved(
                                    index_name_value,
                                    "dynamic-options index",
                                    choice_child.location.clone(),
                                )?;
                            }
                            let templates = element_children(choice_child).collect::<Vec<_>>();
                            if templates.is_empty() {
                                return Err(ScriptLangError::with_span(
                                    "XML_DYNAMIC_OPTIONS_TEMPLATE_REQUIRED",
                                    "<dynamic-options> must contain exactly one <option> template child.",
                                    choice_child.location.clone(),
                                ));
                            }
                            if templates.len() != 1 || templates[0].name != "option" {
                                return Err(ScriptLangError::with_span(
                                    "XML_DYNAMIC_OPTIONS_CHILD_INVALID",
                                    "<dynamic-options> only supports exactly one direct <option> template child.",
                                    choice_child.location.clone(),
                                ));
                            }

                            let template_option = templates[0];
                            let has_once = parse_bool_attr(template_option, "once", false)?;
                            if has_once {
                                return Err(ScriptLangError::with_span(
                                    "XML_DYNAMIC_OPTION_ONCE_UNSUPPORTED",
                                    "<dynamic-options> template <option> does not support once.",
                                    template_option.location.clone(),
                                ));
                            }
                            let has_fall_over =
                                parse_bool_attr(template_option, "fall_over", false)?;
                            if has_fall_over {
                                return Err(ScriptLangError::with_span(
                                    "XML_DYNAMIC_OPTION_FALL_OVER_UNSUPPORTED",
                                    "<dynamic-options> template <option> does not support fall_over.",
                                    template_option.location.clone(),
                                ));
                            }

                            let option_group_id = builder.next_group_id();
                            let option_mode = CompileGroupMode::new(mode.while_depth, true);
                            let template_result = compile_child_group(
                                group_id,
                                &option_group_id,
                                template_option,
                                builder,
                                visible_types,
                                local_var_types,
                                option_mode,
                            );
                            template_result?;

                            entries.push(ChoiceEntry::Dynamic {
                                block: DynamicChoiceBlock {
                                    id: builder.next_choice_id(),
                                    array_expr,
                                    item_name,
                                    index_name,
                                    template: DynamicChoiceTemplate {
                                        text: get_required_non_empty_attr(template_option, "text")?,
                                        when_expr: get_optional_attr(template_option, "when"),
                                        group_id: option_group_id,
                                        location: template_option.location.clone(),
                                    },
                                    location: choice_child.location.clone(),
                                },
                            });
                        }
                        _ => {
                            return Err(ScriptLangError::with_span(
                                "XML_CHOICE_CHILD_INVALID",
                                format!(
                                    "Unsupported child <{}> under <choice>.",
                                    choice_child.name
                                ),
                                choice_child.location.clone(),
                            ));
                        }
                    }
                }

                if fall_over_seen > 1 {
                    return Err(ScriptLangError::with_span(
                        "XML_OPTION_FALL_OVER_DUPLICATE",
                        "At most one fall_over option is allowed per choice.",
                        child.location.clone(),
                    ));
                }

                if let Some(index) = fall_over_entry_index {
                    if index != entries.len().saturating_sub(1) {
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
                    entries,
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

pub(crate) fn node_id(node: &ScriptNode) -> &str {
    match node {
        ScriptNode::Text { id, .. }
        | ScriptNode::Debug { id, .. }
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

pub(crate) fn parse_var_declaration(
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

pub(crate) fn parse_type_name_segment<'a>(
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

pub(crate) fn parse_script_args(
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

pub(crate) fn parse_function_args(
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

pub(crate) fn parse_function_return(
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

    fn script_type_kind(ty: &ScriptType) -> &'static str {
        match ty {
            ScriptType::Primitive { .. } => "primitive",
            ScriptType::Array { .. } => "array",
            ScriptType::Map { .. } => "map",
            ScriptType::Object { .. } => "object",
        }
    }

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

        let root_bad_type = xml_element("script", &[("args", "#{ }:a")], Vec::new());
        let error =
            parse_script_args(&root_bad_type, &visible_types).expect_err("invalid arg type expr");
        assert_eq!(error.code, "TYPE_PARSE_ERROR");
        let root_unknown_type = xml_element("script", &[("args", "Missing:a")], Vec::new());
        let error =
            parse_script_args(&root_unknown_type, &visible_types).expect_err("unknown arg type");
        assert_eq!(error.code, "TYPE_UNKNOWN");

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

        let fn_reserved_arg = xml_element(
            "function",
            &[("name", "f"), ("args", "int:__sl_a"), ("return", "int:r")],
            vec![xml_text("r = 1;")],
        );
        let error =
            parse_function_declaration_node(&fn_reserved_arg).expect_err("reserved arg name");
        assert_eq!(error.code, "NAME_RESERVED_PREFIX");

        let fn_bad_arg_type = xml_element(
            "function",
            &[("name", "f"), ("args", "#{ }:a"), ("return", "int:r")],
            vec![xml_text("r = 1;")],
        );
        let error =
            parse_function_declaration_node(&fn_bad_arg_type).expect_err("bad arg type syntax");
        assert_eq!(error.code, "TYPE_PARSE_ERROR");

        let fn_missing_return = xml_element(
            "function",
            &[("name", "f"), ("args", "int:a")],
            vec![xml_text("a = a + 1;")],
        );
        let error =
            parse_function_declaration_node(&fn_missing_return).expect_err("missing return attr");
        assert_eq!(error.code, "XML_MISSING_ATTR");
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
        let _ = parse_type_expr("int[]", &span).expect("array should parse");
        let _ = parse_type_expr("#{int}", &span).expect("map should parse");

        let reserved_return = xml_element(
            "function",
            &[("name", "f"), ("return", "int:__sl_out")],
            vec![xml_text("__sl_out = 1;")],
        );
        let error = parse_function_return(&reserved_return).expect_err("reserved return binding");
        assert_eq!(error.code, "NAME_RESERVED_PREFIX");

        let invalid_return = xml_element(
            "function",
            &[("name", "f"), ("return", "#{ }:out")],
            vec![xml_text("out = 1;")],
        );
        let error = parse_function_return(&invalid_return).expect_err("invalid return type");
        assert_eq!(error.code, "TYPE_PARSE_ERROR");
    }

    #[test]
    fn compile_group_recurses_for_if_while_and_choice_children() {
        let mut builder = GroupBuilder::new("recursive.xml");
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
                    vec![
                        XmlNode::Element(xml_element(
                            "option",
                            &[("text", "O")],
                            vec![XmlNode::Element(xml_element(
                                "text",
                                &[],
                                vec![xml_text("X")],
                            ))],
                        )),
                        XmlNode::Element(xml_element(
                            "dynamic-options",
                            &[("array", "arr"), ("item", "it"), ("index", "i")],
                            vec![XmlNode::Element(xml_element(
                                "option",
                                &[("text", "D")],
                                vec![XmlNode::Element(xml_element(
                                    "text",
                                    &[],
                                    vec![xml_text("DX")],
                                ))],
                            ))],
                        )),
                    ],
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
    fn compile_group_supports_debug_and_rejects_debug_attributes() {
        let mut builder = GroupBuilder::new("debug.xml");
        let root_group = builder.next_group_id();
        let container = xml_element(
            "script",
            &[("name", "main")],
            vec![XmlNode::Element(xml_element(
                "debug",
                &[],
                vec![xml_text("hp=${hp}")],
            ))],
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
        .expect("debug should compile");
        let group = builder.groups.get(&root_group).expect("group");
        assert_eq!(group.nodes.len(), 1);

        for attrs in [[("text", "x")], [("once", "true")], [("tag", "x")]] {
            let mut bad_builder = GroupBuilder::new("debug-attr.xml");
            let bad_root = bad_builder.next_group_id();
            let bad_container = xml_element(
                "script",
                &[("name", "main")],
                vec![XmlNode::Element(xml_element(
                    "debug",
                    &attrs,
                    vec![xml_text("hp=${hp}")],
                ))],
            );
            let error = compile_group(
                &bad_root,
                None,
                &bad_container,
                &mut bad_builder,
                &BTreeMap::new(),
                &BTreeMap::new(),
                CompileGroupMode::new(0, false),
            )
            .expect_err("debug attrs should fail");
            assert_eq!(error.code, "XML_ATTR_NOT_ALLOWED");
        }

        let mut empty_builder = GroupBuilder::new("debug-empty.xml");
        let empty_root = empty_builder.next_group_id();
        let empty_container = xml_element(
            "script",
            &[("name", "main")],
            vec![XmlNode::Element(xml_element("debug", &[], vec![]))],
        );
        let error = compile_group(
            &empty_root,
            None,
            &empty_container,
            &mut empty_builder,
            &BTreeMap::new(),
            &BTreeMap::new(),
            CompileGroupMode::new(0, false),
        )
        .expect_err("empty debug body should fail");
        assert_eq!(error.code, "XML_EMPTY_NODE_CONTENT");
    }

    #[test]
    fn compile_group_creates_scoped_child_group_node() {
        let mut builder = GroupBuilder::new("group.xml");
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
        let extract_then_group_id = |node: &ScriptNode| match node {
            ScriptNode::If {
                then_group_id: child_group_id,
                ..
            } => Some(child_group_id.clone()),
            _ => None,
        };
        let then_group_id = extract_then_group_id(&group.nodes[0])
            .expect("group node should compile into an if wrapper");
        assert!(extract_then_group_id(&group.nodes[1]).is_none());
        let scoped_group = builder
            .groups
            .get(&then_group_id)
            .expect("group child should exist");
        assert_eq!(scoped_group.nodes.len(), 2);
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
        let mut builder = GroupBuilder::new("main.xml");
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
        let mut builder = GroupBuilder::new("main.xml");
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
        let mut builder = GroupBuilder::new("main.xml");
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
        let mut builder = GroupBuilder::new("main.xml");
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
        let mut builder = GroupBuilder::new("main.xml");
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

        let bad_text_inline = xml_element(
            "script",
            &[("name", "main")],
            vec![XmlNode::Element(xml_element("text", &[], Vec::new()))],
        );
        let mut builder = GroupBuilder::new("main.xml");
        let group_id = builder.next_group_id();
        let error = compile_group(
            &group_id,
            None,
            &bad_text_inline,
            &mut builder,
            &visible_types,
            &local_var_types,
            CompileGroupMode::new(0, false),
        )
        .expect_err("text inline content should be required");
        assert_eq!(error.code, "XML_EMPTY_NODE_CONTENT");

        let bad_code_inline = xml_element(
            "script",
            &[("name", "main")],
            vec![XmlNode::Element(xml_element("code", &[], Vec::new()))],
        );
        let mut builder = GroupBuilder::new("main.xml");
        let group_id = builder.next_group_id();
        let error = compile_group(
            &group_id,
            None,
            &bad_code_inline,
            &mut builder,
            &visible_types,
            &local_var_types,
            CompileGroupMode::new(0, false),
        )
        .expect_err("code inline content should be required");
        assert_eq!(error.code, "XML_EMPTY_NODE_CONTENT");

        let bad_if_then = xml_element(
            "script",
            &[("name", "main")],
            vec![XmlNode::Element(xml_element(
                "if",
                &[("when", "true")],
                vec![XmlNode::Element(xml_element("loop", &[], Vec::new()))],
            ))],
        );
        let mut builder = GroupBuilder::new("main.xml");
        let group_id = builder.next_group_id();
        let error = compile_group(
            &group_id,
            None,
            &bad_if_then,
            &mut builder,
            &visible_types,
            &local_var_types,
            CompileGroupMode::new(0, false),
        )
        .expect_err("if then group child compile errors should propagate");
        assert_eq!(error.code, "XML_LOOP_INTERNAL");

        let bad_if_else = xml_element(
            "script",
            &[("name", "main")],
            vec![XmlNode::Element(xml_element(
                "if",
                &[("when", "true")],
                vec![XmlNode::Element(xml_element(
                    "else",
                    &[],
                    vec![XmlNode::Element(xml_element("loop", &[], Vec::new()))],
                ))],
            ))],
        );
        let mut builder = GroupBuilder::new("main.xml");
        let group_id = builder.next_group_id();
        let error = compile_group(
            &group_id,
            None,
            &bad_if_else,
            &mut builder,
            &visible_types,
            &local_var_types,
            CompileGroupMode::new(0, false),
        )
        .expect_err("if else group child compile errors should propagate");
        assert_eq!(error.code, "XML_LOOP_INTERNAL");

        let bad_while_body = xml_element(
            "script",
            &[("name", "main")],
            vec![XmlNode::Element(xml_element(
                "while",
                &[("when", "true")],
                vec![XmlNode::Element(xml_element("loop", &[], Vec::new()))],
            ))],
        );
        let mut builder = GroupBuilder::new("main.xml");
        let group_id = builder.next_group_id();
        let error = compile_group(
            &group_id,
            None,
            &bad_while_body,
            &mut builder,
            &visible_types,
            &local_var_types,
            CompileGroupMode::new(0, false),
        )
        .expect_err("while body compile errors should propagate");
        assert_eq!(error.code, "XML_LOOP_INTERNAL");

        let bad_choice_text = xml_element(
            "script",
            &[("name", "main")],
            vec![XmlNode::Element(xml_element(
                "choice",
                &[],
                vec![XmlNode::Element(xml_element(
                    "option",
                    &[("text", "a")],
                    Vec::new(),
                ))],
            ))],
        );
        let mut builder = GroupBuilder::new("main.xml");
        let group_id = builder.next_group_id();
        let error = compile_group(
            &group_id,
            None,
            &bad_choice_text,
            &mut builder,
            &visible_types,
            &local_var_types,
            CompileGroupMode::new(0, false),
        )
        .expect_err("choice text should be required");
        assert_eq!(error.code, "XML_MISSING_ATTR");

        let bad_option_fall_over_bool = xml_element(
            "script",
            &[("name", "main")],
            vec![XmlNode::Element(xml_element(
                "choice",
                &[("text", "c")],
                vec![XmlNode::Element(xml_element(
                    "option",
                    &[("text", "a"), ("fall_over", "bad")],
                    Vec::new(),
                ))],
            ))],
        );
        let mut builder = GroupBuilder::new("main.xml");
        let group_id = builder.next_group_id();
        let error = compile_group(
            &group_id,
            None,
            &bad_option_fall_over_bool,
            &mut builder,
            &visible_types,
            &local_var_types,
            CompileGroupMode::new(0, false),
        )
        .expect_err("option fall_over bool should be validated");
        assert_eq!(error.code, "XML_ATTR_BOOL_INVALID");

        let bad_dynamic_template_bool = xml_element(
            "script",
            &[("name", "main")],
            vec![XmlNode::Element(xml_element(
                "choice",
                &[("text", "c")],
                vec![XmlNode::Element(xml_element(
                    "dynamic-options",
                    &[("array", "arr"), ("item", "it")],
                    vec![XmlNode::Element(xml_element(
                        "option",
                        &[("text", "t"), ("once", "bad")],
                        Vec::new(),
                    ))],
                ))],
            ))],
        );
        let mut builder = GroupBuilder::new("main.xml");
        let group_id = builder.next_group_id();
        let error = compile_group(
            &group_id,
            None,
            &bad_dynamic_template_bool,
            &mut builder,
            &visible_types,
            &local_var_types,
            CompileGroupMode::new(0, false),
        )
        .expect_err("dynamic template bool should be validated");
        assert_eq!(error.code, "XML_ATTR_BOOL_INVALID");

        let bad_choice_option_body = xml_element(
            "script",
            &[("name", "main")],
            vec![XmlNode::Element(xml_element(
                "choice",
                &[("text", "c")],
                vec![XmlNode::Element(xml_element(
                    "option",
                    &[("text", "a")],
                    vec![XmlNode::Element(xml_element("loop", &[], Vec::new()))],
                ))],
            ))],
        );
        let mut builder = GroupBuilder::new("main.xml");
        let group_id = builder.next_group_id();
        let error = compile_group(
            &group_id,
            None,
            &bad_choice_option_body,
            &mut builder,
            &visible_types,
            &local_var_types,
            CompileGroupMode::new(0, false),
        )
        .expect_err("choice option body compile errors should propagate");
        assert_eq!(error.code, "XML_LOOP_INTERNAL");

        let bad_dynamic_fall_over_bool = xml_element(
            "script",
            &[("name", "main")],
            vec![XmlNode::Element(xml_element(
                "choice",
                &[("text", "c")],
                vec![XmlNode::Element(xml_element(
                    "dynamic-options",
                    &[("array", "arr"), ("item", "it")],
                    vec![XmlNode::Element(xml_element(
                        "option",
                        &[("text", "t"), ("fall_over", "bad")],
                        Vec::new(),
                    ))],
                ))],
            ))],
        );
        let mut builder = GroupBuilder::new("main.xml");
        let group_id = builder.next_group_id();
        let error = compile_group(
            &group_id,
            None,
            &bad_dynamic_fall_over_bool,
            &mut builder,
            &visible_types,
            &local_var_types,
            CompileGroupMode::new(0, false),
        )
        .expect_err("dynamic template fall_over bool should be validated");
        assert_eq!(error.code, "XML_ATTR_BOOL_INVALID");

        let bad_dynamic_template_body = xml_element(
            "script",
            &[("name", "main")],
            vec![XmlNode::Element(xml_element(
                "choice",
                &[("text", "c")],
                vec![XmlNode::Element(xml_element(
                    "dynamic-options",
                    &[("array", "arr"), ("item", "it")],
                    vec![XmlNode::Element(xml_element(
                        "option",
                        &[("text", "t")],
                        vec![XmlNode::Element(xml_element("loop", &[], Vec::new()))],
                    ))],
                ))],
            ))],
        );
        let mut builder = GroupBuilder::new("main.xml");
        let group_id = builder.next_group_id();
        let error = compile_group(
            &group_id,
            None,
            &bad_dynamic_template_body,
            &mut builder,
            &visible_types,
            &local_var_types,
            CompileGroupMode::new(0, false),
        )
        .expect_err("dynamic option template body errors should propagate");
        assert_eq!(error.code, "XML_LOOP_INTERNAL");
    }

    #[test]
    fn compiler_error_matrix_covers_more_validation_paths() {
        let cases: Vec<(&str, BTreeMap<String, String>, &str)> = vec![
                (
                    "defs child invalid",
                    map(&[
                        (
                            "x.xml",
                            "<defs name=\"x\"><unknown/></defs>",
                        ),
                        (
                            "main.xml",
                            r#"
    <!-- include: x.xml -->
    <module name="main">
<script name="main"><text>x</text></script>
</module>
    "#,
                        ),
                    ]),
                    "XML_MODULE_CHILD_INVALID",
                ),
                (
                    "type field child invalid",
                    map(&[
                        (
                            "x.xml",
                            "<defs name=\"x\"><type name=\"A\"><bad/></type></defs>",
                        ),
                        (
                            "main.xml",
                            r#"
    <!-- include: x.xml -->
    <module name="main">
<script name="main"><text>x</text></script>
</module>
    "#,
                        ),
                    ]),
                    "XML_TYPE_CHILD_INVALID",
                ),
                (
                    "type field duplicate",
                    map(&[
                        (
                            "x.xml",
                            "<defs name=\"x\"><type name=\"A\"><field name=\"v\" type=\"int\"/><field name=\"v\" type=\"int\"/></type></defs>",
                        ),
                        (
                            "main.xml",
                            r#"
    <!-- include: x.xml -->
    <module name="main">
<script name="main"><text>x</text></script>
</module>
    "#,
                        ),
                    ]),
                    "TYPE_FIELD_DUPLICATE",
                ),
                (
                    "function duplicate",
                    map(&[
                        (
                            "x.xml",
                            "<defs name=\"x\"><function name=\"f\" return=\"int:r\">r=1;</function><function name=\"f\" return=\"int:r\">r=2;</function></defs>",
                        ),
                        (
                            "main.xml",
                            r#"
    <!-- include: x.xml -->
    <module name="main">
<script name="main"><text>x</text></script>
</module>
    "#,
                        ),
                    ]),
                    "FUNCTION_DECL_DUPLICATE",
                ),
                (
                    "unknown custom type in var",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><var name=\"x\" type=\"Unknown\"/></script>",
                    )]),
                    "TYPE_UNKNOWN",
                ),
                (
                    "choice child invalid",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><choice text=\"c\"><bad/></choice></script>",
                    )]),
                    "XML_CHOICE_CHILD_INVALID",
                ),
                (
                    "choice fall_over with when forbidden",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><choice text=\"c\"><option text=\"a\" fall_over=\"true\" when=\"true\"/></choice></script>",
                    )]),
                    "XML_OPTION_FALL_OVER_WHEN_FORBIDDEN",
                ),
                (
                    "choice fall_over duplicate",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><choice text=\"c\"><option text=\"a\" fall_over=\"true\"/><option text=\"b\" fall_over=\"true\"/></choice></script>",
                    )]),
                    "XML_OPTION_FALL_OVER_DUPLICATE",
                ),
                (
                    "choice fall_over not last",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><choice text=\"c\"><option text=\"a\" fall_over=\"true\"/><option text=\"b\"/></choice></script>",
                    )]),
                    "XML_OPTION_FALL_OVER_NOT_LAST",
                ),
                (
                    "dynamic options template required",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><choice text=\"c\"><dynamic-options array=\"arr\" item=\"it\"/></choice></script>",
                    )]),
                    "XML_DYNAMIC_OPTIONS_TEMPLATE_REQUIRED",
                ),
                (
                    "dynamic options child invalid",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><choice text=\"c\"><dynamic-options array=\"arr\" item=\"it\"><option text=\"a\"/><option text=\"b\"/></dynamic-options></choice></script>",
                    )]),
                    "XML_DYNAMIC_OPTIONS_CHILD_INVALID",
                ),
                (
                    "dynamic option once unsupported",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><choice text=\"c\"><dynamic-options array=\"arr\" item=\"it\"><option text=\"a\" once=\"true\"/></dynamic-options></choice></script>",
                    )]),
                    "XML_DYNAMIC_OPTION_ONCE_UNSUPPORTED",
                ),
                (
                    "dynamic option fall_over unsupported",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><choice text=\"c\"><dynamic-options array=\"arr\" item=\"it\"><option text=\"a\" fall_over=\"true\"/></dynamic-options></choice></script>",
                    )]),
                    "XML_DYNAMIC_OPTION_FALL_OVER_UNSUPPORTED",
                ),
                (
                    "dynamic options reserved item",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><choice text=\"c\"><dynamic-options array=\"arr\" item=\"__sl_it\"><option text=\"a\"/></dynamic-options></choice></script>",
                    )]),
                    "NAME_RESERVED_PREFIX",
                ),
                (
                    "dynamic options reserved index",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><choice text=\"c\"><dynamic-options array=\"arr\" item=\"it\" index=\"__sl_i\"><option text=\"a\"/></dynamic-options></choice></script>",
                    )]),
                    "NAME_RESERVED_PREFIX",
                ),
                (
                    "input default unsupported",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><input var=\"x\" text=\"p\" default=\"d\"/></script>",
                    )]),
                    "XML_INPUT_DEFAULT_UNSUPPORTED",
                ),
                (
                    "input content forbidden",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><input var=\"x\" text=\"p\">x</input></script>",
                    )]),
                    "XML_INPUT_CONTENT_FORBIDDEN",
                ),
                (
                    "return ref unsupported",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><return script=\"next\" args=\"ref:x\"/></script>",
                    )]),
                    "XML_RETURN_REF_UNSUPPORTED",
                ),
                (
                    "removed node",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><set/></script>",
                    )]),
                    "XML_REMOVED_NODE",
                ),
                (
                    "else at top level",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><else/></script>",
                    )]),
                    "XML_ELSE_POSITION",
                ),
                (
                    "break outside while",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><break/></script>",
                    )]),
                    "XML_BREAK_OUTSIDE_WHILE",
                ),
                (
                    "continue outside while or option",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><continue/></script>",
                    )]),
                    "XML_CONTINUE_OUTSIDE_WHILE_OR_OPTION",
                ),
                (
                    "call args parse error",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><call script=\"s\" args=\"ref:\"/></script>",
                    )]),
                    "CALL_ARGS_PARSE_ERROR",
                ),
                (
                    "script args reserved prefix",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\" args=\"int:__sl_x\"><text>x</text></script>",
                    )]),
                    "NAME_RESERVED_PREFIX",
                ),
                (
                    "loop times template unsupported",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><loop times=\"${n}\"><text>x</text></loop></script>",
                    )]),
                    "XML_LOOP_TIMES_TEMPLATE_UNSUPPORTED",
                ),
                (
                    "text inline required",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><text/></script>",
                    )]),
                    "XML_EMPTY_NODE_CONTENT",
                ),
                (
                    "text once bool invalid",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><text once=\"bad\">x</text></script>",
                    )]),
                    "XML_ATTR_BOOL_INVALID",
                ),
                (
                    "if missing when",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><if><text>x</text></if></script>",
                    )]),
                    "XML_MISSING_ATTR",
                ),
                (
                    "while missing when",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><while><text>x</text></while></script>",
                    )]),
                    "XML_MISSING_ATTR",
                ),
                (
                    "choice option text required",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><choice text=\"c\"><option><text>x</text></option></choice></script>",
                    )]),
                    "XML_MISSING_ATTR",
                ),
                (
                    "choice option once bool invalid",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><choice text=\"c\"><option text=\"a\" once=\"bad\"/></choice></script>",
                    )]),
                    "XML_ATTR_BOOL_INVALID",
                ),
                (
                    "dynamic options array required",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><choice text=\"c\"><dynamic-options item=\"it\"><option text=\"a\"/></dynamic-options></choice></script>",
                    )]),
                    "XML_MISSING_ATTR",
                ),
                (
                    "dynamic options item required",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><choice text=\"c\"><dynamic-options array=\"arr\"><option text=\"a\"/></dynamic-options></choice></script>",
                    )]),
                    "XML_MISSING_ATTR",
                ),
                (
                    "dynamic option text required",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><choice text=\"c\"><dynamic-options array=\"arr\" item=\"it\"><option/></dynamic-options></choice></script>",
                    )]),
                    "XML_MISSING_ATTR",
                ),
                (
                    "input var missing",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><input text=\"p\"/></script>",
                    )]),
                    "XML_MISSING_ATTR",
                ),
                (
                    "input text missing",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><var name=\"n\" type=\"string\">\"\"</var><input var=\"n\"/></script>",
                    )]),
                    "XML_MISSING_ATTR",
                ),
                (
                    "call script missing",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><call/></script>",
                    )]),
                    "XML_MISSING_ATTR",
                ),
                (
                    "return args parse error",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><return script=\"s\" args=\"ref:\"/></script>",
                    )]),
                    "CALL_ARGS_PARSE_ERROR",
                ),
                (
                    "var missing name",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><var type=\"int\">1</var></script>",
                    )]),
                    "XML_MISSING_ATTR",
                ),
                (
                    "var missing type",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><var name=\"x\">1</var></script>",
                    )]),
                    "XML_MISSING_ATTR",
                ),
                (
                    "var type parse error",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><var name=\"x\" type=\"#{ }\">1</var></script>",
                    )]),
                    "TYPE_PARSE_ERROR",
                ),
            ];

        for (name, files, expected_code) in cases {
            let error =
                compile_project_bundle_from_xml_map(&files).expect_err("error should exist");
            assert_eq!(error.code, expected_code, "case: {}", name);
        }
    }

    #[test]
    fn compiler_private_helpers_cover_remaining_paths() {
        assert_eq!(
            script_type_kind(&ScriptType::Primitive {
                name: "int".to_string()
            }),
            "primitive"
        );
        assert_eq!(
            script_type_kind(&ScriptType::Object {
                type_name: "Obj".to_string(),
                fields: BTreeMap::new()
            }),
            "object"
        );

        assert_eq!(
            resolve_import_path("scripts/main.xml", "/shared.xml"),
            "shared.xml"
        );
        let reachable = collect_reachable_files("missing.xml", &BTreeMap::new());
        assert!(reachable.contains("missing.xml"));

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
        assert_eq!(script_type_kind(&array_ty), "array");
        let map_ty = resolve_type_expr_with_lookup(
            &ParsedTypeExpr::Map(Box::new(ParsedTypeExpr::Primitive("string".to_string()))),
            &BTreeMap::new(),
            &mut resolved_for_lookup,
            &mut visiting_for_lookup,
            &span,
        )
        .expect("map lookup should resolve");
        assert_eq!(script_type_kind(&map_ty), "map");

        let array = resolve_type_expr(
            &ParsedTypeExpr::Array(Box::new(ParsedTypeExpr::Primitive("int".to_string()))),
            &BTreeMap::new(),
            &span,
        )
        .expect("array should resolve");
        assert_eq!(script_type_kind(&array), "array");
        let map_resolved = resolve_type_expr(
            &ParsedTypeExpr::Map(Box::new(ParsedTypeExpr::Primitive("int".to_string()))),
            &BTreeMap::new(),
            &span,
        )
        .expect("map should resolve");
        assert_eq!(script_type_kind(&map_resolved), "map");

        let non_script_root = xml_element("defs", &[("name", "x")], Vec::new());
        let compile_root_error = compile_script(CompileScriptOptions {
            script_path: "x.xml",
            root: &non_script_root,
            qualified_script_name: None,
            module_name: None,
            visible_types: &BTreeMap::new(),
            visible_functions: &BTreeMap::new(),
            visible_defs_globals: &BTreeMap::new(),
        })
        .expect_err("compile_script should require script root");
        assert_eq!(compile_root_error.code, "XML_ROOT_INVALID");

        let missing_name_root = xml_element("script", &[], Vec::new());
        let missing_name_error = compile_script(CompileScriptOptions {
            script_path: "x.xml",
            root: &missing_name_root,
            qualified_script_name: None,
            module_name: None,
            visible_types: &BTreeMap::new(),
            visible_functions: &BTreeMap::new(),
            visible_defs_globals: &BTreeMap::new(),
        })
        .expect_err("compile_script should require script name");
        assert_eq!(missing_name_error.code, "XML_MISSING_ATTR");

        let reserved_name_root = xml_element("script", &[("name", "__bad")], Vec::new());
        let reserved_name_error = compile_script(CompileScriptOptions {
            script_path: "x.xml",
            root: &reserved_name_root,
            qualified_script_name: None,
            module_name: None,
            visible_types: &BTreeMap::new(),
            visible_functions: &BTreeMap::new(),
            visible_defs_globals: &BTreeMap::new(),
        })
        .expect_err("compile_script should reject reserved name");
        assert_eq!(reserved_name_error.code, "NAME_RESERVED_PREFIX");

        let reserved_var_root = xml_element(
            "script",
            &[("name", "main")],
            vec![XmlNode::Element(xml_element(
                "var",
                &[("name", "__bad"), ("type", "int")],
                vec![xml_text("1")],
            ))],
        );
        let reserved_var_error = compile_script(CompileScriptOptions {
            script_path: "x.xml",
            root: &reserved_var_root,
            qualified_script_name: Some("x.main"),
            module_name: Some("x"),
            visible_types: &BTreeMap::new(),
            visible_functions: &BTreeMap::new(),
            visible_defs_globals: &BTreeMap::new(),
        })
        .expect_err("compile_script should reject reserved var names");
        assert_eq!(reserved_var_error.code, "NAME_RESERVED_PREFIX");

        let no_module_root =
            parse_xml_document(r#"<script name="main"><call script="next"/></script>"#)
                .expect("xml")
                .root;
        let no_module_ir = compile_script(CompileScriptOptions {
            script_path: "x.xml",
            root: &no_module_root,
            qualified_script_name: Some("main"),
            module_name: None,
            visible_types: &BTreeMap::new(),
            visible_functions: &BTreeMap::new(),
            visible_defs_globals: &BTreeMap::new(),
        })
        .expect("compile without module name");
        let root_group = no_module_ir
            .groups
            .get(&no_module_ir.root_group_id)
            .expect("root group");
        let node_debug = format!("{:?}", &root_group.nodes[0]);
        assert!(node_debug.contains("target_script: \"next\""));

        let rich_script = map(&[(
            "main.xml",
            r#"
    <module name="main">
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
    </module>
    "#,
        )]);
        let compiled =
            compile_project_bundle_from_xml_map(&rich_script).expect("compile should pass");
        let main = compiled.scripts.get("main.main").expect("main script");
        let root_group = main.groups.get(&main.root_group_id).expect("root group");
        let while_count = root_group
            .nodes
            .iter()
            .filter(|node| matches!(node, ScriptNode::While { .. }))
            .count();
        let if_count = root_group
            .nodes
            .iter()
            .filter(|node| matches!(node, ScriptNode::If { .. }))
            .count();
        assert_eq!(while_count, 1);
        assert_eq!(if_count, 1);

        let defs_resolution = map(&[
            (
                "shared.xml",
                r##"
    <module name="shared">
      <type name="Obj">
        <field name="values" type="#{int[]}"/>
      </type>
      <function name="build" return="Obj:r">
        r = #{values: #{a: [1]}};
      </function>
    </module>
    "##,
            ),
            (
                "main.xml",
                r#"
	    <!-- include: shared.xml -->
	    <module name="main">
	<script name="main">
	      <var name="x" type="shared.Obj"/>
	    </script>
	</module>
	    "#,
            ),
        ]);
        let _ = compile_project_bundle_from_xml_map(&defs_resolution)
            .expect("defs return/field type resolution should pass");

        let mut builder_ok = GroupBuilder::new("manual.xml");
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

        let mut loop_builder = GroupBuilder::new("loop.xml");
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
        let choice_node = ScriptNode::Choice {
            id: "ch1".to_string(),
            prompt_text: "Pick".to_string(),
            entries: Vec::new(),
            location: SourceSpan::synthetic(),
        };
        let choice_id = node_id(&choice_node);
        assert_eq!(choice_id, "ch1");
        let break_node = ScriptNode::Break {
            id: "b1".to_string(),
            location: SourceSpan::synthetic(),
        };
        let break_id = node_id(&break_node);
        assert_eq!(break_id, "b1");
        let continue_node = ScriptNode::Continue {
            id: "k1".to_string(),
            target: ContinueTarget::Choice,
            location: SourceSpan::synthetic(),
        };
        let continue_id = node_id(&continue_node);
        assert_eq!(continue_id, "k1");

        let mut choice_builder = GroupBuilder::new("choice.xml");
        let choice_group = choice_builder.next_group_id();
        compile_group(
            &choice_group,
            None,
            &xml_element(
                "script",
                &[("name", "main")],
                vec![XmlNode::Element(xml_element(
                    "choice",
                    &[("text", "Pick")],
                    vec![
                        XmlNode::Element(xml_element(
                            "option",
                            &[("text", "A")],
                            vec![XmlNode::Element(xml_element("continue", &[], Vec::new()))],
                        )),
                        XmlNode::Element(xml_element(
                            "option",
                            &[("text", "B"), ("fall_over", "true")],
                            Vec::new(),
                        )),
                    ],
                ))],
            ),
            &mut choice_builder,
            &BTreeMap::new(),
            &BTreeMap::new(),
            CompileGroupMode::new(0, false),
        )
        .expect("option continue and last fall_over should compile");

        let dynamic_choice = map(&[(
            "main.xml",
            r#"
    <script name="main">
      <var name="arr" type="int[]">[1,2]</var>
      <choice text="Pick">
        <option text="A"><text>A</text></option>
        <dynamic-options array="arr" item="it" index="i">
          <option text="${it}:${i}" when="it > 0">
            <text>dyn</text>
          </option>
        </dynamic-options>
      </choice>
    </script>
</module>
    "#,
        )]);
        let dynamic_compiled =
            compile_project_bundle_from_xml_map(&dynamic_choice).expect("dynamic choice compile");
        let dynamic_main = dynamic_compiled
            .scripts
            .get("main.main")
            .expect("main script");
        let dynamic_root = dynamic_main
            .groups
            .get(&dynamic_main.root_group_id)
            .expect("root group");
        let dynamic_choice_count = dynamic_root
            .nodes
            .iter()
            .filter(|node| match node {
                ScriptNode::Choice { entries, .. } => {
                    entries
                        .iter()
                        .filter(|entry| matches!(entry, ChoiceEntry::Dynamic { .. }))
                        .count()
                        > 0
                }
                _ => false,
            })
            .count();
        assert_eq!(dynamic_choice_count, 1);

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
            crate::defaults::slvalue_from_json(JsonValue::Null),
            SlValue::String("null".to_string())
        );
    }
}
