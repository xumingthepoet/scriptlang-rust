use std::cmp::Reverse;
use std::collections::BTreeMap;

use regex::Regex;
use rhai::{Array, Dynamic, ImmutableString, Map, FLOAT, INT};
use sl_core::{ScriptLangError, SlValue};

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

pub(crate) fn defs_namespace_symbol(namespace: &str) -> String {
    format!("__sl_defs_ns_{}", rhai_function_symbol(namespace))
}

pub(crate) fn rewrite_function_calls(
    source: &str,
    function_symbol_map: &BTreeMap<String, String>,
) -> Result<String, ScriptLangError> {
    if function_symbol_map.is_empty() {
        return Ok(source.to_string());
    }

    let mut names = function_symbol_map.iter().collect::<Vec<_>>();
    names.sort_by_key(|(name, _)| Reverse(name.len()));

    let mut rewritten = source.to_string();
    for (public_name, symbol_name) in names {
        if public_name == symbol_name {
            continue;
        }
        let pattern = Regex::new(&format!(
            r"(^|[^A-Za-z0-9_]){}\s*\(",
            regex::escape(public_name)
        ))
        .expect("escaped function name regex should compile");

        rewritten = pattern
            .replace_all(&rewritten, |captures: &regex::Captures<'_>| {
                format!("{}{}(", &captures[1], symbol_name)
            })
            .to_string();
    }

    Ok(rewritten)
}

pub(crate) fn rewrite_defs_global_qualified_access(
    source: &str,
    qualified_to_expr: &BTreeMap<String, String>,
) -> Result<String, ScriptLangError> {
    if qualified_to_expr.is_empty() {
        return Ok(source.to_string());
    }

    let mut names = qualified_to_expr.iter().collect::<Vec<_>>();
    names.sort_by_key(|(name, _)| Reverse(name.len()));

    let mut rewritten = source.to_string();
    for (qualified_name, target_expr) in names {
        rewritten = replace_defs_global_symbol(&rewritten, qualified_name, target_expr);
    }

    Ok(rewritten)
}

