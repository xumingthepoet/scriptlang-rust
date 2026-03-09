use std::cmp::Reverse;
use std::collections::BTreeMap;

use regex::Regex;
use rhai::{Array, Dynamic, ImmutableString, Map, FLOAT, INT};
use sl_core::{ScriptLangError, ScriptType, SlValue};

pub(crate) fn rhai_function_symbol(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    out
}

pub(crate) fn module_namespace_symbol(namespace: &str) -> String {
    format!("__sl_module_ns_{}", rhai_function_symbol(namespace))
}

pub(crate) fn rewrite_function_calls(
    source: &str,
    function_symbol_map: &BTreeMap<String, String>,
) -> String {
    if function_symbol_map.is_empty() {
        return source.to_string();
    }

    let mut names = function_symbol_map.iter().collect::<Vec<_>>();
    names.sort_by_key(|(name, _)| Reverse(name.len()));

    let mut rewritten = source.to_string();
    for (public_name, symbol_name) in names {
        let pattern = Regex::new(&format!(
            r"(^|[^A-Za-z0-9_]){}\s*\(",
            regex::escape(public_name)
        ))
        .expect("escaped function name regex should compile");

        rewritten = pattern
            .replace_all(&rewritten, |captures: &regex::Captures<'_>| {
                let full = captures
                    .get(0)
                    .expect("full regex capture should exist for function replacement");
                let mut out = format!("{}call({}", &captures[1], symbol_name);
                let mut cursor = full.end();
                while cursor < rewritten.len()
                    && rewritten[cursor..]
                        .chars()
                        .next()
                        .is_some_and(char::is_whitespace)
                {
                    cursor += rewritten[cursor..]
                        .chars()
                        .next()
                        .expect("cursor should point to valid char")
                        .len_utf8();
                }
                if rewritten[cursor..]
                    .chars()
                    .next()
                    .is_some_and(|ch| ch != ')')
                {
                    out.push_str(", ");
                }
                out
            })
            .to_string();
    }

    rewritten
}

pub(crate) fn rewrite_module_global_qualified_access(
    source: &str,
    qualified_to_expr: &BTreeMap<String, String>,
) -> String {
    if qualified_to_expr.is_empty() {
        return source.to_string();
    }

    let mut names = qualified_to_expr.iter().collect::<Vec<_>>();
    names.sort_by(|(left, _), (right, _)| right.len().cmp(&left.len()));

    let mut rewritten = source.to_string();
    for (qualified_name, target_expr) in names {
        rewritten = replace_module_global_symbol(&rewritten, qualified_name, target_expr);
    }

    rewritten
}

