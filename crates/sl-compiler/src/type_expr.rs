use crate::*;

#[cfg(test)]
pub(crate) fn resolve_named_type(
    name: &str,
    type_decls_map: &BTreeMap<String, ParsedTypeDecl>,
    resolved: &mut BTreeMap<String, ScriptType>,
    visiting: &mut HashSet<String>,
) -> Result<ScriptType, ScriptLangError> {
    let empty_aliases = BTreeMap::new();
    resolve_named_type_with_aliases(name, type_decls_map, &empty_aliases, resolved, visiting)
}

pub(crate) fn resolve_named_type_with_aliases(
    name: &str,
    type_decls_map: &BTreeMap<String, ParsedTypeDecl>,
    type_aliases: &BTreeMap<String, String>,
    resolved: &mut BTreeMap<String, ScriptType>,
    visiting: &mut HashSet<String>,
) -> Result<ScriptType, ScriptLangError> {
    let lookup_name = if type_decls_map.contains_key(name) {
        name.to_string()
    } else if let Some(qualified) = type_aliases.get(name) {
        qualified.clone()
    } else {
        return Err(ScriptLangError::new(
            "TYPE_UNKNOWN",
            format!("Unknown type \"{}\".", name),
        ));
    };

    if let Some(found) = resolved.get(&lookup_name) {
        return Ok(found.clone());
    }

    if !visiting.insert(lookup_name.clone()) {
        return Err(ScriptLangError::new(
            "TYPE_DECL_RECURSIVE",
            format!("Recursive type declaration detected for \"{}\".", name),
        ));
    }

    let Some(decl) = type_decls_map.get(&lookup_name) else {
        visiting.remove(&lookup_name);
        return Err(ScriptLangError::new(
            "TYPE_UNKNOWN",
            format!("Unknown type \"{}\".", name),
        ));
    };

    let resolved_type = if !decl.enum_members.is_empty() {
        visiting.remove(&lookup_name);
        ScriptType::Enum {
            type_name: lookup_name.clone(),
            members: decl.enum_members.clone(),
        }
    } else {
        let mut fields = BTreeMap::new();
        for field in &decl.fields {
            if fields.contains_key(&field.name) {
                visiting.remove(&lookup_name);
                return Err(ScriptLangError::with_span(
                    "TYPE_FIELD_DUPLICATE",
                    format!("Duplicate field \"{}\" in type \"{}\".", field.name, name),
                    field.location.clone(),
                ));
            }
            let field_type = resolve_type_expr_with_lookup_with_aliases(
                &field.type_expr,
                type_decls_map,
                type_aliases,
                resolved,
                visiting,
                &field.location,
            )?;
            fields.insert(field.name.clone(), field_type);
        }

        visiting.remove(&lookup_name);
        ScriptType::Object {
            type_name: lookup_name.clone(),
            fields,
        }
    };
    resolved.insert(lookup_name, resolved_type.clone());
    Ok(resolved_type)
}

#[cfg(test)]
pub(crate) fn resolve_type_expr_with_lookup(
    expr: &ParsedTypeExpr,
    type_decls_map: &BTreeMap<String, ParsedTypeDecl>,
    resolved: &mut BTreeMap<String, ScriptType>,
    visiting: &mut HashSet<String>,
    span: &SourceSpan,
) -> Result<ScriptType, ScriptLangError> {
    let empty_aliases = BTreeMap::new();
    resolve_type_expr_with_lookup_with_aliases(
        expr,
        type_decls_map,
        &empty_aliases,
        resolved,
        visiting,
        span,
    )
}