pub(crate) fn replace_defs_global_symbol(source: &str, symbol: &str, replacement: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut cursor = 0usize;

    while let Some(found) = source[cursor..].find(symbol) {
        let start = cursor + found;
        let end = start + symbol.len();

        let left = source[..start].chars().next_back();
        let right = source[end..].chars().next();
        if is_defs_global_left_boundary(left) && is_defs_global_right_boundary(right) {
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

pub(crate) fn is_defs_identifier_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '$' || ch == '_'
}

pub(crate) fn is_defs_global_left_boundary(left: Option<char>) -> bool {
    match left {
        None => true,
        Some(ch) => !is_defs_identifier_char(ch) && ch != '.',
    }
}

pub(crate) fn is_defs_global_right_boundary(right: Option<char>) -> bool {
    match right {
        None => true,
        Some(ch) => !is_defs_identifier_char(ch) && ch != ':',
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

pub(crate) fn slvalue_to_dynamic(value: &SlValue) -> Result<Dynamic, ScriptLangError> {
    match value {
        SlValue::Bool(value) => Ok(Dynamic::from_bool(*value)),
        SlValue::Number(value) => Ok(Dynamic::from_float(*value as FLOAT)),
        SlValue::String(value) => Ok(Dynamic::from(value.clone())),
        SlValue::Array(values) => {
            let mut array = Array::new();
            for value in values {
                array.push(slvalue_to_dynamic(value)?);
            }
            Ok(Dynamic::from_array(array))
        }
        SlValue::Map(values) => {
            let mut map = Map::new();
            for (key, value) in values {
                map.insert(key.clone().into(), slvalue_to_dynamic(value)?);
            }
            Ok(Dynamic::from_map(map))
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
    fn replace_defs_global_symbol_skips_non_boundary_matches() {
        let source = "shared.hp2 = 1; xshared.hp = 2; shared.hp = 3;";
        let rewritten =
            replace_defs_global_symbol(source, "shared.hp", "__sl_defs_ns_shared[\"hp\"]");
        assert!(rewritten.contains("shared.hp2 = 1;"));
        assert!(rewritten.contains("xshared.hp = 2;"));
        assert!(rewritten.contains("__sl_defs_ns_shared[\"hp\"] = 3;"));
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
        assert_eq!(defs_namespace_symbol("shared.ns"), "__sl_defs_ns_shared_ns");
        assert!(is_defs_identifier_char('a'));
        assert!(is_defs_identifier_char('$'));
        assert!(!is_defs_identifier_char('.'));
        assert!(is_defs_global_left_boundary(None));
        assert!(is_defs_global_left_boundary(Some(' ')));
        assert!(!is_defs_global_left_boundary(Some('a')));
        assert!(is_defs_global_right_boundary(None));
        assert!(is_defs_global_right_boundary(Some(' ')));
        assert!(!is_defs_global_right_boundary(Some(':')));

        let rewritten = rewrite_function_calls(
            "x = shared.add(1); y = add(2);",
            &BTreeMap::from([
                ("shared.add".to_string(), "__fn_shared_add".to_string()),
                ("add".to_string(), "__fn_add".to_string()),
            ]),
        )
        .expect("rewrite function calls");
        assert!(rewritten.contains("__fn_shared_add("));
        assert!(rewritten.contains("__fn_add("));

        let rewritten_defs = rewrite_defs_global_qualified_access(
            "x = shared.hp + other.hp;",
            &BTreeMap::from([(
                "shared.hp".to_string(),
                "__sl_defs_ns_shared.hp".to_string(),
            )]),
        )
        .expect("rewrite defs globals");
        assert!(rewritten_defs.contains("__sl_defs_ns_shared.hp"));
        assert!(rewritten_defs.contains("other.hp"));

        assert_eq!(slvalue_to_text(&SlValue::Bool(true)), "true");
        assert!(slvalue_to_text(&SlValue::Array(vec![SlValue::Number(1.0)])).contains("Array"));

        let dynamic_map = slvalue_to_dynamic(&SlValue::Map(BTreeMap::from([(
            "k".to_string(),
            SlValue::Array(vec![SlValue::Bool(false)]),
        )])))
        .expect("to dynamic");
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
    fn replace_defs_global_symbol_no_match_covered() {
        // 行93: 当 source 中没有找到任何匹配项时
        let source = "x = 1; y = 2;";
        let rewritten =
            replace_defs_global_symbol(source, "shared.hp", "__sl_defs_ns_shared[\"hp\"]");
        assert_eq!(rewritten, source);
    }

    #[test]
    fn slvalue_to_dynamic_array_covered() {
        // 行140, 142: slvalue_to_dynamic 处理 Array
        let arr = SlValue::Array(vec![SlValue::Number(1.0), SlValue::Number(2.0)]);
        let dynamic = slvalue_to_dynamic(&arr).expect("array to dynamic");
        assert!(dynamic.is::<Array>());
    }

    #[test]
    fn slvalue_to_dynamic_map_covered() {
        // 行147, 150: slvalue_to_dynamic 处理 Map
        let map = SlValue::Map(BTreeMap::from([
            ("a".to_string(), SlValue::Number(1.0)),
            ("b".to_string(), SlValue::Number(2.0)),
        ]));
        let dynamic = slvalue_to_dynamic(&map).expect("map to dynamic");
        assert!(dynamic.is::<Map>());
    }

    #[test]
    fn dynamic_to_slvalue_array_recursive_covered() {
        // 行171: dynamic_to_slvalue 处理 Array 中的递归
        let arr = Array::from([Dynamic::from_array(Array::from([Dynamic::from_bool(true)]))]);
        let dynamic = Dynamic::from_array(arr);
        let result = dynamic_to_slvalue(dynamic).expect("array recursive");
        assert!(matches!(result, SlValue::Array(vec) if vec.len() == 1));
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