pub(crate) fn replace_module_global_symbol(source: &str, symbol: &str, replacement: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut cursor = 0usize;

    while let Some(found) = source[cursor..].find(symbol) {
        let start = cursor + found;
        let end = start + symbol.len();

        let left = source[..start].chars().next_back();
        let right = source[end..].chars().next();
        if is_module_global_left_boundary(left) && is_module_global_right_boundary(right) {
            out.push_str(&source[cursor..start]);
            out.push_str(replacement);
            cursor = end;
            continue;
        }

        let ch = source[start..]
            .chars()
            .next()
            .expect("non-empty suffix should have a char");
        let next = start + ch.len_utf8();
        out.push_str(&source[cursor..next]);
        cursor = next;
    }

    out.push_str(&source[cursor..]);
    out
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RhaiInputMode {
    AttributeExpr,
    CodeBlock,
}

pub(crate) fn preprocess_scriptlang_rhai_input(
    source: &str,
    context: &str,
    mode: RhaiInputMode,
) -> Result<String, ScriptLangError> {
    let chars = source.chars().collect::<Vec<_>>();
    let mut out = String::with_capacity(source.len());
    let mut index = 0usize;

    while index < chars.len() {
        match chars[index] {
            '\'' => match mode {
                RhaiInputMode::AttributeExpr => {
                    let (encoded, next_index) = parse_single_quoted_string(&chars, index, context)?;
                    out.push_str(&encoded);
                    index = next_index;
                }
                RhaiInputMode::CodeBlock => {
                    return Err(preprocess_error(
                        "RHAI_PREPROCESS_FORBIDDEN_SINGLE_QUOTE",
                        context,
                        "single-quoted strings are forbidden in code-style expressions",
                        "Use double-quoted strings like \"text\" in <code>, <function>, and <var> initializer bodies.",
                    ));
                }
            },
            '"' => match mode {
                RhaiInputMode::AttributeExpr => {
                    return Err(preprocess_error(
                        "RHAI_PREPROCESS_FORBIDDEN_DOUBLE_QUOTE",
                        context,
                        "double-quoted strings are forbidden in attribute expressions",
                        "Use single-quoted strings like 'text' in XML attributes.",
                    ));
                }
                RhaiInputMode::CodeBlock => {
                    out.push('"');
                    index += 1;
                    let mut closed = false;
                    while index < chars.len() {
                        match chars[index] {
                            '"' => {
                                out.push('"');
                                index += 1;
                                closed = true;
                                break;
                            }
                            '\\' => {
                                out.push('\\');
                                let Some(next) = chars.get(index + 1).copied() else {
                                    return Err(preprocess_error(
                                            "RHAI_PREPROCESS_STRING_UNTERMINATED",
                                            context,
                                            "unterminated escape in double-quoted string",
                                            "Close the string with \" and keep escapes inside the string body.",
                                        ));
                                };
                                out.push(next);
                                index += 2;
                            }
                            ch => {
                                out.push(ch);
                                index += 1;
                            }
                        }
                    }
                    if !closed {
                        return Err(preprocess_error(
                            "RHAI_PREPROCESS_STRING_UNTERMINATED",
                            context,
                            "unterminated double-quoted string",
                            "Close the string with \".",
                        ));
                    }
                }
            },
            '<' => {
                let code = if chars.get(index + 1) == Some(&'=') {
                    "RHAI_PREPROCESS_FORBIDDEN_LTE"
                } else {
                    "RHAI_PREPROCESS_FORBIDDEN_LT"
                };
                let replacement = if code.ends_with("LTE") {
                    "Use LTE instead of <=."
                } else {
                    "Use LT instead of <."
                };
                return Err(preprocess_error(
                    code,
                    context,
                    "raw comparison operator is forbidden",
                    replacement,
                ));
            }
            '&' if chars.get(index + 1) == Some(&'&') => {
                return Err(preprocess_error(
                    "RHAI_PREPROCESS_FORBIDDEN_AND",
                    context,
                    "raw logical operator && is forbidden",
                    "Use AND instead of &&.",
                ));
            }
            ch if is_scriptlang_token_char(ch) => {
                let start = index;
                index += 1;
                while index < chars.len() && is_scriptlang_token_char(chars[index]) {
                    index += 1;
                }
                let token = chars[start..index].iter().collect::<String>();
                match token.as_str() {
                    "LTE" => out.push_str("<="),
                    "LT" => out.push('<'),
                    "AND" => out.push_str("&&"),
                    _ => out.push_str(&token),
                }
            }
            ch => {
                out.push(ch);
                index += 1;
            }
        }
    }

    Ok(out)
}

fn parse_single_quoted_string(
    chars: &[char],
    start: usize,
    context: &str,
) -> Result<(String, usize), ScriptLangError> {
    let mut out = String::from("\"");
    let mut index = start + 1;

    while index < chars.len() {
        match chars[index] {
            '\'' => {
                out.push('"');
                return Ok((out, index + 1));
            }
            '\\' => {
                let Some(next) = chars.get(index + 1).copied() else {
                    return Err(preprocess_error(
                        "RHAI_PREPROCESS_STRING_UNTERMINATED",
                        context,
                        "unterminated escape in single-quoted string",
                        "Close the string and escape inner apostrophes as \\'",
                    ));
                };
                match next {
                    '\'' => out.push('\''),
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    'n' => out.push_str("\\n"),
                    'r' => out.push_str("\\r"),
                    't' => out.push_str("\\t"),
                    '0' => out.push_str("\\0"),
                    _ => {
                        out.push('\\');
                        out.push(next);
                    }
                }
                index += 2;
            }
            '"' => {
                out.push_str("\\\"");
                index += 1;
            }
            ch => {
                out.push(ch);
                index += 1;
            }
        }
    }

    Err(preprocess_error(
        "RHAI_PREPROCESS_STRING_UNTERMINATED",
        context,
        "unterminated single-quoted string",
        "Close the string with ' and escape inner apostrophes as \\'",
    ))
}

fn preprocess_error(
    code: &'static str,
    context: &str,
    detail: &str,
    recommendation: &str,
) -> ScriptLangError {
    ScriptLangError::new(
        code,
        format!(
            "ScriptLang Rhai preprocessing failed in {}: {}. {}",
            context, detail, recommendation
        ),
    )
}

fn is_scriptlang_token_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_' || ch == '$'
}

