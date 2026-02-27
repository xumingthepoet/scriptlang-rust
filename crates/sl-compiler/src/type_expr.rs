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

    let resolved_type = ScriptType::Object {
        type_name: lookup_name.clone(),
        fields,
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

#[cfg(test)]
pub(crate) fn parse_type_declaration_node(
    node: &XmlElementNode,
) -> Result<ParsedTypeDecl, ScriptLangError> {
    parse_type_declaration_node_with_namespace(node, "defs")
}

pub(crate) fn parse_type_declaration_node_with_namespace(
    node: &XmlElementNode,
    namespace: &str,
) -> Result<ParsedTypeDecl, ScriptLangError> {
    let name = get_required_non_empty_attr(node, "name")?;
    assert_name_not_reserved(&name, "type", node.location.clone())?;

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
        assert_name_not_reserved(&field_name, "type field", child.location.clone())?;
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
        fields,
        location: node.location.clone(),
    })
}

#[cfg(test)]
pub(crate) fn parse_function_declaration_node(
    node: &XmlElementNode,
) -> Result<ParsedFunctionDecl, ScriptLangError> {
    parse_function_declaration_node_with_namespace(node, "defs")
}

pub(crate) fn parse_function_declaration_node_with_namespace(
    node: &XmlElementNode,
    namespace: &str,
) -> Result<ParsedFunctionDecl, ScriptLangError> {
    let name = get_required_non_empty_attr(node, "name")?;
    assert_name_not_reserved(&name, "function", node.location.clone())?;

    let params = parse_function_args(node)?;
    let return_binding = parse_function_return(node)?;
    let code = parse_inline_required_no_element_children(node)?;

    let qualified_name = format!("{}.{}", namespace, name);
    Ok(ParsedFunctionDecl {
        name,
        qualified_name,
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

    #[test]
    fn type_resolution_helpers_cover_nested_array_and_map_paths() {
        let span = SourceSpan::synthetic();
        let mut resolved = BTreeMap::new();
        let mut visiting = HashSet::new();
        let type_map = BTreeMap::from([(
            "Obj".to_string(),
            ParsedTypeDecl {
                name: "Obj".to_string(),
                qualified_name: "Obj".to_string(),
                fields: vec![ParsedTypeFieldDecl {
                    name: "n".to_string(),
                    type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                    location: span.clone(),
                }],
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
        assert!(matches!(array, ScriptType::Array { .. }));

        let map = resolve_type_expr_with_lookup(
            &ParsedTypeExpr::Map(Box::new(ParsedTypeExpr::Custom("Obj".to_string()))),
            &type_map,
            &mut resolved,
            &mut visiting,
            &span,
        )
        .expect("map custom type should resolve");
        assert!(matches!(map, ScriptType::Map { .. }));

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

        let nested = parse_type_expr("#{int[]}", &span).expect("type should parse");
        assert!(matches!(nested, ParsedTypeExpr::Map(_)));

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
    }
}
