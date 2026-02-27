use super::*;

#[test]
fn replace_defs_global_symbol_skips_non_boundary_matches() {
    let source = "shared.hp2 = 1; xshared.hp = 2; shared.hp = 3;";
    let rewritten = replace_defs_global_symbol(source, "shared.hp", "__sl_defs_ns_shared[\"hp\"]");
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
    let rewritten = replace_defs_global_symbol(source, "shared.hp", "__sl_defs_ns_shared[\"hp\"]");
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