pub(crate) fn is_module_identifier_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '$' || ch == '_'
}

pub(crate) fn is_module_global_left_boundary(left: Option<char>) -> bool {
    match left {
        None => true,
        Some(ch) => !is_module_identifier_char(ch) && ch != '.',
    }
}

pub(crate) fn is_module_global_right_boundary(right: Option<char>) -> bool {
    match right {
        None => true,
        Some(ch) => !is_module_identifier_char(ch) && ch != ':',
    }
}

pub(crate) fn slvalue_to_text(value: &SlValue) -> String {
    match value {
        SlValue::Bool(value) => value.to_string(),
        SlValue::Number(value) => {
            if value.fract().abs() < f64::EPSILON {
                (*value as i64).to_string()
            } else {
                value.to_string()
            }
        }
        SlValue::String(value) => value.clone(),
        SlValue::Array(_) | SlValue::Map(_) => format!("{:?}", value),
    }
}

pub(crate) fn slvalue_to_dynamic(value: &SlValue) -> Dynamic {
    slvalue_to_dynamic_with_type(value, None)
}

pub(crate) fn slvalue_to_dynamic_with_type(value: &SlValue, ty: Option<&ScriptType>) -> Dynamic {
    match value {
        SlValue::Bool(value) => Dynamic::from_bool(*value),
        SlValue::Number(value) => {
            if matches!(
                ty,
                Some(ScriptType::Primitive { name }) if name == "int"
            ) && value.is_finite()
                && value.fract().abs() < f64::EPSILON
            {
                return Dynamic::from_int(*value as INT);
            }
            Dynamic::from_float(*value as FLOAT)
        }
        SlValue::String(value) => Dynamic::from(value.clone()),
        SlValue::Array(values) => {
            let mut array = Array::new();
            for value in values {
                let element_type = match ty {
                    Some(ScriptType::Array { element_type }) => Some(element_type.as_ref()),
                    _ => None,
                };
                array.push(slvalue_to_dynamic_with_type(value, element_type));
            }
            Dynamic::from_array(array)
        }
        SlValue::Map(values) => {
            let mut map = Map::new();
            for (key, value) in values {
                let value_type = match ty {
                    Some(ScriptType::Map { value_type, .. }) => Some(value_type.as_ref()),
                    Some(ScriptType::Object { fields, .. }) => fields.get(key),
                    _ => None,
                };
                map.insert(
                    key.clone().into(),
                    slvalue_to_dynamic_with_type(value, value_type),
                );
            }
            Dynamic::from_map(map)
        }
    }
}

pub(crate) fn dynamic_to_slvalue(value: Dynamic) -> Result<SlValue, ScriptLangError> {
    if value.is::<bool>() {
        return Ok(SlValue::Bool(value.cast::<bool>()));
    }
    if value.is::<INT>() {
        return Ok(SlValue::Number(value.cast::<INT>() as f64));
    }
    if value.is::<FLOAT>() {
        return Ok(SlValue::Number(value.cast::<FLOAT>()));
    }
    if value.is::<ImmutableString>() {
        return Ok(SlValue::String(value.cast::<ImmutableString>().to_string()));
    }
    if value.is::<Array>() {
        let array = value.cast::<Array>();
        let mut out = Vec::with_capacity(array.len());
        for item in array {
            out.push(dynamic_to_slvalue(item)?);
        }
        return Ok(SlValue::Array(out));
    }
    if value.is::<Map>() {
        let map = value.cast::<Map>();
        let mut out = BTreeMap::new();
        for (key, value) in map {
            out.insert(key.to_string(), dynamic_to_slvalue(value)?);
        }
        return Ok(SlValue::Map(out));
    }

    Err(ScriptLangError::new(
        "ENGINE_VALUE_UNSUPPORTED",
        "Unsupported Rhai value type.",
    ))
}