pub(crate) fn resolve_type_expr_with_lookup_with_aliases(
    expr: &ParsedTypeExpr,
    type_decls_map: &BTreeMap<String, ParsedTypeDecl>,
    type_aliases: &BTreeMap<String, String>,
    resolved: &mut BTreeMap<String, ScriptType>,
    visiting: &mut HashSet<String>,
    span: &SourceSpan,
) -> Result<ScriptType, ScriptLangError> {
    match expr {
        ParsedTypeExpr::Script => Ok(ScriptType::Script),
        ParsedTypeExpr::Primitive(name) => Ok(ScriptType::Primitive { name: name.clone() }),
        ParsedTypeExpr::Array(element_type) => {
            let resolved_element = resolve_type_expr_with_lookup_with_aliases(
                element_type,
                type_decls_map,
                type_aliases,
                resolved,
                visiting,
                span,
            )?;
            Ok(ScriptType::Array {
                element_type: Box::new(resolved_element),
            })
        }
        ParsedTypeExpr::Map(value_type) => {
            let resolved_value = resolve_type_expr_with_lookup_with_aliases(
                value_type,
                type_decls_map,
                type_aliases,
                resolved,
                visiting,
                span,
            )?;
            Ok(ScriptType::Map {
                key_type: "string".to_string(),
                value_type: Box::new(resolved_value),
            })
        }
        ParsedTypeExpr::Custom(name) => {
            match resolve_named_type_with_aliases(
                name,
                type_decls_map,
                type_aliases,
                resolved,
                visiting,
            ) {
                Ok(value) => Ok(value),
                Err(_) => Err(ScriptLangError::with_span(
                    "TYPE_UNKNOWN",
                    format!("Unknown custom type \"{}\".", name),
                    span.clone(),
                )),
            }
        }
    }
}

pub(crate) fn resolve_type_expr(
    expr: &ParsedTypeExpr,
    resolved_types: &BTreeMap<String, ScriptType>,
    span: &SourceSpan,
) -> Result<ScriptType, ScriptLangError> {
    match expr {
        ParsedTypeExpr::Script => Ok(ScriptType::Script),
        ParsedTypeExpr::Primitive(name) => Ok(ScriptType::Primitive { name: name.clone() }),
        ParsedTypeExpr::Array(element_type) => Ok(ScriptType::Array {
            element_type: Box::new(resolve_type_expr(element_type, resolved_types, span)?),
        }),
        ParsedTypeExpr::Map(value_type) => Ok(ScriptType::Map {
            key_type: "string".to_string(),
            value_type: Box::new(resolve_type_expr(value_type, resolved_types, span)?),
        }),
        ParsedTypeExpr::Custom(name) => match resolved_types.get(name).cloned() {
            Some(value) => Ok(value),
            None => Err(ScriptLangError::with_span(
                "TYPE_UNKNOWN",
                format!("Unknown custom type \"{}\".", name),
                span.clone(),
            )),
        },
    }
}

pub(crate) fn resolve_type_expr_in_namespace(
    expr: &ParsedTypeExpr,
    resolved_types: &BTreeMap<String, ScriptType>,
    namespace: &str,
    span: &SourceSpan,
) -> Result<ScriptType, ScriptLangError> {
    match expr {
        ParsedTypeExpr::Custom(name) if !name.contains('.') => {
            let qualified = format!("{}.{}", namespace, name);
            if let Some(value) = resolved_types.get(&qualified).cloned() {
                Ok(value)
            } else {
                resolve_type_expr(expr, resolved_types, span)
            }
        }
        _ => resolve_type_expr(expr, resolved_types, span),
    }
}

#[cfg(test)]
pub(crate) fn parse_type_declaration_node(
    node: &XmlElementNode,
) -> Result<ParsedTypeDecl, ScriptLangError> {
    parse_type_declaration_node_with_namespace(node, "module", AccessLevel::Private)
}

