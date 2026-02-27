fn rhai_function_symbol(name: &str) -> String {
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

fn defs_namespace_symbol(namespace: &str) -> String {
    format!("__sl_defs_ns_{}", rhai_function_symbol(namespace))
}

fn rewrite_function_calls(
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

fn rewrite_defs_global_qualified_access(
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

fn replace_defs_global_symbol(source: &str, symbol: &str, replacement: &str) -> String {
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

fn is_defs_identifier_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '$' || ch == '_'
}

fn is_defs_global_left_boundary(left: Option<char>) -> bool {
    match left {
        None => true,
        Some(ch) => !is_defs_identifier_char(ch) && ch != '.',
    }
}

fn is_defs_global_right_boundary(right: Option<char>) -> bool {
    match right {
        None => true,
        Some(ch) => !is_defs_identifier_char(ch) && ch != ':',
    }
}

fn slvalue_to_text(value: &SlValue) -> String {
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

fn slvalue_to_dynamic(value: &SlValue) -> Result<Dynamic, ScriptLangError> {
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

fn dynamic_to_slvalue(value: Dynamic) -> Result<SlValue, ScriptLangError> {
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

fn slvalue_to_rhai_literal(value: &SlValue) -> String {
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

