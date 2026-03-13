use std::cmp::Reverse;
use std::collections::BTreeMap;

use regex::Regex;

use crate::ScriptLangError;

pub fn rhai_function_symbol(name: &str) -> String {
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

pub fn module_namespace_symbol(namespace: &str) -> String {
    format!("__sl_module_ns_{}", rhai_function_symbol(namespace))
}

pub fn rewrite_function_calls(
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

pub fn rewrite_module_global_qualified_access(
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

pub fn replace_module_global_symbol(source: &str, symbol: &str, replacement: &str) -> String {
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
pub enum RhaiInputMode {
    AttributeExpr,
    TextInterpolationExpr,
    CodeBlock,
}

pub fn preprocess_scriptlang_rhai_input(
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
                RhaiInputMode::TextInterpolationExpr => {
                    return Err(preprocess_error(
                        "RHAI_PREPROCESS_FORBIDDEN_SINGLE_QUOTE",
                        context,
                        "single-quoted strings are forbidden in text interpolation expressions",
                        "Use double-quoted strings like \"text\" inside ${...}.",
                    ));
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
                RhaiInputMode::TextInterpolationExpr | RhaiInputMode::CodeBlock => {
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
            '@' if is_script_literal_left_boundary(chars.get(index.wrapping_sub(1)).copied()) => {
                if let Some((script_name, next_index)) = parse_script_literal_name(&chars, index) {
                    out.push('"');
                    out.push('@');
                    out.push_str(&script_name);
                    out.push('"');
                    index = next_index;
                    continue;
                }
                out.push('@');
                index += 1;
            }
            '*' if is_function_literal_start(&chars, index) => {
                if let Some((function_name, next_index)) =
                    parse_function_literal_name(&chars, index)
                {
                    let mut lookahead = next_index;
                    while lookahead < chars.len() && chars[lookahead].is_whitespace() {
                        lookahead += 1;
                    }
                    if chars.get(lookahead) == Some(&'(') {
                        return Err(preprocess_error(
                            "RHAI_PREPROCESS_FUNCTION_LITERAL_CALL_FORBIDDEN",
                            context,
                            "function literal cannot be called directly",
                            "Use method(...) or module.method(...), not *module.method(...).",
                        ));
                    }
                    out.push('"');
                    out.push('*');
                    out.push_str(&function_name);
                    out.push('"');
                    index = next_index;
                    continue;
                }
                out.push('*');
                index += 1;
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

fn is_script_literal_left_boundary(ch: Option<char>) -> bool {
    match ch {
        None => true,
        Some(value) => !value.is_ascii_alphanumeric() && value != '_' && value != '.',
    }
}

fn is_function_literal_start(chars: &[char], index: usize) -> bool {
    if chars.get(index) != Some(&'*') || index + 1 >= chars.len() {
        return false;
    }
    let mut left = index;
    while left > 0 && chars[left - 1].is_whitespace() {
        left -= 1;
    }
    if left == 0 {
        return true;
    }
    let prev = chars[left - 1];
    !prev.is_ascii_alphanumeric() && prev != '_' && prev != '.' && prev != ')' && prev != ']'
}

fn parse_script_literal_name(chars: &[char], at_index: usize) -> Option<(String, usize)> {
    let mut index = at_index + 1;
    let mut name = String::new();

    let first = *chars.get(index)?;
    if !first.is_ascii_alphabetic() && first != '_' {
        return None;
    }
    name.push(first);
    index += 1;

    while let Some(ch) = chars.get(index).copied() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            name.push(ch);
            index += 1;
            continue;
        }
        if ch == '.' {
            let next = chars.get(index + 1).copied()?;
            if !next.is_ascii_alphabetic() && next != '_' {
                return None;
            }
            name.push(ch);
            index += 1;
            continue;
        }
        break;
    }

    Some((name, index))
}

fn parse_function_literal_name(chars: &[char], at_index: usize) -> Option<(String, usize)> {
    let mut index = at_index + 1;
    let mut name = String::new();

    let first = *chars.get(index)?;
    if !first.is_ascii_alphabetic() && first != '_' {
        return None;
    }
    name.push(first);
    index += 1;

    while let Some(ch) = chars.get(index).copied() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            name.push(ch);
            index += 1;
            continue;
        }
        if ch == '.' {
            let next = chars.get(index + 1).copied()?;
            if !next.is_ascii_alphabetic() && next != '_' {
                return None;
            }
            name.push(ch);
            index += 1;
            continue;
        }
        break;
    }
    Some((name, index))
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
                        "Close the string and escape inner apostrophes as \\\\'",
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
        "Close the string with ' and escape inner apostrophes as \\\\'",
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

pub fn is_module_identifier_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '$' || ch == '_'
}

pub fn is_module_global_left_boundary(left: Option<char>) -> bool {
    match left {
        None => true,
        Some(ch) => !is_module_identifier_char(ch) && ch != '.',
    }
}

pub fn is_module_global_right_boundary(right: Option<char>) -> bool {
    match right {
        None => true,
        Some(ch) => !is_module_identifier_char(ch) && ch != ':',
    }
}

#[cfg(test)]
mod rhai_tests {
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
    fn helper_functions_cover_remaining_paths() {
        assert_eq!(rhai_function_symbol("a.b-c"), "a_b_c");
        assert_eq!(
            module_namespace_symbol("shared.ns"),
            "__sl_module_ns_shared_ns"
        );
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

        let rewritten_ws = rewrite_function_calls(
            "x = add (1);",
            &BTreeMap::from([("add".to_string(), "__fn_add".to_string())]),
        );
        assert!(rewritten_ws.contains("call(__fn_add, 1)"));

        let rewritten_ws2 = rewrite_function_calls(
            "x = add( 1);",
            &BTreeMap::from([("add".to_string(), "__fn_add".to_string())]),
        );
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
    }

    #[test]
    fn preprocess_rewrites_keywords_and_single_quotes() {
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
    fn preprocess_text_interpolation_uses_double_quote_rules() {
        let rewritten = preprocess_scriptlang_rhai_input(
            r#"name == "Rin" AND hp LTE 1"#,
            "text interpolation",
            RhaiInputMode::TextInterpolationExpr,
        )
        .expect("text interpolation should allow double quotes");
        assert_eq!(rewritten, r#"name == "Rin" && hp <= 1"#);

        let single_quote = preprocess_scriptlang_rhai_input(
            "name == 'Rin'",
            "text interpolation",
            RhaiInputMode::TextInterpolationExpr,
        )
        .expect_err("text interpolation should reject single quotes");
        assert_eq!(single_quote.code, "RHAI_PREPROCESS_FORBIDDEN_SINGLE_QUOTE");
    }

    #[test]
    fn preprocess_script_literals_handles_hyphen_and_invalid_shapes() {
        let rewritten = preprocess_scriptlang_rhai_input(
            "dst = @battle-loop.main;",
            "code",
            RhaiInputMode::CodeBlock,
        )
        .expect("hyphenated script literal should be accepted");
        assert_eq!(rewritten, "dst = \"@battle-loop.main\";");

        let dotted =
            preprocess_scriptlang_rhai_input("dst = @main.next;", "code", RhaiInputMode::CodeBlock)
                .expect("qualified script literal should be accepted");
        assert_eq!(dotted, "dst = \"@main.next\";");

        let invalid = preprocess_scriptlang_rhai_input("@a.", "code", RhaiInputMode::CodeBlock)
            .expect("invalid literal shape should keep raw token");
        assert_eq!(invalid, "@a.");

        let invalid_first =
            preprocess_scriptlang_rhai_input("@1next", "code", RhaiInputMode::CodeBlock)
                .expect("invalid first char should keep raw token");
        assert_eq!(invalid_first, "@1next");

        let invalid_segment =
            preprocess_scriptlang_rhai_input("@main.1next", "code", RhaiInputMode::CodeBlock)
                .expect("invalid segment head should keep raw token");
        assert_eq!(invalid_segment, "@main.1next");

        let trailing_at = preprocess_scriptlang_rhai_input("@", "code", RhaiInputMode::CodeBlock)
            .expect("trailing @ should keep raw token");
        assert_eq!(trailing_at, "@");

        let prefixed =
            preprocess_scriptlang_rhai_input("obj.@next", "code", RhaiInputMode::CodeBlock)
                .expect("non-boundary @ should keep raw token");
        assert_eq!(prefixed, "obj.@next");
    }

    #[test]
    fn preprocess_function_literals_support_assignment_and_reject_direct_call() {
        let rewritten = preprocess_scriptlang_rhai_input(
            "fn_ref = *main.add;",
            "code",
            RhaiInputMode::CodeBlock,
        )
        .expect("function literal assignment should be accepted");
        assert_eq!(rewritten, "fn_ref = \"*main.add\";");

        let short =
            preprocess_scriptlang_rhai_input("fn_ref = *add;", "code", RhaiInputMode::CodeBlock)
                .expect("short function literal should be accepted");
        assert_eq!(short, "fn_ref = \"*add\";");

        let multiply =
            preprocess_scriptlang_rhai_input("v = x * y;", "code", RhaiInputMode::CodeBlock)
                .expect("multiply should not become function literal");
        assert_eq!(multiply, "v = x * y;");

        let direct_call =
            preprocess_scriptlang_rhai_input("*main.add(1)", "code", RhaiInputMode::CodeBlock)
                .expect_err("direct function literal call should fail");
        assert_eq!(
            direct_call.code,
            "RHAI_PREPROCESS_FUNCTION_LITERAL_CALL_FORBIDDEN"
        );
        let spaced_call =
            preprocess_scriptlang_rhai_input("*main.add   (1)", "code", RhaiInputMode::CodeBlock)
                .expect_err("spaced direct function literal call should fail");
        assert_eq!(
            spaced_call.code,
            "RHAI_PREPROCESS_FUNCTION_LITERAL_CALL_FORBIDDEN"
        );

        let invalid_shape =
            preprocess_scriptlang_rhai_input("*main.", "code", RhaiInputMode::CodeBlock)
                .expect("invalid shape should keep raw token");
        assert_eq!(invalid_shape, "*main.");
        let invalid_first =
            preprocess_scriptlang_rhai_input("*1main.add", "code", RhaiInputMode::CodeBlock)
                .expect("invalid first char should stay raw");
        assert_eq!(invalid_first, "*1main.add");
        let invalid_segment =
            preprocess_scriptlang_rhai_input("*main.1add", "code", RhaiInputMode::CodeBlock)
                .expect("invalid segment head should stay raw");
        assert_eq!(invalid_segment, "*main.1add");
    }

    #[test]
    fn replace_module_global_symbol_no_match_covered() {
        let source = "x = 1; y = 2;";
        let rewritten =
            replace_module_global_symbol(source, "shared.hp", "__sl_module_ns_shared[\"hp\"]");
        assert_eq!(rewritten, source);
    }

    #[test]
    fn is_function_literal_start_covers_early_return() {
        // chars.get(index) != Some(&'*') - not a star
        let chars: Vec<char> = "abc".chars().collect();
        assert!(!is_function_literal_start(&chars, 0));

        // index + 1 >= chars.len() - star at end of input
        let chars: Vec<char> = "*".chars().collect();
        assert!(!is_function_literal_start(&chars, 0));

        // Valid case - star followed by identifier
        let chars: Vec<char> = "*foo".chars().collect();
        assert!(is_function_literal_start(&chars, 0));

        // Valid case with preceding whitespace
        let chars: Vec<char> = " *foo".chars().collect();
        assert!(is_function_literal_start(&chars, 1));
    }

    #[test]
    fn parse_function_literal_name_covers_error_path() {
        // chars.get(index) returns None - empty input after at_index
        let chars: Vec<char> = "".chars().collect();
        assert_eq!(parse_function_literal_name(&chars, 0), None);

        // Valid case - star followed by identifier
        let chars: Vec<char> = "*foo".chars().collect();
        assert_eq!(
            parse_function_literal_name(&chars, 0),
            Some(("foo".to_string(), 4))
        );

        // Valid case with dots
        let chars: Vec<char> = "*foo.bar".chars().collect();
        assert_eq!(
            parse_function_literal_name(&chars, 0),
            Some(("foo.bar".to_string(), 8))
        );
    }
}