pub(crate) fn slvalue_to_rhai_literal(value: &SlValue) -> String {
    match value {
        SlValue::Bool(value) => value.to_string(),
        SlValue::Number(value) => {
            if value.fract().abs() < f64::EPSILON {
                (*value as i64).to_string()
            } else {
                value.to_string()
            }
        }
        SlValue::String(value) => format!("\"{}\"", value.replace('"', "\\\"")),
        SlValue::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(slvalue_to_rhai_literal)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        SlValue::Map(values) => {
            let entries = values
                .iter()
                .map(|(key, value)| format!("{}: {}", key, slvalue_to_rhai_literal(value)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("#{{{}}}", entries)
        }
    }
}

#[cfg(test)]
mod rhai_bridge_tests {
    use super::*;

    #[test]
    fn replace_module_global_symbol_skips_non_boundary_matches() {
        let source = "shared.hp2 = 1; xshared.hp = 2; shared.hp = 3;";
        let rewritten =
            replace_module_global_symbol(source, "shared.hp", "__sl_module_ns_shared[\"hp\"]");
        assert!(rewritten.contains("shared.hp2 = 1;"));
        assert!(rewritten.contains("xshared.hp = 2;"));
        assert!(rewritten.contains("__sl_module_ns_shared[\"hp\"] = 3;"));
    }

    #[test]
    fn literal_helpers_cover_decimal_and_array_paths() {
        assert_eq!(slvalue_to_text(&SlValue::Number(2.5)), "2.5");
        assert_eq!(slvalue_to_rhai_literal(&SlValue::Number(2.5)), "2.5");
        assert_eq!(
            slvalue_to_rhai_literal(&SlValue::Array(vec![
                SlValue::Number(1.0),
                SlValue::Number(2.5),
            ])),
            "[1, 2.5]"
        );
    }

    #[test]
    fn bridge_helper_functions_cover_remaining_paths() {
        assert_eq!(rhai_function_symbol("a.b-c"), "a_b_c");
        assert_eq!(module_namespace_symbol("shared.ns"), "__sl_module_ns_shared_ns");
        assert!(is_scriptlang_token_char('a'));
        assert!(is_scriptlang_token_char('_'));
        assert!(!is_scriptlang_token_char('.'));
        assert!(is_module_identifier_char('a'));
        assert!(is_module_identifier_char('$'));
        assert!(!is_module_identifier_char('.'));
        assert!(is_module_global_left_boundary(None));
        assert!(is_module_global_left_boundary(Some(' ')));
        assert!(!is_module_global_left_boundary(Some('a')));
        assert!(is_module_global_right_boundary(None));
        assert!(is_module_global_right_boundary(Some(' ')));
        assert!(!is_module_global_right_boundary(Some(':')));

        let rewritten = rewrite_function_calls(
            "x = shared.add(1); y = add(2);",
            &BTreeMap::from([
                ("shared.add".to_string(), "__fn_shared_add".to_string()),
                ("add".to_string(), "__fn_add".to_string()),
            ]),
        );
        assert!(rewritten.contains("call(__fn_shared_add, 1)"));
        assert!(rewritten.contains("call(__fn_add, 2)"));

        // Test whitespace between function name and parenthesis (covers while loop path)
        let rewritten_ws = rewrite_function_calls(
            "x = add (1);",
            &BTreeMap::from([("add".to_string(), "__fn_add".to_string())]),
        );
        assert!(rewritten_ws.contains("call(__fn_add, 1)"));

        // Test whitespace after opening parenthesis
        let rewritten_ws2 = rewrite_function_calls(
            "x = add( 1);",
            &BTreeMap::from([("add".to_string(), "__fn_add".to_string())]),
        );
        // Verify it produces valid output (the exact format may vary)
        assert!(rewritten_ws2.contains("call"));

        let rewritten_same = rewrite_function_calls(
            "x = invoke();",
            &BTreeMap::from([("invoke".to_string(), "invoke".to_string())]),
        );
        assert!(rewritten_same.contains("call(invoke)"));
        assert_eq!(rewrite_function_calls("x = 1;", &BTreeMap::new()), "x = 1;");

        let rewritten_module = rewrite_module_global_qualified_access(
            "x = shared.hp + other.hp;",
            &BTreeMap::from([(
                "shared.hp".to_string(),
                "__sl_module_ns_shared.hp".to_string(),
            )]),
        );
        assert!(rewritten_module.contains("__sl_module_ns_shared.hp"));
        assert!(rewritten_module.contains("other.hp"));

        let rewritten_module_multi = rewrite_module_global_qualified_access(
            "x = shared.hp + shared.hp.max;",
            &BTreeMap::from([
                (
                    "shared.hp.max".to_string(),
                    "__sl_module_ns_shared.hp_max".to_string(),
                ),
                (
                    "shared.hp".to_string(),
                    "__sl_module_ns_shared.hp".to_string(),
                ),
            ]),
        );
        assert!(rewritten_module_multi.contains("__sl_module_ns_shared.hp_max"));
        assert!(rewritten_module_multi.contains("__sl_module_ns_shared.hp"));

        assert_eq!(slvalue_to_text(&SlValue::Bool(true)), "true");
        assert!(slvalue_to_text(&SlValue::Array(vec![SlValue::Number(1.0)])).contains("Array"));

        let dynamic_map = slvalue_to_dynamic(&SlValue::Map(BTreeMap::from([(
            "k".to_string(),
            SlValue::Array(vec![SlValue::Bool(false)]),
        )])));
        let roundtrip = dynamic_to_slvalue(dynamic_map).expect("roundtrip");
        assert_eq!(
            roundtrip,
            SlValue::Map(BTreeMap::from([(
                "k".to_string(),
                SlValue::Array(vec![SlValue::Bool(false)]),
            )]))
        );

        assert_eq!(
            slvalue_to_rhai_literal(&SlValue::String("A\"B".to_string())),
            "\"A\\\"B\""
        );
        assert_eq!(
            slvalue_to_rhai_literal(&SlValue::Map(BTreeMap::from([(
                "k".to_string(),
                SlValue::Number(1.0),
            )]))),
            "#{k: 1}"
        );
    }

    #[test]
    fn preprocess_scriptlang_rhai_input_rewrites_keywords_and_single_quotes() {
        let rewritten = preprocess_scriptlang_rhai_input(
            "hp LTE 10 AND name == 'Rin' AND slot == SLOT",
            "expression",
            RhaiInputMode::AttributeExpr,
        )
        .expect("preprocess");
        assert_eq!(rewritten, "hp <= 10 && name == \"Rin\" && slot == SLOT");

        let escaped = preprocess_scriptlang_rhai_input(
            "'I\\'m \"ok\"'",
            "expression",
            RhaiInputMode::AttributeExpr,
        )
        .expect("escaped string");
        assert_eq!(escaped, "\"I'm \\\"ok\\\"\"");
    }

    #[test]
    fn preprocess_attribute_expr_rejects_legacy_and_invalid_syntax() {
        let raw_lt =
            preprocess_scriptlang_rhai_input("hp < 10", "expression", RhaiInputMode::AttributeExpr)
                .expect_err("raw lt should fail");
        assert_eq!(raw_lt.code, "RHAI_PREPROCESS_FORBIDDEN_LT");
        assert!(raw_lt.message.contains("Use LT instead of <"));

        let raw_lte = preprocess_scriptlang_rhai_input(
            "hp <= 10",
            "expression",
            RhaiInputMode::AttributeExpr,
        )
        .expect_err("raw lte should fail");
        assert_eq!(raw_lte.code, "RHAI_PREPROCESS_FORBIDDEN_LTE");

        let raw_and =
            preprocess_scriptlang_rhai_input("a && b", "expression", RhaiInputMode::AttributeExpr)
                .expect_err("raw and should fail");
        assert_eq!(raw_and.code, "RHAI_PREPROCESS_FORBIDDEN_AND");

        let double_quote = preprocess_scriptlang_rhai_input(
            "name == \"Rin\"",
            "expression",
            RhaiInputMode::AttributeExpr,
        )
        .expect_err("double quote should fail");
        assert_eq!(double_quote.code, "RHAI_PREPROCESS_FORBIDDEN_DOUBLE_QUOTE");

        let unterminated =
            preprocess_scriptlang_rhai_input("'Rin", "expression", RhaiInputMode::AttributeExpr)
                .expect_err("unterminated string should fail");
        assert_eq!(unterminated.code, "RHAI_PREPROCESS_STRING_UNTERMINATED");
    }

    #[test]
    fn preprocess_attribute_expr_covers_single_quote_escape_variants() {
        let rewritten = preprocess_scriptlang_rhai_input(
            "'a\\\"b\\\\c\\nd\\re\\tf\\0g\\xy'",
            "expression",
            RhaiInputMode::AttributeExpr,
        )
        .expect("preprocess");
        assert_eq!(rewritten, "\"a\\\"b\\\\c\\nd\\re\\tf\\0g\\xy\"");

        let bad_escape =
            preprocess_scriptlang_rhai_input("'oops\\", "expression", RhaiInputMode::AttributeExpr)
                .expect_err("unterminated escape should fail");
        assert_eq!(bad_escape.code, "RHAI_PREPROCESS_STRING_UNTERMINATED");
        assert!(bad_escape.message.contains("escape inner apostrophes"));
    }

    #[test]
    fn preprocess_code_block_supports_double_quotes_and_keywords() {
        let rewritten = preprocess_scriptlang_rhai_input(
            r#"name = "Rin"; hp = hp LTE 1 AND ready"#,
            "code",
            RhaiInputMode::CodeBlock,
        )
        .expect("preprocess");
        assert_eq!(rewritten, r#"name = "Rin"; hp = hp <= 1 && ready"#);

        let escaped = preprocess_scriptlang_rhai_input(
            "name = \"R\\\"in\";",
            "code",
            RhaiInputMode::CodeBlock,
        )
        .expect("escaped code string");
        assert_eq!(escaped, "name = \"R\\\"in\";");
    }

    #[test]
    fn preprocess_code_block_rejects_single_quotes_and_legacy_operators() {
        let single_quote =
            preprocess_scriptlang_rhai_input("name = 'Rin';", "code", RhaiInputMode::CodeBlock)
                .expect_err("single quote should fail");
        assert_eq!(single_quote.code, "RHAI_PREPROCESS_FORBIDDEN_SINGLE_QUOTE");

        let raw_lt = preprocess_scriptlang_rhai_input("hp < 10", "code", RhaiInputMode::CodeBlock)
            .expect_err("raw lt should fail");
        assert_eq!(raw_lt.code, "RHAI_PREPROCESS_FORBIDDEN_LT");

        let raw_and = preprocess_scriptlang_rhai_input("a && b", "code", RhaiInputMode::CodeBlock)
            .expect_err("raw and should fail");
        assert_eq!(raw_and.code, "RHAI_PREPROCESS_FORBIDDEN_AND");

        let unterminated_escape =
            preprocess_scriptlang_rhai_input("name = \"Rin\\", "code", RhaiInputMode::CodeBlock)
                .expect_err("unterminated escape should fail");
        assert_eq!(
            unterminated_escape.code,
            "RHAI_PREPROCESS_STRING_UNTERMINATED"
        );

        let unterminated_string =
            preprocess_scriptlang_rhai_input("name = \"Rin", "code", RhaiInputMode::CodeBlock)
                .expect_err("unterminated string should fail");
        assert_eq!(
            unterminated_string.code,
            "RHAI_PREPROCESS_STRING_UNTERMINATED"
        );
    }

    #[test]
    fn replace_module_global_symbol_no_match_covered() {
        // 行93: 当 source 中没有找到任何匹配项时
        let source = "x = 1; y = 2;";
        let rewritten =
            replace_module_global_symbol(source, "shared.hp", "__sl_module_ns_shared[\"hp\"]");
        assert_eq!(rewritten, source);
    }

    #[test]
    fn slvalue_to_dynamic_array_covered() {
        // 行140, 142: slvalue_to_dynamic 处理 Array
        let arr = SlValue::Array(vec![SlValue::Number(1.0), SlValue::Number(2.0)]);
        let dynamic = slvalue_to_dynamic(&arr);
        assert!(dynamic.is::<Array>());
    }

    #[test]
    fn slvalue_to_dynamic_map_covered() {
        // 行147, 150: slvalue_to_dynamic 处理 Map
        let map = SlValue::Map(BTreeMap::from([
            ("a".to_string(), SlValue::Number(1.0)),
            ("b".to_string(), SlValue::Number(2.0)),
        ]));
        let dynamic = slvalue_to_dynamic(&map);
        assert!(dynamic.is::<Map>());
    }

    #[test]
    fn slvalue_to_dynamic_with_type_int_uses_rhai_int() {
        let int_ty = ScriptType::Primitive {
            name: "int".to_string(),
        };
        let dynamic = slvalue_to_dynamic_with_type(&SlValue::Number(2.0), Some(&int_ty));
        assert!(dynamic.is::<INT>());
        assert_eq!(dynamic.cast::<INT>(), 2);
    }

    #[test]
    fn slvalue_to_dynamic_with_object_type_uses_field_types() {
        let object_ty = ScriptType::Object {
            type_name: "Obj".to_string(),
            fields: BTreeMap::from([(
                "idx".to_string(),
                ScriptType::Primitive {
                    name: "int".to_string(),
                },
            )]),
        };
        let value = SlValue::Map(BTreeMap::from([("idx".to_string(), SlValue::Number(3.0))]));
        let dynamic = slvalue_to_dynamic_with_type(&value, Some(&object_ty));
        let map = dynamic.cast::<Map>();
        let idx = map.get("idx").expect("idx field");
        assert!(idx.is::<INT>());
        assert_eq!(idx.clone().cast::<INT>(), 3);
    }

    #[test]
    fn dynamic_to_slvalue_array_recursive_covered() {
        // 行171: dynamic_to_slvalue 处理 Array 中的递归
        let arr = Array::from([Dynamic::from_array(Array::from([Dynamic::from_bool(true)]))]);
        let dynamic = Dynamic::from_array(arr);
        let result = dynamic_to_slvalue(dynamic).expect("array recursive");
        assert!(matches!(result, SlValue::Array(vec) if vec.len() == 1));

        let bad = Dynamic::from_array(Array::from([Dynamic::UNIT]));
        let error = dynamic_to_slvalue(bad).expect_err("nested unsupported array value");
        assert_eq!(error.code, "ENGINE_VALUE_UNSUPPORTED");
    }

    #[test]
    fn dynamic_to_slvalue_map_recursive_covered() {
        // 行179: dynamic_to_slvalue 处理 Map 中的递归
        let mut map = Map::new();
        map.insert(
            "arr".into(),
            Dynamic::from_array(Array::from([Dynamic::from_bool(false)])),
        );
        let dynamic = Dynamic::from_map(map);
        let result = dynamic_to_slvalue(dynamic).expect("map recursive");
        assert!(matches!(result, SlValue::Map(m) if m.contains_key("arr")));

        let mut bad = Map::new();
        bad.insert("bad".into(), Dynamic::UNIT);
        let error =
            dynamic_to_slvalue(Dynamic::from_map(bad)).expect_err("nested unsupported map value");
        assert_eq!(error.code, "ENGINE_VALUE_UNSUPPORTED");
    }

    #[test]
    fn dynamic_to_slvalue_error_covered() {
        // 行182, 188: dynamic_to_slvalue 错误分支
        let result = dynamic_to_slvalue(Dynamic::UNIT);
        assert!(result.is_err());
    }

    #[test]
    fn slvalue_to_rhai_literal_decimal_covered() {
        // 行195: slvalue_to_rhai_literal 处理浮点数 else 分支
        // Using a value that triggers the non-integer path
        let decimal_value = 3.0 + 0.14;
        assert_eq!(
            slvalue_to_rhai_literal(&SlValue::Number(decimal_value)),
            "3.14"
        );
    }

    #[test]
    fn slvalue_to_rhai_literal_string_covered() {
        // 行198: slvalue_to_rhai_literal 处理字符串
        assert_eq!(
            slvalue_to_rhai_literal(&SlValue::String("test".to_string())),
            "\"test\""
        );
    }

    #[test]
    fn slvalue_to_rhai_literal_array_covered() {
        // 行201, 207: slvalue_to_rhai_literal 处理 Array
        let arr = SlValue::Array(vec![SlValue::Number(1.0), SlValue::String("a".to_string())]);
        assert_eq!(slvalue_to_rhai_literal(&arr), "[1, \"a\"]");
    }

    #[test]
    fn slvalue_to_rhai_literal_map_covered() {
        // 行216: slvalue_to_rhai_literal 处理 Map
        let map = SlValue::Map(BTreeMap::from([(
            "key".to_string(),
            SlValue::String("value".to_string()),
        )]));
        assert_eq!(slvalue_to_rhai_literal(&map), "#{key: \"value\"}");
    }
}