pub(crate) fn parse_type_declaration_node_with_namespace(
    node: &XmlElementNode,
    namespace: &str,
    default_access: AccessLevel,
) -> Result<ParsedTypeDecl, ScriptLangError> {
    let name = get_required_non_empty_attr(node, "name")?;
    assert_decl_name_not_reserved_or_rhai_keyword(&name, "type", node.location.clone())?;
    let access = parse_access_attr(node, "access", default_access)?;

    let mut fields = Vec::new();
    let mut seen = HashSet::new();

    for child in element_children(node) {
        if child.name != "field" {
            return Err(ScriptLangError::with_span(
                "XML_TYPE_CHILD_INVALID",
                format!("Unsupported child <{}> under <type>.", child.name),
                child.location.clone(),
            ));
        }

        let field_name = get_required_non_empty_attr(child, "name")?;
        assert_decl_name_not_reserved_or_rhai_keyword(
            &field_name,
            "type field",
            child.location.clone(),
        )?;
        if !seen.insert(field_name.clone()) {
            return Err(ScriptLangError::with_span(
                "TYPE_FIELD_DUPLICATE",
                format!("Duplicate field \"{}\" in type \"{}\".", field_name, name),
                child.location.clone(),
            ));
        }

        let field_type_raw = get_required_non_empty_attr(child, "type")?;
        let field_type = parse_type_expr(&field_type_raw, &child.location)?;
        fields.push(ParsedTypeFieldDecl {
            name: field_name,
            type_expr: field_type,
            location: child.location.clone(),
        });
    }

    let qualified_name = format!("{}.{}", namespace, name);
    Ok(ParsedTypeDecl {
        name,
        qualified_name,
        access,
        fields,
        enum_members: Vec::new(),
        location: node.location.clone(),
    })
}

pub(crate) fn parse_enum_declaration_node_with_namespace(
    node: &XmlElementNode,
    namespace: &str,
    default_access: AccessLevel,
) -> Result<ParsedTypeDecl, ScriptLangError> {
    let name = get_required_non_empty_attr(node, "name")?;
    assert_decl_name_not_reserved_or_rhai_keyword(&name, "enum", node.location.clone())?;
    let access = parse_access_attr(node, "access", default_access)?;

    let mut members = Vec::new();
    let mut seen = HashSet::new();
    for child in element_children(node) {
        if child.name != "member" {
            return Err(ScriptLangError::with_span(
                "XML_ENUM_CHILD_INVALID",
                format!("Unsupported child <{}> under <enum>.", child.name),
                child.location.clone(),
            ));
        }
        let member_name = get_required_non_empty_attr(child, "name")?;
        assert_decl_name_not_reserved_or_rhai_keyword(
            &member_name,
            "enum member",
            child.location.clone(),
        )?;
        if !seen.insert(member_name.clone()) {
            return Err(ScriptLangError::with_span(
                "ENUM_MEMBER_DUPLICATE",
                format!(
                    "Duplicate enum member \"{}\" in enum \"{}\".",
                    member_name, name
                ),
                child.location.clone(),
            ));
        }
        if has_any_child_content(child) {
            return Err(ScriptLangError::with_span(
                "XML_ENUM_MEMBER_CONTENT_FORBIDDEN",
                "<member> cannot contain child nodes or inline text.",
                child.location.clone(),
            ));
        }
        members.push(member_name);
    }
    if members.is_empty() {
        return Err(ScriptLangError::with_span(
            "ENUM_DECL_EMPTY",
            format!("Enum \"{}\" must declare at least one <member>.", name),
            node.location.clone(),
        ));
    }

    let qualified_name = format!("{}.{}", namespace, name);
    Ok(ParsedTypeDecl {
        name,
        qualified_name,
        access,
        fields: Vec::new(),
        enum_members: members,
        location: node.location.clone(),
    })
}

#[cfg(test)]
pub(crate) fn parse_function_declaration_node(
    node: &XmlElementNode,
) -> Result<ParsedFunctionDecl, ScriptLangError> {
    parse_function_declaration_node_with_namespace(node, "module", AccessLevel::Private)
}

pub(crate) fn parse_function_declaration_node_with_namespace(
    node: &XmlElementNode,
    namespace: &str,
    default_access: AccessLevel,
) -> Result<ParsedFunctionDecl, ScriptLangError> {
    let name = get_required_non_empty_attr(node, "name")?;
    assert_decl_name_not_reserved_or_rhai_keyword(&name, "function", node.location.clone())?;
    let access = parse_access_attr(node, "access", default_access)?;

    let params = parse_function_args(node)?;
    let return_binding = parse_function_return(node)?;
    let code = parse_inline_required_no_element_children(node)?;

    let qualified_name = format!("{}.{}", namespace, name);
    Ok(ParsedFunctionDecl {
        name,
        qualified_name,
        access,
        params,
        return_binding,
        code,
        location: node.location.clone(),
    })
}

#[cfg(test)]
mod type_expr_tests {
    use super::*;
    use crate::compiler_test_support::*;

    fn script_type_kind(ty: &ScriptType) -> &'static str {
        match ty {
            ScriptType::Primitive { .. } => "primitive",
            ScriptType::Enum { .. } => "enum",
            ScriptType::Script => "script",
            ScriptType::Array { .. } => "array",
            ScriptType::Map { .. } => "map",
            ScriptType::Object { .. } => "object",
        }
    }

    #[test]
    fn type_resolution_helpers_cover_nested_array_and_map_paths() {
        assert_eq!(script_type_kind(&ScriptType::Script), "script");
        let span = SourceSpan::synthetic();
        let mut resolved = BTreeMap::new();
        let mut visiting = HashSet::new();
        let type_map = BTreeMap::from([(
            "Obj".to_string(),
            ParsedTypeDecl {
                name: "Obj".to_string(),
                qualified_name: "Obj".to_string(),
                access: AccessLevel::Private,
                fields: vec![ParsedTypeFieldDecl {
                    name: "n".to_string(),
                    type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                    location: span.clone(),
                }],
                enum_members: Vec::new(),
                location: span.clone(),
            },
        )]);

        let array = resolve_type_expr_with_lookup(
            &ParsedTypeExpr::Array(Box::new(ParsedTypeExpr::Custom("Obj".to_string()))),
            &type_map,
            &mut resolved,
            &mut visiting,
            &span,
        )
        .expect("array custom type should resolve");
        assert_eq!(script_type_kind(&array), "array");

        let map = resolve_type_expr_with_lookup(
            &ParsedTypeExpr::Map(Box::new(ParsedTypeExpr::Custom("Obj".to_string()))),
            &type_map,
            &mut resolved,
            &mut visiting,
            &span,
        )
        .expect("map custom type should resolve");
        assert_eq!(script_type_kind(&map), "map");

        let script = resolve_type_expr_with_lookup(
            &ParsedTypeExpr::Script,
            &type_map,
            &mut resolved,
            &mut visiting,
            &span,
        )
        .expect("script type should resolve");
        assert_eq!(script_type_kind(&script), "script");

        let array_err = resolve_type_expr_with_lookup(
            &ParsedTypeExpr::Array(Box::new(ParsedTypeExpr::Custom("Missing".to_string()))),
            &type_map,
            &mut resolved,
            &mut visiting,
            &span,
        )
        .expect_err("unknown array element type should fail");
        assert_eq!(array_err.code, "TYPE_UNKNOWN");

        let map_err = resolve_type_expr_with_lookup(
            &ParsedTypeExpr::Map(Box::new(ParsedTypeExpr::Custom("Missing".to_string()))),
            &type_map,
            &mut resolved,
            &mut visiting,
            &span,
        )
        .expect_err("unknown map value type should fail");
        assert_eq!(map_err.code, "TYPE_UNKNOWN");

        let _ = parse_type_expr("#{int[]}", &span).expect("type should parse");
        let nested_array_error =
            parse_type_expr("Custom[]]", &span).expect_err("invalid nested array syntax");
        assert_eq!(nested_array_error.code, "TYPE_PARSE_ERROR");
        let nested_map_error =
            parse_type_expr("#{#{}}", &span).expect_err("invalid nested map syntax");
        assert_eq!(nested_map_error.code, "TYPE_PARSE_ERROR");

        let type_node = xml_element(
            "type",
            &[("name", "Bag")],
            vec![XmlNode::Element(xml_element(
                "field",
                &[("name", "values"), ("type", "#{int[]}")],
                Vec::new(),
            ))],
        );
        let parsed = parse_type_declaration_node(&type_node).expect("type node should parse");
        assert_eq!(parsed.fields.len(), 1);

        let mut resolved_types = BTreeMap::new();
        resolved_types.insert(
            "Obj".to_string(),
            ScriptType::Object {
                type_name: "Obj".to_string(),
                fields: BTreeMap::new(),
            },
        );
        let resolved_array = resolve_type_expr(
            &ParsedTypeExpr::Array(Box::new(ParsedTypeExpr::Custom("Obj".to_string()))),
            &resolved_types,
            &span,
        )
        .expect("resolve array custom type");
        assert_eq!(script_type_kind(&resolved_array), "array");
        let resolved_map = resolve_type_expr(
            &ParsedTypeExpr::Map(Box::new(ParsedTypeExpr::Custom("Obj".to_string()))),
            &resolved_types,
            &span,
        )
        .expect("resolve map custom type");
        assert_eq!(script_type_kind(&resolved_map), "map");

        let type_node_reserved = xml_element(
            "type",
            &[("name", "Bag")],
            vec![XmlNode::Element(xml_element(
                "field",
                &[("name", "__sl_bad"), ("type", "int")],
                Vec::new(),
            ))],
        );
        let field_error =
            parse_type_declaration_node(&type_node_reserved).expect_err("reserved field name");
        assert_eq!(field_error.code, "NAME_RESERVED_PREFIX");
        let type_node_keyword = xml_element(
            "type",
            &[("name", "Bag")],
            vec![XmlNode::Element(xml_element(
                "field",
                &[("name", "shared"), ("type", "int")],
                Vec::new(),
            ))],
        );
        let field_error =
            parse_type_declaration_node(&type_node_keyword).expect_err("keyword field name");
        assert_eq!(field_error.code, "NAME_RHAI_KEYWORD_RESERVED");

        let unknown_in_array = resolve_type_expr(
            &ParsedTypeExpr::Array(Box::new(ParsedTypeExpr::Custom("Missing".to_string()))),
            &resolved_types,
            &span,
        )
        .expect_err("unknown custom type in array should fail");
        assert_eq!(unknown_in_array.code, "TYPE_UNKNOWN");
        let unknown_in_map = resolve_type_expr(
            &ParsedTypeExpr::Map(Box::new(ParsedTypeExpr::Custom("Missing".to_string()))),
            &resolved_types,
            &span,
        )
        .expect_err("unknown custom type in map should fail");
        assert_eq!(unknown_in_map.code, "TYPE_UNKNOWN");

        let type_missing_name = parse_type_declaration_node(&xml_element(
            "type",
            &[],
            vec![XmlNode::Element(xml_element(
                "field",
                &[("name", "v"), ("type", "int")],
                Vec::new(),
            ))],
        ))
        .expect_err("type name should be required");
        assert_eq!(type_missing_name.code, "XML_MISSING_ATTR");

        let type_reserved_name = parse_type_declaration_node(&xml_element(
            "type",
            &[("name", "__sl_type")],
            vec![XmlNode::Element(xml_element(
                "field",
                &[("name", "v"), ("type", "int")],
                Vec::new(),
            ))],
        ))
        .expect_err("type name cannot be reserved");
        assert_eq!(type_reserved_name.code, "NAME_RESERVED_PREFIX");
        let type_keyword_name = parse_type_declaration_node(&xml_element(
            "type",
            &[("name", "shared")],
            vec![XmlNode::Element(xml_element(
                "field",
                &[("name", "v"), ("type", "int")],
                Vec::new(),
            ))],
        ))
        .expect_err("type name cannot be keyword");
        assert_eq!(type_keyword_name.code, "NAME_RHAI_KEYWORD_RESERVED");

        let field_missing_name = parse_type_declaration_node(&xml_element(
            "type",
            &[("name", "T")],
            vec![XmlNode::Element(xml_element(
                "field",
                &[("type", "int")],
                Vec::new(),
            ))],
        ))
        .expect_err("field name should be required");
        assert_eq!(field_missing_name.code, "XML_MISSING_ATTR");

        let field_missing_type = parse_type_declaration_node(&xml_element(
            "type",
            &[("name", "T")],
            vec![XmlNode::Element(xml_element(
                "field",
                &[("name", "v")],
                Vec::new(),
            ))],
        ))
        .expect_err("field type should be required");
        assert_eq!(field_missing_type.code, "XML_MISSING_ATTR");

        let field_bad_type = parse_type_declaration_node(&xml_element(
            "type",
            &[("name", "T")],
            vec![XmlNode::Element(xml_element(
                "field",
                &[("name", "v"), ("type", "#{ }")],
                Vec::new(),
            ))],
        ))
        .expect_err("field type syntax should be valid");
        assert_eq!(field_bad_type.code, "TYPE_PARSE_ERROR");

        let function_missing_name = parse_function_declaration_node(&xml_element(
            "function",
            &[("return", "int:r")],
            vec![xml_text("r = 1;")],
        ))
        .expect_err("function name should be required");
        assert_eq!(function_missing_name.code, "XML_MISSING_ATTR");

        let function_reserved_name = parse_function_declaration_node(&xml_element(
            "function",
            &[("name", "__sl_f"), ("return", "int:r")],
            vec![xml_text("r = 1;")],
        ))
        .expect_err("function name cannot be reserved");
        assert_eq!(function_reserved_name.code, "NAME_RESERVED_PREFIX");
        let function_keyword_name = parse_function_declaration_node(&xml_element(
            "function",
            &[("name", "shared"), ("return", "int:r")],
            vec![xml_text("r = 1;")],
        ))
        .expect_err("function name cannot be keyword");
        assert_eq!(function_keyword_name.code, "NAME_RHAI_KEYWORD_RESERVED");

        let function_child_error = parse_function_declaration_node(&xml_element(
            "function",
            &[("name", "f"), ("return", "int:r")],
            vec![XmlNode::Element(xml_element("x", &[], Vec::new()))],
        ))
        .expect_err("function code cannot contain child nodes");
        assert_eq!(function_child_error.code, "XML_FUNCTION_CHILD_NODE_INVALID");

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

        // Test invalid access attribute on type
        let type_invalid_access = parse_type_declaration_node_with_namespace(
            &xml_element(
                "type",
                &[("name", "T"), ("access", "invalid")],
                vec![XmlNode::Element(xml_element(
                    "field",
                    &[("name", "v"), ("type", "int")],
                    vec![],
                ))],
            ),
            "module",
            AccessLevel::Private,
        )
        .expect_err("invalid access should fail");
        assert_eq!(type_invalid_access.code, "XML_ACCESS_INVALID");

        // Test invalid access attribute on function
        let function_invalid_access = parse_function_declaration_node_with_namespace(
            &xml_element(
                "function",
                &[("name", "f"), ("access", "bad"), ("return", "int:r")],
                vec![xml_text("r = 1;")],
            ),
            "module",
            AccessLevel::Private,
        )
        .expect_err("invalid access should fail");
        assert_eq!(function_invalid_access.code, "XML_ACCESS_INVALID");
    }

    #[test]
    fn parse_enum_declaration_covers_various_error_paths() {
        // Test invalid child element under enum
        let enum_invalid_child = parse_enum_declaration_node_with_namespace(
            &xml_element(
                "enum",
                &[("name", "Color")],
                vec![XmlNode::Element(xml_element("not_member", &[], Vec::new()))],
            ),
            "module",
            AccessLevel::Private,
        )
        .expect_err("invalid child should fail");
        assert_eq!(enum_invalid_child.code, "XML_ENUM_CHILD_INVALID");

        // Test duplicate enum member
        let enum_duplicate_member = parse_enum_declaration_node_with_namespace(
            &xml_element(
                "enum",
                &[("name", "Color")],
                vec![
                    XmlNode::Element(xml_element("member", &[("name", "Red")], Vec::new())),
                    XmlNode::Element(xml_element("member", &[("name", "Red")], Vec::new())),
                ],
            ),
            "module",
            AccessLevel::Private,
        )
        .expect_err("duplicate member should fail");
        assert_eq!(enum_duplicate_member.code, "ENUM_MEMBER_DUPLICATE");

        // Test enum member with content (forbidden)
        let enum_member_with_content = parse_enum_declaration_node_with_namespace(
            &xml_element(
                "enum",
                &[("name", "Color")],
                vec![XmlNode::Element(xml_element(
                    "member",
                    &[("name", "Red")],
                    vec![xml_text("some content")],
                ))],
            ),
            "module",
            AccessLevel::Private,
        )
        .expect_err("member with content should fail");
        assert_eq!(
            enum_member_with_content.code,
            "XML_ENUM_MEMBER_CONTENT_FORBIDDEN"
        );

        // Test empty enum (no members)
        let enum_empty = parse_enum_declaration_node_with_namespace(
            &xml_element("enum", &[("name", "Empty")], Vec::new()),
            "module",
            AccessLevel::Private,
        )
        .expect_err("empty enum should fail");
        assert_eq!(enum_empty.code, "ENUM_DECL_EMPTY");

        // Test valid enum declaration
        let enum_valid = parse_enum_declaration_node_with_namespace(
            &xml_element(
                "enum",
                &[("name", "Color")],
                vec![
                    XmlNode::Element(xml_element("member", &[("name", "Red")], Vec::new())),
                    XmlNode::Element(xml_element("member", &[("name", "Green")], Vec::new())),
                    XmlNode::Element(xml_element("member", &[("name", "Blue")], Vec::new())),
                ],
            ),
            "module",
            AccessLevel::Private,
        )
        .expect("valid enum should parse");
        assert_eq!(enum_valid.name, "Color");
        assert_eq!(enum_valid.enum_members.len(), 3);

        // Test enum with reserved member name
        let enum_reserved_name = parse_enum_declaration_node_with_namespace(
            &xml_element(
                "enum",
                &[("name", "Status")],
                vec![XmlNode::Element(xml_element(
                    "member",
                    &[("name", "__sl_bad")],
                    Vec::new(),
                ))],
            ),
            "module",
            AccessLevel::Private,
        )
        .expect_err("reserved member name should fail");
        assert_eq!(enum_reserved_name.code, "NAME_RESERVED_PREFIX");
        let enum_keyword_member = parse_enum_declaration_node_with_namespace(
            &xml_element(
                "enum",
                &[("name", "Status")],
                vec![XmlNode::Element(xml_element(
                    "member",
                    &[("name", "shared")],
                    Vec::new(),
                ))],
            ),
            "module",
            AccessLevel::Private,
        )
        .expect_err("keyword member name should fail");
        assert_eq!(enum_keyword_member.code, "NAME_RHAI_KEYWORD_RESERVED");

        // Test enum with reserved type name
        let enum_reserved_type = parse_enum_declaration_node_with_namespace(
            &xml_element(
                "enum",
                &[("name", "__sl_bad")],
                vec![XmlNode::Element(xml_element(
                    "member",
                    &[("name", "Value")],
                    Vec::new(),
                ))],
            ),
            "module",
            AccessLevel::Private,
        )
        .expect_err("reserved type name should fail");
        assert_eq!(enum_reserved_type.code, "NAME_RESERVED_PREFIX");
        let enum_keyword_type = parse_enum_declaration_node_with_namespace(
            &xml_element(
                "enum",
                &[("name", "shared")],
                vec![XmlNode::Element(xml_element(
                    "member",
                    &[("name", "Value")],
                    Vec::new(),
                ))],
            ),
            "module",
            AccessLevel::Private,
        )
        .expect_err("keyword type name should fail");
        assert_eq!(enum_keyword_type.code, "NAME_RHAI_KEYWORD_RESERVED");

        // Test enum with namespace (qualified name)
        let enum_with_ns = parse_enum_declaration_node_with_namespace(
            &xml_element(
                "enum",
                &[("name", "Color")],
                vec![XmlNode::Element(xml_element(
                    "member",
                    &[("name", "Red")],
                    Vec::new(),
                ))],
            ),
            "my.namespace",
            AccessLevel::Public,
        )
        .expect("enum with namespace should parse");
        assert_eq!(enum_with_ns.qualified_name, "my.namespace.Color");
        assert_eq!(enum_with_ns.access, AccessLevel::Public);

        // Test enum missing name attribute
        let enum_missing_name = parse_enum_declaration_node_with_namespace(
            &xml_element("enum", &[], Vec::new()),
            "module",
            AccessLevel::Private,
        )
        .expect_err("missing name should fail");
        assert_eq!(enum_missing_name.code, "XML_MISSING_ATTR");

        // Test enum with invalid access
        let enum_invalid_access = parse_enum_declaration_node_with_namespace(
            &xml_element(
                "enum",
                &[("name", "Color"), ("access", "bad")],
                vec![XmlNode::Element(xml_element(
                    "member",
                    &[("name", "Red")],
                    Vec::new(),
                ))],
            ),
            "module",
            AccessLevel::Private,
        )
        .expect_err("invalid access should fail");
        assert_eq!(enum_invalid_access.code, "XML_ACCESS_INVALID");

        // Test enum member missing name attribute (line 287 error branch)
        let enum_member_missing_name = parse_enum_declaration_node_with_namespace(
            &xml_element(
                "enum",
                &[("name", "Color")],
                vec![XmlNode::Element(xml_element("member", &[], Vec::new()))],
            ),
            "module",
            AccessLevel::Private,
        )
        .expect_err("member missing name should fail");
        assert_eq!(enum_member_missing_name.code, "XML_MISSING_ATTR");

        // Test enum member with empty name attribute (line 287 error branch)
        let enum_member_empty_name = parse_enum_declaration_node_with_namespace(
            &xml_element(
                "enum",
                &[("name", "Color")],
                vec![XmlNode::Element(xml_element(
                    "member",
                    &[("name", "")],
                    Vec::new(),
                ))],
            ),
            "module",
            AccessLevel::Private,
        )
        .expect_err("member with empty name should fail");
        assert_eq!(enum_member_empty_name.code, "XML_EMPTY_ATTR");
    }

    #[test]
    fn resolve_enum_type_covers_enum_branch() {
        // Test that resolving a type with enum_members creates ScriptType::Enum
        // This covers lines 51-56 in resolve_type_expr_with_lookup_with_aliases
        let span = SourceSpan::synthetic();
        let mut resolved = BTreeMap::new();
        let mut visiting = HashSet::new();

        // Create a type declaration with enum_members
        let type_map = BTreeMap::from([(
            "Status".to_string(),
            ParsedTypeDecl {
                name: "Status".to_string(),
                qualified_name: "Status".to_string(),
                access: AccessLevel::Private,
                fields: Vec::new(),
                enum_members: vec!["Pending".to_string(), "Done".to_string()],
                location: span.clone(),
            },
        )]);

        let result = resolve_type_expr_with_lookup(
            &ParsedTypeExpr::Custom("Status".to_string()),
            &type_map,
            &mut resolved,
            &mut visiting,
            &span,
        )
        .expect("enum type should resolve");
        assert_eq!(script_type_kind(&result), "enum");

        // Also test with visited set tracking (visiting.remove branch)
        let mut visiting2 = HashSet::new();
        visiting2.insert("Status".to_string());
        let result2 = resolve_type_expr_with_lookup(
            &ParsedTypeExpr::Custom("Status".to_string()),
            &type_map,
            &mut resolved,
            &mut visiting2,
            &span,
        )
        .expect("enum type should resolve even when in visiting set");
        assert_eq!(script_type_kind(&result2), "enum");
    }
}
