use crate::*;
#[cfg(test)]
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy)]
pub(crate) struct CompileGroupMode {
    script_kind: ScriptKind,
    while_depth: usize,
    allow_option_direct_continue: bool,
}

impl CompileGroupMode {
    pub(crate) fn new(while_depth: usize, allow_option_direct_continue: bool) -> Self {
        Self {
            script_kind: ScriptKind::Goto,
            while_depth,
            allow_option_direct_continue,
        }
    }

    pub(crate) fn with_script_kind(mut self, script_kind: ScriptKind) -> Self {
        self.script_kind = script_kind;
        self
    }
}

fn script_target_var_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*$").expect("target var regex"))
}

fn script_literal_name_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"^[A-Za-z_][A-Za-z0-9_-]*(?:\.[A-Za-z_][A-Za-z0-9_-]*)?$")
            .expect("script literal name regex")
    })
}

fn function_ref_var_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*$").expect("function ref var regex"))
}

fn template_expr_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"\$\{([^{}]+)\}").expect("template expression regex"))
}

#[derive(Clone, Copy)]
pub(crate) enum RhaiCompileTarget {
    Expression,
    CodeBlock,
}

fn map_rhai_preprocess_error_to_compile(
    error: ScriptLangError,
    span: &SourceSpan,
) -> ScriptLangError {
    let code = format!("XML_{}", error.code);
    ScriptLangError::with_span(code, error.message, span.clone())
}

fn compile_rhai_source_for_target(
    source: &str,
    span: &SourceSpan,
    context: &str,
    target: RhaiCompileTarget,
) -> Result<(), ScriptLangError> {
    let engine = rhai::Engine::new();
    let result = match target {
        RhaiCompileTarget::Expression => engine.compile_expression(source).map(|_| ()),
        RhaiCompileTarget::CodeBlock => engine.compile(source).map(|_| ()),
    };
    result.map_err(|error| {
        ScriptLangError::with_span(
            "XML_RHAI_SYNTAX_INVALID",
            format!("Invalid Rhai {}: {}", context, error),
            span.clone(),
        )
    })
}

pub(crate) fn preprocess_and_compile_rhai_source(
    source: &str,
    span: &SourceSpan,
    context: &str,
    input_mode: RhaiInputMode,
    target: RhaiCompileTarget,
    runtime_function_symbol_map: &BTreeMap<String, String>,
    runtime_module_global_rewrite_map: &BTreeMap<String, String>,
) -> Result<String, ScriptLangError> {
    let preprocessed = preprocess_scriptlang_rhai_input(source, context, input_mode)
        .map_err(|error| map_rhai_preprocess_error_to_compile(error, span))?;
    let source_for_compile = rewrite_function_calls(&preprocessed, runtime_function_symbol_map);
    let source_for_compile = rewrite_module_global_qualified_access(
        &source_for_compile,
        runtime_module_global_rewrite_map,
    );
    compile_rhai_source_for_target(&source_for_compile, span, context, target)?;
    Ok(preprocessed)
}

fn preprocess_and_compile_template_expressions(
    template: &str,
    span: &SourceSpan,
    runtime_function_symbol_map: &BTreeMap<String, String>,
    runtime_module_global_rewrite_map: &BTreeMap<String, String>,
) -> Result<String, ScriptLangError> {
    let mut out = String::with_capacity(template.len());
    let mut last_index = 0usize;
    for captures in template_expr_regex().captures_iter(template) {
        let full = captures
            .get(0)
            .expect("capture group 0 must exist for each template capture");
        let expr = captures
            .get(1)
            .expect("capture group 1 must exist for each template capture");
        out.push_str(&template[last_index..full.start()]);
        let preprocessed = preprocess_and_compile_rhai_source(
            expr.as_str(),
            span,
            "text interpolation expression",
            RhaiInputMode::TextInterpolationExpr,
            RhaiCompileTarget::Expression,
            runtime_function_symbol_map,
            runtime_module_global_rewrite_map,
        )?;
        out.push_str("${");
        out.push_str(&preprocessed);
        out.push('}');
        last_index = full.end();
    }
    out.push_str(&template[last_index..]);
    Ok(out)
}

fn build_runtime_module_global_rewrite_map(
    visible_module_vars: &BTreeMap<String, ModuleVarDecl>,
    visible_module_consts: &BTreeMap<String, ModuleConstDecl>,
) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for qualified_name in visible_module_vars
        .values()
        .map(|decl| decl.qualified_name.as_str())
        .chain(
            visible_module_consts
                .values()
                .map(|decl| decl.qualified_name.as_str()),
        )
    {
        let Some((namespace, name)) = qualified_name.split_once('.') else {
            continue;
        };
        map.entry(qualified_name.to_string())
            .or_insert_with(|| format!("{}.{}", module_namespace_symbol(namespace), name));
    }
    map
}

fn build_runtime_function_symbol_map(
    visible_functions: &BTreeMap<String, FunctionDecl>,
) -> BTreeMap<String, String> {
    visible_functions
        .keys()
        .map(|name| (name.clone(), rhai_function_symbol(name)))
        .collect()
}

fn is_identifier_char(ch: Option<char>) -> bool {
    ch.is_some_and(|value| value.is_ascii_alphanumeric() || value == '_')
}

#[derive(Clone, Copy)]
enum ScriptMacroQuoteStyle {
    Single,
    Double,
}

fn script_name_literal(script_name: &str, quote_style: ScriptMacroQuoteStyle) -> String {
    match quote_style {
        ScriptMacroQuoteStyle::Single => {
            format!(
                "'{}'",
                script_name.replace('\\', "\\\\").replace('\'', "\\'")
            )
        }
        ScriptMacroQuoteStyle::Double => {
            format!(
                "\"{}\"",
                script_name.replace('\\', "\\\\").replace('"', "\\\"")
            )
        }
    }
}

fn rewrite_script_context_macro_in_expression(
    expr: &str,
    script_name: Option<&str>,
    quote_style: ScriptMacroQuoteStyle,
) -> String {
    let Some(script_name) = script_name else {
        return expr.to_string();
    };
    let replacement = script_name_literal(script_name, quote_style);
    let target_chars = "__script__".chars().collect::<Vec<_>>();
    let chars = expr.chars().collect::<Vec<_>>();
    let mut out = String::with_capacity(expr.len() + 8);
    let mut index = 0usize;
    let mut quote: Option<char> = None;

    while index < chars.len() {
        let ch = chars[index];
        if let Some(active_quote) = quote {
            out.push(ch);
            if ch == '\\' && index + 1 < chars.len() {
                index += 1;
                out.push(chars[index]);
            } else if ch == active_quote {
                quote = None;
            }
            index += 1;
            continue;
        }

        if ch == '\'' || ch == '"' {
            quote = Some(ch);
            out.push(ch);
            index += 1;
            continue;
        }

        if index + target_chars.len() <= chars.len()
            && chars[index..index + target_chars.len()] == target_chars[..]
            && !is_identifier_char(if index == 0 {
                None
            } else {
                Some(chars[index - 1])
            })
            && !is_identifier_char(chars.get(index + target_chars.len()).copied())
        {
            out.push_str(&replacement);
            index += target_chars.len();
            continue;
        }

        out.push(ch);
        index += 1;
    }

    out
}

fn rewrite_script_context_macro_in_template(template: &str, script_name: Option<&str>) -> String {
    if script_name.is_none() {
        return template.to_string();
    }

    let mut out = String::with_capacity(template.len());
    let mut last_index = 0usize;
    for captures in template_expr_regex().captures_iter(template) {
        let full = captures
            .get(0)
            .expect("capture group 0 must exist for each template capture");
        let expr = captures
            .get(1)
            .expect("capture group 1 must exist for each template capture");
        out.push_str(&template[last_index..full.start()]);
        let rewritten = rewrite_script_context_macro_in_expression(
            expr.as_str(),
            script_name,
            ScriptMacroQuoteStyle::Double,
        );
        out.push_str("${");
        out.push_str(&rewritten);
        out.push('}');
        last_index = full.end();
    }
    out.push_str(&template[last_index..]);
    out
}

fn qualify_script_literal_name(
    literal_name: &str,
    module_name: Option<&str>,
    span: &SourceSpan,
) -> Result<String, ScriptLangError> {
    if literal_name.contains('.') {
        return Ok(literal_name.to_string());
    }
    if let Some(module_name) = module_name {
        return Ok(format!("{}.{}", module_name, literal_name));
    }
    Err(ScriptLangError::with_span(
        "XML_SCRIPT_TARGET_INVALID",
        format!(
            "Short script literal \"@{}\" requires module context.",
            literal_name
        ),
        span.clone(),
    ))
}

fn validate_script_literal_access(
    qualified: &str,
    all_script_access: &BTreeMap<String, AccessLevel>,
    module_name: Option<&str>,
    span: &SourceSpan,
) -> Result<(), ScriptLangError> {
    let Some(access) = all_script_access.get(qualified).copied() else {
        return Err(ScriptLangError::with_span(
            "XML_SCRIPT_TARGET_NOT_FOUND",
            format!("Script target \"{}\" not found.", qualified),
            span.clone(),
        ));
    };

    if access == AccessLevel::Private {
        let target_module = qualified.split_once('.').map(|(ns, _)| ns).unwrap_or("");
        if module_name != Some(target_module) {
            return Err(ScriptLangError::with_span(
                "XML_SCRIPT_TARGET_ACCESS_DENIED",
                format!(
                    "Script target \"{}\" is private and cannot be referenced here.",
                    qualified
                ),
                span.clone(),
            ));
        }
    }
    Ok(())
}

fn parse_script_literal_name(chars: &[char], at_index: usize) -> Option<(String, usize)> {
    let mut index = at_index + 1;
    let mut name = String::new();
    let mut seen_dot = false;

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
        if ch == '.' && !seen_dot {
            let next = chars.get(index + 1).copied()?;
            if !next.is_ascii_alphabetic() && next != '_' {
                return None;
            }
            seen_dot = true;
            name.push(ch);
            index += 1;
            continue;
        }
        break;
    }

    Some((name, index))
}

fn is_script_literal_left_boundary(ch: Option<char>) -> bool {
    match ch {
        None => true,
        Some(value) => !value.is_ascii_alphanumeric() && value != '_' && value != '.',
    }
}

pub(crate) fn normalize_and_validate_script_literals_in_expression(
    expr: &str,
    span: &SourceSpan,
    module_name: Option<&str>,
    all_script_access: Option<&BTreeMap<String, AccessLevel>>,
) -> Result<String, ScriptLangError> {
    let chars = expr.chars().collect::<Vec<_>>();
    let mut out = String::with_capacity(expr.len());
    let mut index = 0usize;
    let mut quote: Option<char> = None;

    while index < chars.len() {
        let ch = chars[index];
        if let Some(active_quote) = quote {
            out.push(ch);
            if ch == '\\' && index + 1 < chars.len() {
                index += 1;
                out.push(chars[index]);
            } else if ch == active_quote {
                quote = None;
            }
            index += 1;
            continue;
        }

        if ch == '\'' || ch == '"' {
            quote = Some(ch);
            out.push(ch);
            index += 1;
            continue;
        }

        if ch == '@' && is_script_literal_left_boundary(chars.get(index.wrapping_sub(1)).copied()) {
            if let Some((literal_name, next_index)) = parse_script_literal_name(&chars, index) {
                let qualified = qualify_script_literal_name(&literal_name, module_name, span)?;
                if let Some(access_map) = all_script_access {
                    validate_script_literal_access(&qualified, access_map, module_name, span)?;
                }
                out.push('@');
                out.push_str(&qualified);
                index = next_index;
                continue;
            }
        }

        out.push(ch);
        index += 1;
    }

    Ok(out)
}

fn qualify_function_literal_name(
    literal_name: &str,
    module_name: Option<&str>,
    span: &SourceSpan,
) -> Result<String, ScriptLangError> {
    if literal_name.contains('.') {
        return Ok(literal_name.to_string());
    }
    if let Some(module_name) = module_name {
        return Ok(format!("{}.{}", module_name, literal_name));
    }
    Err(ScriptLangError::with_span(
        "XML_FUNCTION_LITERAL_INVALID",
        format!(
            "Short function literal \"*{}\" requires module context.",
            literal_name
        ),
        span.clone(),
    ))
}

fn parse_function_literal_name(chars: &[char], start: usize) -> Option<(String, usize)> {
    let mut index = start + 1;
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

fn is_function_literal_start(chars: &[char], index: usize) -> bool {
    if chars[index] != '*' {
        return false;
    }
    if index + 1 >= chars.len() {
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

pub(crate) fn normalize_and_validate_function_literals(
    expr: &str,
    span: &SourceSpan,
    module_name: Option<&str>,
    visible_functions: &BTreeMap<String, FunctionDecl>,
) -> Result<String, ScriptLangError> {
    normalize_and_validate_function_literals_with_lookup(expr, span, module_name, |qualified| {
        visible_functions.contains_key(qualified)
    })
}

pub(crate) fn normalize_and_validate_function_literals_with_names(
    expr: &str,
    span: &SourceSpan,
    module_name: Option<&str>,
    visible_function_names: &BTreeSet<String>,
) -> Result<String, ScriptLangError> {
    normalize_and_validate_function_literals_with_lookup(expr, span, module_name, |qualified| {
        visible_function_names.contains(qualified)
    })
}

fn normalize_and_validate_function_literals_with_lookup<F>(
    expr: &str,
    span: &SourceSpan,
    module_name: Option<&str>,
    mut has_visible_function: F,
) -> Result<String, ScriptLangError>
where
    F: FnMut(&str) -> bool,
{
    let chars = expr.chars().collect::<Vec<_>>();
    let mut out = String::with_capacity(expr.len());
    let mut index = 0usize;

    while index < chars.len() {
        if is_function_literal_start(&chars, index) {
            if let Some((literal_name, next_index)) = parse_function_literal_name(&chars, index) {
                let qualified = qualify_function_literal_name(&literal_name, module_name, span)?;
                if !has_visible_function(&qualified) {
                    return Err(ScriptLangError::with_span(
                        "XML_FUNCTION_LITERAL_NOT_FOUND",
                        format!(
                            "Function literal target \"{}\" not found or not visible.",
                            qualified
                        ),
                        span.clone(),
                    ));
                }
                let mut lookahead = next_index;
                while lookahead < chars.len() && chars[lookahead].is_whitespace() {
                    lookahead += 1;
                }
                if chars.get(lookahead) == Some(&'(') {
                    return Err(ScriptLangError::with_span(
                        "XML_FUNCTION_LITERAL_CALL_FORBIDDEN",
                        "Function literal cannot be called directly. Use method(...) or module.method(...).",
                        span.clone(),
                    ));
                }
                out.push('*');
                out.push_str(&qualified);
                index = next_index;
                continue;
            }
        }

        out.push(chars[index]);
        index += 1;
    }

    Ok(out)
}

fn extract_first_invoke_arg(chars: &[char], open_paren_index: usize) -> Option<String> {
    let mut index = open_paren_index;
    index += 1;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut brace_depth = 0usize;
    let mut quote: Option<char> = None;
    let mut out = String::new();

    while index < chars.len() {
        let ch = chars[index];
        if let Some(active_quote) = quote {
            out.push(ch);
            if ch == '\\' && index + 1 < chars.len() {
                index += 1;
                out.push(chars[index]);
            } else if ch == active_quote {
                quote = None;
            }
            index += 1;
            continue;
        }

        match ch {
            '\'' | '"' => {
                quote = Some(ch);
                out.push(ch);
            }
            '(' => {
                paren_depth += 1;
                out.push(ch);
            }
            ')' => {
                if paren_depth == 0 && bracket_depth == 0 && brace_depth == 0 {
                    return Some(out.trim().to_string());
                }
                paren_depth = paren_depth.saturating_sub(1);
                out.push(ch);
            }
            '[' => {
                bracket_depth += 1;
                out.push(ch);
            }
            ']' => {
                bracket_depth = bracket_depth.saturating_sub(1);
                out.push(ch);
            }
            '{' => {
                brace_depth += 1;
                out.push(ch);
            }
            '}' => {
                brace_depth = brace_depth.saturating_sub(1);
                out.push(ch);
            }
            ',' if paren_depth == 0 && bracket_depth == 0 && brace_depth == 0 => {
                return Some(out.trim().to_string());
            }
            _ => out.push(ch),
        }
        index += 1;
    }

    None
}

fn validate_invoke_first_arg(
    expr: &str,
    span: &SourceSpan,
    local_var_types: &BTreeMap<String, ScriptType>,
    visible_module_vars: &BTreeMap<String, ModuleVarDecl>,
    visible_module_consts: &BTreeMap<String, ModuleConstDecl>,
) -> Result<(), ScriptLangError> {
    let chars = expr.chars().collect::<Vec<_>>();
    let mut index = 0usize;
    while index + 6 <= chars.len() {
        if chars[index] != 'i' {
            index += 1;
            continue;
        }
        let candidate = chars[index..chars.len().min(index + 6)]
            .iter()
            .collect::<String>();
        if candidate != "invoke" {
            index += 1;
            continue;
        }
        let left_ok = if index == 0 {
            true
        } else {
            let prev = chars[index - 1];
            !prev.is_ascii_alphanumeric() && prev != '_' && prev != '.'
        };
        if !left_ok {
            index += 1;
            continue;
        }
        let mut open_index = index + 6;
        while open_index < chars.len() && chars[open_index].is_whitespace() {
            open_index += 1;
        }
        if chars.get(open_index) != Some(&'(') {
            index += 1;
            continue;
        }
        let first_arg = extract_first_invoke_arg(&chars, open_index).unwrap_or_default();
        if !function_ref_var_regex().is_match(first_arg.trim()) {
            return Err(ScriptLangError::with_span(
                "XML_INVOKE_TARGET_VAR_REQUIRED",
                "invoke first argument must be a function variable name.",
                span.clone(),
            ));
        }
        let var_name = first_arg.trim();
        let declared_type = local_var_types
            .get(var_name)
            .or_else(|| visible_module_vars.get(var_name).map(|decl| &decl.r#type))
            .or_else(|| visible_module_consts.get(var_name).map(|decl| &decl.r#type));
        if !matches!(declared_type, Some(ScriptType::Function)) {
            return Err(ScriptLangError::with_span(
                "XML_INVOKE_TARGET_VAR_TYPE",
                format!(
                    "invoke first argument \"{}\" must declare type=\"function\".",
                    var_name
                ),
                span.clone(),
            ));
        }
        index = open_index + 1;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn normalize_expression_literals(
    expr: &str,
    span: &SourceSpan,
    rhai_context: &str,
    rhai_compile_target: RhaiCompileTarget,
    all_script_access: &BTreeMap<String, AccessLevel>,
    module_name: Option<&str>,
    current_script_name: Option<&str>,
    visible_types: &BTreeMap<String, ScriptType>,
    visible_functions: &BTreeMap<String, FunctionDecl>,
    local_var_types: &BTreeMap<String, ScriptType>,
    visible_module_vars: &BTreeMap<String, ModuleVarDecl>,
    visible_module_consts: &BTreeMap<String, ModuleConstDecl>,
) -> Result<String, ScriptLangError> {
    let macro_rewritten = rewrite_script_context_macro_in_expression(
        expr,
        current_script_name,
        ScriptMacroQuoteStyle::Double,
    );
    let script_rewritten = normalize_and_validate_script_literals_in_expression(
        &macro_rewritten,
        span,
        module_name,
        Some(all_script_access),
    )?;
    let normalized = normalize_and_validate_function_literals(
        &script_rewritten,
        span,
        module_name,
        visible_functions,
    )?;
    let blocked_names = local_var_types
        .keys()
        .cloned()
        .collect::<BTreeSet<String>>();
    let module_aliases = build_module_symbol_alias_rewrite_map(
        visible_module_vars,
        visible_module_consts,
        &blocked_names,
    );
    let alias_rewritten = rewrite_module_symbol_aliases_in_expression(&normalized, &module_aliases);
    let rewritten =
        rewrite_and_validate_enum_literals_in_expression(&alias_rewritten, visible_types, span)?;
    validate_invoke_first_arg(
        &rewritten,
        span,
        local_var_types,
        visible_module_vars,
        visible_module_consts,
    )?;
    let runtime_module_global_rewrite_map =
        build_runtime_module_global_rewrite_map(visible_module_vars, visible_module_consts);
    let runtime_function_symbol_map = build_runtime_function_symbol_map(visible_functions);
    preprocess_and_compile_rhai_source(
        &rewritten,
        span,
        rhai_context,
        RhaiInputMode::CodeBlock,
        rhai_compile_target,
        &runtime_function_symbol_map,
        &runtime_module_global_rewrite_map,
    )
}

#[allow(clippy::too_many_arguments)]
fn normalize_attribute_expression_literals(
    expr: &str,
    span: &SourceSpan,
    all_script_access: &BTreeMap<String, AccessLevel>,
    module_name: Option<&str>,
    current_script_name: Option<&str>,
    visible_types: &BTreeMap<String, ScriptType>,
    visible_functions: &BTreeMap<String, FunctionDecl>,
    local_var_types: &BTreeMap<String, ScriptType>,
    visible_module_vars: &BTreeMap<String, ModuleVarDecl>,
    visible_module_consts: &BTreeMap<String, ModuleConstDecl>,
) -> Result<String, ScriptLangError> {
    let macro_rewritten = rewrite_script_context_macro_in_expression(
        expr,
        current_script_name,
        ScriptMacroQuoteStyle::Single,
    );
    let script_rewritten = normalize_and_validate_script_literals_in_expression(
        &macro_rewritten,
        span,
        module_name,
        Some(all_script_access),
    )?;
    let normalized = normalize_and_validate_function_literals(
        &script_rewritten,
        span,
        module_name,
        visible_functions,
    )?;
    let blocked_names = local_var_types
        .keys()
        .cloned()
        .collect::<BTreeSet<String>>();
    let module_aliases = build_module_symbol_alias_rewrite_map(
        visible_module_vars,
        visible_module_consts,
        &blocked_names,
    );
    let alias_rewritten = rewrite_module_symbol_aliases_in_expression(&normalized, &module_aliases);
    let rewritten = rewrite_and_validate_enum_literals_in_attr_expression(
        &alias_rewritten,
        visible_types,
        span,
    )?;
    validate_invoke_first_arg(
        &rewritten,
        span,
        local_var_types,
        visible_module_vars,
        visible_module_consts,
    )?;
    let runtime_module_global_rewrite_map =
        build_runtime_module_global_rewrite_map(visible_module_vars, visible_module_consts);
    let runtime_function_symbol_map = build_runtime_function_symbol_map(visible_functions);
    preprocess_and_compile_rhai_source(
        &rewritten,
        span,
        "attribute expression",
        RhaiInputMode::AttributeExpr,
        RhaiCompileTarget::Expression,
        &runtime_function_symbol_map,
        &runtime_module_global_rewrite_map,
    )
}

#[allow(clippy::too_many_arguments)]
fn normalize_template_literals(
    template: &str,
    span: &SourceSpan,
    all_script_access: &BTreeMap<String, AccessLevel>,
    module_name: Option<&str>,
    current_script_name: Option<&str>,
    visible_types: &BTreeMap<String, ScriptType>,
    visible_functions: &BTreeMap<String, FunctionDecl>,
    local_var_types: &BTreeMap<String, ScriptType>,
    visible_module_vars: &BTreeMap<String, ModuleVarDecl>,
    visible_module_consts: &BTreeMap<String, ModuleConstDecl>,
) -> Result<String, ScriptLangError> {
    let macro_rewritten = rewrite_script_context_macro_in_template(template, current_script_name);
    let rewritten =
        rewrite_and_validate_enum_literals_in_template(&macro_rewritten, visible_types, span)?;
    let rewritten = normalize_and_validate_script_literals_in_expression(
        &rewritten,
        span,
        module_name,
        Some(all_script_access),
    )?;
    let rewritten =
        normalize_and_validate_function_literals(&rewritten, span, module_name, visible_functions)?;
    let blocked_names = local_var_types
        .keys()
        .cloned()
        .collect::<BTreeSet<String>>();
    let module_aliases = build_module_symbol_alias_rewrite_map(
        visible_module_vars,
        visible_module_consts,
        &blocked_names,
    );
    let rewritten = rewrite_module_symbol_aliases_in_template(&rewritten, &module_aliases);
    validate_invoke_first_arg(
        &rewritten,
        span,
        local_var_types,
        visible_module_vars,
        visible_module_consts,
    )?;
    let runtime_module_global_rewrite_map =
        build_runtime_module_global_rewrite_map(visible_module_vars, visible_module_consts);
    let runtime_function_symbol_map = build_runtime_function_symbol_map(visible_functions);
    preprocess_and_compile_template_expressions(
        &rewritten,
        span,
        &runtime_function_symbol_map,
        &runtime_module_global_rewrite_map,
    )
}

fn parse_script_target_attr(
    raw_target: &str,
    node: &XmlElementNode,
    local_var_types: &BTreeMap<String, ScriptType>,
    visible_module_vars: &BTreeMap<String, ModuleVarDecl>,
    visible_module_consts: &BTreeMap<String, ModuleConstDecl>,
    all_script_access: &BTreeMap<String, AccessLevel>,
    module_name: Option<&str>,
) -> Result<ScriptTarget, ScriptLangError> {
    let target = raw_target.trim();
    if target.contains("${") {
        return Err(ScriptLangError::with_span(
            "XML_SCRIPT_TARGET_TEMPLATE_REMOVED",
            "Attribute \"script\" no longer supports ${...}; use @literal or script variable name.",
            node.location.clone(),
        ));
    }

    if let Some(stripped) = target.strip_prefix('@') {
        let script_name = stripped.trim();
        if script_name.is_empty() || !script_literal_name_regex().is_match(script_name) {
            return Err(ScriptLangError::with_span(
                "XML_SCRIPT_TARGET_INVALID",
                format!("Invalid script literal \"{}\".", target),
                node.location.clone(),
            ));
        }

        let qualified = qualify_script_literal_name(script_name, module_name, &node.location)?;
        validate_script_literal_access(&qualified, all_script_access, module_name, &node.location)?;

        return Ok(ScriptTarget::Literal {
            script_name: qualified,
        });
    }

    if !script_target_var_regex().is_match(target) {
        return Err(ScriptLangError::with_span(
            "XML_SCRIPT_TARGET_INVALID",
            format!(
                "script=\"{}\" is invalid. Use @module.script or script variable name.",
                target
            ),
            node.location.clone(),
        ));
    }

    let declared_type = local_var_types
        .get(target)
        .or_else(|| visible_module_vars.get(target).map(|decl| &decl.r#type))
        .or_else(|| visible_module_consts.get(target).map(|decl| &decl.r#type))
        .ok_or_else(|| {
            ScriptLangError::with_span(
                "XML_SCRIPT_TARGET_VAR_UNKNOWN",
                format!("Script target variable \"{}\" is not declared.", target),
                node.location.clone(),
            )
        })?;

    if !matches!(declared_type, ScriptType::Script) {
        return Err(ScriptLangError::with_span(
            "XML_SCRIPT_TARGET_VAR_TYPE",
            format!(
                "Script target variable \"{}\" must declare type=\"script\".",
                target
            ),
            node.location.clone(),
        ));
    }

    Ok(ScriptTarget::Variable {
        var_name: target.to_string(),
    })
}

pub(crate) fn compile_script(
    options: CompileScriptOptions<'_>,
) -> Result<ScriptIr, ScriptLangError> {
    let CompileScriptOptions {
        script_path,
        root,
        script_access,
        qualified_script_name,
        module_name,
        visible_types,
        visible_functions,
        visible_module_vars,
        visible_module_consts,
        all_script_access,
        invoke_all_functions,
    } = options;
    if root.name != "script" {
        return Err(ScriptLangError::with_span(
            "XML_ROOT_INVALID",
            "Script file root must be <script>.",
            root.location.clone(),
        ));
    }

    let local_script_name = get_required_non_empty_attr(root, "name")?;
    assert_decl_name_not_reserved_or_rhai_keyword(
        &local_script_name,
        "script",
        root.location.clone(),
    )?;
    let script_name = qualified_script_name
        .unwrap_or(&local_script_name)
        .to_string();

    let script_kind = parse_script_kind(root)?;
    let params = parse_script_args(root, visible_types, script_kind)?;
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

    compile_group_with_context(
        &root_group_id,
        None,
        &expanded_root,
        &mut builder,
        visible_types,
        visible_functions,
        visible_module_vars,
        visible_module_consts,
        all_script_access,
        module_name,
        Some(script_name.as_str()),
        &visible_var_types,
        CompileGroupMode::new(0, false).with_script_kind(script_kind),
    )?;

    Ok(ScriptIr {
        script_path: script_path.to_string(),
        script_name,
        access: script_access,
        module_name: module_name.map(|value| value.to_string()),
        local_script_name: module_name.map(|_| local_script_name.clone()),
        kind: script_kind,
        params,
        root_group_id,
        groups: builder.groups,
        visible_globals: Vec::new(),
        visible_functions: visible_functions.clone(),
        visible_module_vars: visible_module_vars.clone(),
        visible_module_consts: visible_module_consts.clone(),
        invoke_all_functions: invoke_all_functions.clone(),
    })
}

#[cfg(test)]
pub(crate) fn compile_group(
    group_id: &str,
    parent_group_id: Option<&str>,
    container: &XmlElementNode,
    builder: &mut GroupBuilder,
    visible_types: &BTreeMap<String, ScriptType>,
    visible_var_types: &BTreeMap<String, ScriptType>,
    mode: CompileGroupMode,
) -> Result<(), ScriptLangError> {
    compile_group_with_context(
        group_id,
        parent_group_id,
        container,
        builder,
        visible_types,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        None,
        None,
        visible_var_types,
        mode,
    )
}

#[allow(clippy::too_many_arguments)]
fn compile_group_with_context(
    group_id: &str,
    parent_group_id: Option<&str>,
    container: &XmlElementNode,
    builder: &mut GroupBuilder,
    visible_types: &BTreeMap<String, ScriptType>,
    visible_functions: &BTreeMap<String, FunctionDecl>,
    visible_module_vars: &BTreeMap<String, ModuleVarDecl>,
    visible_module_consts: &BTreeMap<String, ModuleConstDecl>,
    all_script_access: &BTreeMap<String, AccessLevel>,
    module_name: Option<&str>,
    current_script_name: Option<&str>,
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
        visible_functions,
        visible_module_vars,
        visible_module_consts,
        all_script_access,
        module_name,
        current_script_name,
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

#[allow(clippy::too_many_arguments)]
fn compile_child_group(
    parent_group_id: &str,
    child_group_id: &str,
    child_container: &XmlElementNode,
    builder: &mut GroupBuilder,
    visible_types: &BTreeMap<String, ScriptType>,
    visible_functions: &BTreeMap<String, FunctionDecl>,
    visible_module_vars: &BTreeMap<String, ModuleVarDecl>,
    visible_module_consts: &BTreeMap<String, ModuleConstDecl>,
    all_script_access: &BTreeMap<String, AccessLevel>,
    module_name: Option<&str>,
    current_script_name: Option<&str>,
    local_var_types: &mut BTreeMap<String, ScriptType>,
    mode: CompileGroupMode,
) -> Result<(), ScriptLangError> {
    compile_group_with_context(
        child_group_id,
        Some(parent_group_id),
        child_container,
        builder,
        visible_types,
        visible_functions,
        visible_module_vars,
        visible_module_consts,
        all_script_access,
        module_name,
        current_script_name,
        local_var_types,
        mode,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn compile_group_nodes(
    group_id: &str,
    container: &XmlElementNode,
    builder: &mut GroupBuilder,
    visible_types: &BTreeMap<String, ScriptType>,
    visible_functions: &BTreeMap<String, FunctionDecl>,
    visible_module_vars: &BTreeMap<String, ModuleVarDecl>,
    visible_module_consts: &BTreeMap<String, ModuleConstDecl>,
    all_script_access: &BTreeMap<String, AccessLevel>,
    module_name: Option<&str>,
    current_script_name: Option<&str>,
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

                compile_group_with_context(
                    &body_group_id,
                    Some(group_id),
                    child,
                    builder,
                    visible_types,
                    visible_functions,
                    visible_module_vars,
                    visible_module_consts,
                    all_script_access,
                    module_name,
                    current_script_name,
                    local_var_types,
                    CompileGroupMode::new(mode.while_depth, false)
                        .with_script_kind(mode.script_kind),
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
            "temp" => {
                let mut declaration = parse_var_declaration(child, visible_types)?;
                if let Some(expr) = declaration.initial_value_expr.as_mut() {
                    let raw_expr_quoted = {
                        let trimmed = expr.trim_start();
                        trimmed.starts_with('"') || trimmed.starts_with('\'')
                    };
                    *expr = normalize_expression_literals(
                        expr,
                        &child.location,
                        "var initializer expression",
                        RhaiCompileTarget::Expression,
                        all_script_access,
                        module_name,
                        current_script_name,
                        visible_types,
                        visible_functions,
                        local_var_types,
                        visible_module_vars,
                        visible_module_consts,
                    )?;
                    if matches!(declaration.r#type, ScriptType::Script) && raw_expr_quoted {
                        return Err(ScriptLangError::with_span(
                            "XML_SCRIPT_ASSIGN_STRING_FORBIDDEN",
                            "script type does not accept plain string literal; use @module.script.",
                            child.location.clone(),
                        ));
                    }
                    if matches!(declaration.r#type, ScriptType::Function) && raw_expr_quoted {
                        return Err(ScriptLangError::with_span(
                            "XML_FUNCTION_ASSIGN_STRING_FORBIDDEN",
                            "function type does not accept plain string literal; use *module.function.",
                            child.location.clone(),
                        ));
                    }
                }
                local_var_types.insert(declaration.name.clone(), declaration.r#type.clone());
                ScriptNode::Var {
                    id: builder.next_node_id("var"),
                    declaration,
                    location: child.location.clone(),
                }
            }
            "text" => ScriptNode::Text {
                id: builder.next_node_id("text"),
                value: normalize_template_literals(
                    &parse_inline_required(child)?,
                    &child.location,
                    all_script_access,
                    module_name,
                    current_script_name,
                    visible_types,
                    visible_functions,
                    local_var_types,
                    visible_module_vars,
                    visible_module_consts,
                )?,
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
                    value: normalize_template_literals(
                        &parse_inline_required(child)?,
                        &child.location,
                        all_script_access,
                        module_name,
                        current_script_name,
                        visible_types,
                        visible_functions,
                        local_var_types,
                        visible_module_vars,
                        visible_module_consts,
                    )?,
                    location: child.location.clone(),
                }
            }
            "code" => {
                let code = normalize_expression_literals(
                    &parse_inline_required(child)?,
                    &child.location,
                    "code block",
                    RhaiCompileTarget::CodeBlock,
                    all_script_access,
                    module_name,
                    current_script_name,
                    visible_types,
                    visible_functions,
                    local_var_types,
                    visible_module_vars,
                    visible_module_consts,
                )?;
                ScriptNode::Code {
                    id: builder.next_node_id("code"),
                    code,
                    location: child.location.clone(),
                }
            }
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

                let group_mode = CompileGroupMode::new(mode.while_depth, false)
                    .with_script_kind(mode.script_kind);
                let then_result = compile_child_group(
                    group_id,
                    &then_group_id,
                    &then_container,
                    builder,
                    visible_types,
                    visible_functions,
                    visible_module_vars,
                    visible_module_consts,
                    all_script_access,
                    module_name,
                    current_script_name,
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
                        visible_functions,
                        visible_module_vars,
                        visible_module_consts,
                        all_script_access,
                        module_name,
                        current_script_name,
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
                    when_expr: {
                        normalize_attribute_expression_literals(
                            &get_required_non_empty_attr(child, "when")?,
                            &child.location,
                            all_script_access,
                            module_name,
                            current_script_name,
                            visible_types,
                            visible_functions,
                            local_var_types,
                            visible_module_vars,
                            visible_module_consts,
                        )?
                    },
                    then_group_id,
                    else_group_id: Some(else_group_id),
                    location: child.location.clone(),
                }
            }
            "while" => {
                let body_group_id = builder.next_group_id();
                let while_mode = CompileGroupMode::new(mode.while_depth + 1, false)
                    .with_script_kind(mode.script_kind);
                let while_result = compile_child_group(
                    group_id,
                    &body_group_id,
                    child,
                    builder,
                    visible_types,
                    visible_functions,
                    visible_module_vars,
                    visible_module_consts,
                    all_script_access,
                    module_name,
                    current_script_name,
                    local_var_types,
                    while_mode,
                );
                while_result?;
                ScriptNode::While {
                    id: builder.next_node_id("while"),
                    when_expr: {
                        normalize_attribute_expression_literals(
                            &get_required_non_empty_attr(child, "when")?,
                            &child.location,
                            all_script_access,
                            module_name,
                            current_script_name,
                            visible_types,
                            visible_functions,
                            local_var_types,
                            visible_module_vars,
                            visible_module_consts,
                        )?
                    },
                    body_group_id,
                    location: child.location.clone(),
                }
            }
            "choice" => {
                let prompt_text = normalize_template_literals(
                    &get_required_non_empty_attr(child, "text")?,
                    &child.location,
                    all_script_access,
                    module_name,
                    current_script_name,
                    visible_types,
                    visible_functions,
                    local_var_types,
                    visible_module_vars,
                    visible_module_consts,
                )?;
                let mut entries = Vec::new();
                let mut fall_over_seen = 0usize;
                let mut fall_over_entry_index = None;

                for choice_child in element_children(child) {
                    match choice_child.name.as_str() {
                        "option" => {
                            let once = parse_bool_attr(choice_child, "once", false)?;
                            let fall_over = parse_bool_attr(choice_child, "fall_over", false)?;
                            let when_expr = get_optional_attr(choice_child, "when")
                                .map(|expr| {
                                    normalize_attribute_expression_literals(
                                        &expr,
                                        &choice_child.location,
                                        all_script_access,
                                        module_name,
                                        current_script_name,
                                        visible_types,
                                        visible_functions,
                                        local_var_types,
                                        visible_module_vars,
                                        visible_module_consts,
                                    )
                                })
                                .transpose()?;
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
                            let option_mode = CompileGroupMode::new(mode.while_depth, true)
                                .with_script_kind(mode.script_kind);
                            let option_result = compile_child_group(
                                group_id,
                                &option_group_id,
                                choice_child,
                                builder,
                                visible_types,
                                visible_functions,
                                visible_module_vars,
                                visible_module_consts,
                                all_script_access,
                                module_name,
                                current_script_name,
                                local_var_types,
                                option_mode,
                            );
                            option_result?;

                            entries.push(ChoiceEntry::Static {
                                option: ChoiceOption {
                                    id: builder.next_choice_id(),
                                    text: normalize_template_literals(
                                        &get_required_non_empty_attr(choice_child, "text")?,
                                        &choice_child.location,
                                        all_script_access,
                                        module_name,
                                        current_script_name,
                                        visible_types,
                                        visible_functions,
                                        local_var_types,
                                        visible_module_vars,
                                        visible_module_consts,
                                    )?,
                                    when_expr,
                                    once,
                                    fall_over,
                                    group_id: option_group_id,
                                    location: choice_child.location.clone(),
                                },
                            });
                        }
                        "dynamic-options" => {
                            let array_expr = normalize_attribute_expression_literals(
                                &get_required_non_empty_attr(choice_child, "array")?,
                                &choice_child.location,
                                all_script_access,
                                module_name,
                                current_script_name,
                                visible_types,
                                visible_functions,
                                local_var_types,
                                visible_module_vars,
                                visible_module_consts,
                            )?;
                            let item_name = get_required_non_empty_attr(choice_child, "item")?;
                            let index_name = get_optional_attr(choice_child, "index");
                            assert_decl_name_not_reserved_or_rhai_keyword(
                                &item_name,
                                "dynamic-options item",
                                choice_child.location.clone(),
                            )?;
                            if let Some(index_name_value) = &index_name {
                                assert_decl_name_not_reserved_or_rhai_keyword(
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
                            let option_mode = CompileGroupMode::new(mode.while_depth, true)
                                .with_script_kind(mode.script_kind);
                            let template_result = compile_child_group(
                                group_id,
                                &option_group_id,
                                template_option,
                                builder,
                                visible_types,
                                visible_functions,
                                visible_module_vars,
                                visible_module_consts,
                                all_script_access,
                                module_name,
                                current_script_name,
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
                                        text: normalize_template_literals(
                                            &get_required_non_empty_attr(template_option, "text")?,
                                            &template_option.location,
                                            all_script_access,
                                            module_name,
                                            current_script_name,
                                            visible_types,
                                            visible_functions,
                                            local_var_types,
                                            visible_module_vars,
                                            visible_module_consts,
                                        )?,
                                        when_expr: {
                                            let when_expr =
                                                get_optional_attr(template_option, "when");
                                            if let Some(expr) = when_expr.as_deref() {
                                                let rewritten =
                                                    normalize_attribute_expression_literals(
                                                        expr,
                                                        &template_option.location,
                                                        all_script_access,
                                                        module_name,
                                                        current_script_name,
                                                        visible_types,
                                                        visible_functions,
                                                        local_var_types,
                                                        visible_module_vars,
                                                        visible_module_consts,
                                                    )?;
                                                Some(rewritten)
                                            } else {
                                                None
                                            }
                                        },
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
                let max_length = parse_input_max_length(child)?;

                ScriptNode::Input {
                    id: builder.next_node_id("input"),
                    target_var: get_required_non_empty_attr(child, "var")?,
                    prompt_text: get_required_non_empty_attr(child, "text")?,
                    max_length,
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
                target_script: parse_script_target_attr(
                    &get_required_non_empty_attr(child, "script")?,
                    child,
                    local_var_types,
                    visible_module_vars,
                    visible_module_consts,
                    all_script_access,
                    module_name,
                )?,
                args: {
                    parse_args(get_optional_attr(child, "args"))?
                        .into_iter()
                        .map(|mut arg| {
                            if !arg.is_ref {
                                arg.value_expr = normalize_attribute_expression_literals(
                                    &arg.value_expr,
                                    &child.location,
                                    all_script_access,
                                    module_name,
                                    current_script_name,
                                    visible_types,
                                    visible_functions,
                                    local_var_types,
                                    visible_module_vars,
                                    visible_module_consts,
                                )?;
                            }
                            Ok::<_, ScriptLangError>(arg)
                        })
                        .collect::<Result<Vec<_>, _>>()?
                },
                location: child.location.clone(),
            },
            "goto" => {
                if mode.script_kind != ScriptKind::Goto {
                    return Err(ScriptLangError::with_span(
                        "XML_CALL_SCRIPT_GOTO_FORBIDDEN",
                        "Call script does not support <goto/>.",
                        child.location.clone(),
                    ));
                }
                let args = parse_args(get_optional_attr(child, "args"))?
                    .into_iter()
                    .map(|mut arg| {
                        if !arg.is_ref {
                            arg.value_expr = normalize_attribute_expression_literals(
                                &arg.value_expr,
                                &child.location,
                                all_script_access,
                                module_name,
                                current_script_name,
                                visible_types,
                                visible_functions,
                                local_var_types,
                                visible_module_vars,
                                visible_module_consts,
                            )?;
                        }
                        Ok::<_, ScriptLangError>(arg)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if args.iter().any(|arg| arg.is_ref) {
                    return Err(ScriptLangError::with_span(
                        "XML_GOTO_REF_UNSUPPORTED",
                        "Goto args do not support ref mode.",
                        child.location.clone(),
                    ));
                }

                ScriptNode::Goto {
                    id: builder.next_node_id("goto"),
                    target_script: parse_script_target_attr(
                        &get_required_non_empty_attr(child, "script")?,
                        child,
                        local_var_types,
                        visible_module_vars,
                        visible_module_consts,
                        all_script_access,
                        module_name,
                    )?,
                    args,
                    location: child.location.clone(),
                }
            }
            "return" => {
                if mode.script_kind != ScriptKind::Call {
                    return Err(ScriptLangError::with_span(
                        "XML_GOTO_SCRIPT_RETURN_FORBIDDEN",
                        "Goto script does not support <return/>.",
                        child.location.clone(),
                    ));
                }
                if has_attr(child, "script") || has_attr(child, "args") {
                    return Err(ScriptLangError::with_span(
                        "XML_RETURN_ATTR_NOT_ALLOWED",
                        "<return/> in call script does not support script/args attributes.",
                        child.location.clone(),
                    ));
                }
                if has_any_child_content(child) {
                    return Err(ScriptLangError::with_span(
                        "XML_RETURN_CONTENT_FORBIDDEN",
                        "<return/> cannot contain child nodes or inline text.",
                        child.location.clone(),
                    ));
                }

                ScriptNode::Return {
                    id: builder.next_node_id("return"),
                    location: child.location.clone(),
                }
            }
            "end" => {
                if mode.script_kind != ScriptKind::Goto {
                    return Err(ScriptLangError::with_span(
                        "XML_CALL_SCRIPT_END_FORBIDDEN",
                        "Call script does not support <end/>.",
                        child.location.clone(),
                    ));
                }
                if !child.attributes.is_empty() {
                    return Err(ScriptLangError::with_span(
                        "XML_END_ATTR_NOT_ALLOWED",
                        "<end/> does not support attributes.",
                        child.location.clone(),
                    ));
                }
                if has_any_child_content(child) {
                    return Err(ScriptLangError::with_span(
                        "XML_END_CONTENT_FORBIDDEN",
                        "<end/> cannot contain child nodes or inline text.",
                        child.location.clone(),
                    ));
                }
                ScriptNode::End {
                    id: builder.next_node_id("end"),
                    location: child.location.clone(),
                }
            }
            "for" => {
                return Err(ScriptLangError::with_span(
                    "XML_FOR_INTERNAL",
                    "<for> must be expanded before compile phase.",
                    child.location.clone(),
                ))
            }
            "temp-input" => {
                return Err(ScriptLangError::with_span(
                    "XML_TEMP_INPUT_INTERNAL",
                    "<temp-input> must be expanded before compile phase.",
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
            removed @ ("loop" | "var" | "vars" | "step" | "set" | "push" | "remove") => {
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
        | ScriptNode::Goto { id, .. }
        | ScriptNode::End { id, .. }
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
            "Attribute \"value\" is not allowed on <temp>. Use inline content instead.",
            node.location.clone(),
        ));
    }

    if let Some(child) = element_children(node).next() {
        return Err(ScriptLangError::with_span(
            "XML_VAR_CHILD_INVALID",
            format!(
                "<temp> cannot contain child element <{}>. Use inline expression text only.",
                child.name
            ),
            child.location.clone(),
        ));
    }

    let inline = inline_text_content(node);
    let initial_value_expr = if inline.trim().is_empty() {
        if matches!(ty, ScriptType::Enum { .. }) {
            return Err(ScriptLangError::with_span(
                "ENUM_INIT_REQUIRED",
                format!(
                    "<temp name=\"{}\"> with enum type requires explicit Type.Member initializer.",
                    name
                ),
                node.location.clone(),
            ));
        }
        None
    } else {
        let mut expr = inline.trim().to_string();
        if let ScriptType::Enum { type_name, members } = &ty {
            let member = parse_enum_literal_initializer(
                &expr,
                type_name,
                members,
                visible_types,
                &node.location,
            )?;
            expr = format!("\"{}\"", member.replace('"', "\\\""));
        } else if let ScriptType::Map {
            key_type: MapKeyType::Enum { type_name, members },
            ..
        } = &ty
        {
            validate_enum_map_initializer_keys_if_static(
                &expr,
                type_name,
                members,
                &node.location,
            )?;
        }
        Some(expr)
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
    script_kind: ScriptKind,
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
        if is_ref && script_kind == ScriptKind::Goto {
            return Err(ScriptLangError::with_span(
                "SCRIPT_GOTO_ARGS_REF_UNSUPPORTED",
                "Goto script params do not support ref mode.",
                root.location.clone(),
            ));
        }
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

        assert_decl_name_not_reserved_or_rhai_keyword(name, "script arg", root.location.clone())?;
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

fn parse_script_kind(root: &XmlElementNode) -> Result<ScriptKind, ScriptLangError> {
    let Some(raw) = get_optional_attr(root, "kind") else {
        return Ok(ScriptKind::Goto);
    };
    let value = raw.trim();
    if value.is_empty() {
        return Err(ScriptLangError::with_span(
            "XML_SCRIPT_KIND_INVALID",
            "Attribute \"kind\" on <script> must be \"call\" or \"goto\".",
            root.location.clone(),
        ));
    }
    match value {
        "call" => Ok(ScriptKind::Call),
        "goto" => Ok(ScriptKind::Goto),
        _ => Err(ScriptLangError::with_span(
            "XML_SCRIPT_KIND_INVALID",
            format!(
                "Unsupported <script kind=\"{}\">. Allowed values: call, goto.",
                value
            ),
            root.location.clone(),
        )),
    }
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
        assert_decl_name_not_reserved_or_rhai_keyword(name, "function arg", node.location.clone())?;

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
) -> Result<ParsedFunctionReturnDecl, ScriptLangError> {
    if has_attr(node, "return") {
        return Err(ScriptLangError::with_span(
            "FUNCTION_RETURN_ATTR_INVALID",
            "Attribute \"return\" is not allowed on <function>.",
            node.location.clone(),
        ));
    }
    let raw = get_required_non_empty_attr(node, "return_type")?;
    Ok(ParsedFunctionReturnDecl {
        type_expr: parse_type_expr(&raw, &node.location)?,
        location: node.location.clone(),
    })
}

fn parse_input_max_length(node: &XmlElementNode) -> Result<Option<usize>, ScriptLangError> {
    let Some(raw) = get_optional_attr(node, "max_length") else {
        return Ok(None);
    };
    let parsed = raw.trim().parse::<usize>().map_err(|_| {
        ScriptLangError::with_span(
            "XML_INPUT_MAX_LENGTH_INVALID",
            format!(
                "Attribute \"max_length\" on <input> must be a non-negative integer, got \"{}\".",
                raw
            ),
            node.location.clone(),
        )
    })?;
    Ok(Some(parsed))
}

pub(crate) fn contains_return_statement(code: &str) -> bool {
    let bytes = code.as_bytes();
    let mut idx = 0usize;
    while idx + 6 <= bytes.len() {
        if &bytes[idx..idx + 6] == b"return" {
            let left_ok =
                idx == 0 || !(bytes[idx - 1].is_ascii_alphanumeric() || bytes[idx - 1] == b'_');
            let right_ok = idx + 6 == bytes.len()
                || !(bytes[idx + 6].is_ascii_alphanumeric() || bytes[idx + 6] == b'_');
            if left_ok && right_ok {
                return true;
            }
        }
        idx += 1;
    }
    false
}

#[cfg(test)]
mod script_compile_tests {
    use super::*;
    use crate::compiler_test_support::*;

    fn script_type_kind(ty: &ScriptType) -> &'static str {
        match ty {
            ScriptType::Primitive { .. } => "primitive",
            ScriptType::Enum { .. } => "enum",
            ScriptType::Script => "script",
            ScriptType::Function => "function",
            ScriptType::Array { .. } => "array",
            ScriptType::Map { .. } => "map",
            ScriptType::Object { .. } => "object",
        }
    }

    #[test]
    fn parse_var_declaration_rejects_value_attr_and_child_elements() {
        assert_eq!(script_type_kind(&ScriptType::Script), "script");
        assert_eq!(script_type_kind(&ScriptType::Function), "function");
        assert_eq!(
            script_type_kind(&ScriptType::Enum {
                type_name: "Status".to_string(),
                members: vec![]
            }),
            "enum"
        );
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
        let parsed =
            parse_script_args(&root_ok, &visible_types, ScriptKind::Call).expect("args parse");
        assert_eq!(parsed.len(), 2);
        assert!(parsed[1].is_ref);

        let root_bad = xml_element("script", &[("args", "int")], Vec::new());
        let error =
            parse_script_args(&root_bad, &visible_types, ScriptKind::Call).expect_err("bad args");
        assert_eq!(error.code, "SCRIPT_ARGS_PARSE_ERROR");

        let root_dup = xml_element("script", &[("args", "int:a,int:a")], Vec::new());
        let error = parse_script_args(&root_dup, &visible_types, ScriptKind::Call)
            .expect_err("duplicate args");
        assert_eq!(error.code, "SCRIPT_ARGS_DUPLICATE");

        let root_bad_type = xml_element("script", &[("args", "#{ }:a")], Vec::new());
        let error = parse_script_args(&root_bad_type, &visible_types, ScriptKind::Call)
            .expect_err("invalid arg type expr");
        assert_eq!(error.code, "TYPE_PARSE_ERROR");
        let root_unknown_type = xml_element("script", &[("args", "Missing:a")], Vec::new());
        let error = parse_script_args(&root_unknown_type, &visible_types, ScriptKind::Call)
            .expect_err("unknown arg type");
        assert_eq!(error.code, "TYPE_UNKNOWN");
        let root_keyword_arg = xml_element("script", &[("args", "int:shared")], Vec::new());
        let error = parse_script_args(&root_keyword_arg, &visible_types, ScriptKind::Call)
            .expect_err("keyword arg name");
        assert_eq!(error.code, "NAME_RHAI_KEYWORD_RESERVED");

        let fn_node = xml_element(
            "function",
            &[("name", "f"), ("args", "ref:int:a"), ("return_type", "int")],
            vec![xml_text("return a;")],
        );
        let error = parse_function_declaration_node(&fn_node).expect_err("ref arg unsupported");
        assert_eq!(error.code, "XML_FUNCTION_ARGS_REF_UNSUPPORTED");

        let fn_bad_return = xml_element(
            "function",
            &[("name", "f"), ("args", "int:a"), ("return", "int:r")],
            vec![xml_text("return a;")],
        );
        let error =
            parse_function_declaration_node(&fn_bad_return).expect_err("return attr invalid");
        assert_eq!(error.code, "FUNCTION_RETURN_ATTR_INVALID");
        let fn_reserved_arg = xml_element(
            "function",
            &[
                ("name", "f"),
                ("args", "int:__sl_a"),
                ("return_type", "int"),
            ],
            vec![xml_text("return 1;")],
        );
        let error =
            parse_function_declaration_node(&fn_reserved_arg).expect_err("reserved arg name");
        assert_eq!(error.code, "NAME_RESERVED_PREFIX");
        let fn_keyword_arg = xml_element(
            "function",
            &[
                ("name", "f"),
                ("args", "int:shared"),
                ("return_type", "int"),
            ],
            vec![xml_text("return 1;")],
        );
        let error = parse_function_declaration_node(&fn_keyword_arg).expect_err("keyword arg name");
        assert_eq!(error.code, "NAME_RHAI_KEYWORD_RESERVED");

        let fn_bad_arg_type = xml_element(
            "function",
            &[("name", "f"), ("args", "#{ }:a"), ("return_type", "int")],
            vec![xml_text("return 1;")],
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
    fn script_kind_error_paths_are_covered() {
        // Test empty kind attribute (line 1800)
        let root = parse_xml_document(r#"<script name="main" kind=""><text>Main</text></script>"#)
            .expect("xml")
            .root;
        let error = compile_script(CompileScriptOptions {
            script_path: "main.xml",
            root: &root,
            script_access: AccessLevel::Public,
            qualified_script_name: Some("main.main"),
            module_name: Some("main"),
            visible_types: &BTreeMap::new(),
            visible_functions: &BTreeMap::new(),
            visible_module_vars: &BTreeMap::new(),
            visible_module_consts: &BTreeMap::new(),
            all_script_access: &BTreeMap::new(),
            invoke_all_functions: &BTreeMap::new(),
        })
        .expect_err("empty kind should fail");
        assert_eq!(error.code, "XML_SCRIPT_KIND_INVALID");

        // Test explicit goto kind (line 1808)
        let root =
            parse_xml_document(r#"<script name="main" kind="goto"><text>Main</text></script>"#)
                .expect("xml")
                .root;
        let result = compile_script(CompileScriptOptions {
            script_path: "main.xml",
            root: &root,
            script_access: AccessLevel::Public,
            qualified_script_name: Some("main.main"),
            module_name: Some("main"),
            visible_types: &BTreeMap::new(),
            visible_functions: &BTreeMap::new(),
            visible_module_vars: &BTreeMap::new(),
            visible_module_consts: &BTreeMap::new(),
            all_script_access: &BTreeMap::new(),
            invoke_all_functions: &BTreeMap::new(),
        });
        // Explicit goto kind should compile successfully
        assert!(result.is_ok());

        // Test invalid kind attribute (line 1808-1811)
        let root =
            parse_xml_document(r#"<script name="main" kind="invalid"><text>Main</text></script>"#)
                .expect("xml")
                .root;
        let error = compile_script(CompileScriptOptions {
            script_path: "main.xml",
            root: &root,
            script_access: AccessLevel::Public,
            qualified_script_name: Some("main.main"),
            module_name: Some("main"),
            visible_types: &BTreeMap::new(),
            visible_functions: &BTreeMap::new(),
            visible_module_vars: &BTreeMap::new(),
            visible_module_consts: &BTreeMap::new(),
            all_script_access: &BTreeMap::new(),
            invoke_all_functions: &BTreeMap::new(),
        })
        .expect_err("invalid kind should fail");
        assert_eq!(error.code, "XML_SCRIPT_KIND_INVALID");

        // Test goto in call script (line 1472)
        let root = parse_xml_document(
            r#"<script name="main" kind="call"><goto script="@next"/></script>"#,
        )
        .expect("xml")
        .root;
        let error = compile_script(CompileScriptOptions {
            script_path: "main.xml",
            root: &root,
            script_access: AccessLevel::Public,
            qualified_script_name: Some("main.main"),
            module_name: Some("main"),
            visible_types: &BTreeMap::new(),
            visible_functions: &BTreeMap::new(),
            visible_module_vars: &BTreeMap::new(),
            visible_module_consts: &BTreeMap::new(),
            all_script_access: &BTreeMap::new(),
            invoke_all_functions: &BTreeMap::new(),
        })
        .expect_err("goto in call script should fail");
        assert_eq!(error.code, "XML_CALL_SCRIPT_GOTO_FORBIDDEN");

        // Test return in goto script (line 1520)
        let root = parse_xml_document(r#"<script name="main"><return/></script>"#)
            .expect("xml")
            .root;
        let error = compile_script(CompileScriptOptions {
            script_path: "main.xml",
            root: &root,
            script_access: AccessLevel::Public,
            qualified_script_name: Some("main.main"),
            module_name: Some("main"),
            visible_types: &BTreeMap::new(),
            visible_functions: &BTreeMap::new(),
            visible_module_vars: &BTreeMap::new(),
            visible_module_consts: &BTreeMap::new(),
            all_script_access: &BTreeMap::new(),
            invoke_all_functions: &BTreeMap::new(),
        })
        .expect_err("return in goto script should fail");
        assert_eq!(error.code, "XML_GOTO_SCRIPT_RETURN_FORBIDDEN");

        // Test end in call script (line 1548)
        let root = parse_xml_document(r#"<script name="main" kind="call"><end/></script>"#)
            .expect("xml")
            .root;
        let error = compile_script(CompileScriptOptions {
            script_path: "main.xml",
            root: &root,
            script_access: AccessLevel::Public,
            qualified_script_name: Some("main.main"),
            module_name: Some("main"),
            visible_types: &BTreeMap::new(),
            visible_functions: &BTreeMap::new(),
            visible_module_vars: &BTreeMap::new(),
            visible_module_consts: &BTreeMap::new(),
            all_script_access: &BTreeMap::new(),
            invoke_all_functions: &BTreeMap::new(),
        })
        .expect_err("end in call script should fail");
        assert_eq!(error.code, "XML_CALL_SCRIPT_END_FORBIDDEN");

        // Test return with child content (line 1534)
        let root =
            parse_xml_document(r#"<script name="main" kind="call"><return>bad</return></script>"#)
                .expect("xml")
                .root;
        let error = compile_script(CompileScriptOptions {
            script_path: "main.xml",
            root: &root,
            script_access: AccessLevel::Public,
            qualified_script_name: Some("main.main"),
            module_name: Some("main"),
            visible_types: &BTreeMap::new(),
            visible_functions: &BTreeMap::new(),
            visible_module_vars: &BTreeMap::new(),
            visible_module_consts: &BTreeMap::new(),
            all_script_access: &BTreeMap::new(),
            invoke_all_functions: &BTreeMap::new(),
        })
        .expect_err("return with content should fail");
        assert_eq!(error.code, "XML_RETURN_CONTENT_FORBIDDEN");

        // Test end with attributes (line 1555)
        let root = parse_xml_document(r#"<script name="main"><end attr="bad"/></script>"#)
            .expect("xml")
            .root;
        let error = compile_script(CompileScriptOptions {
            script_path: "main.xml",
            root: &root,
            script_access: AccessLevel::Public,
            qualified_script_name: Some("main.main"),
            module_name: Some("main"),
            visible_types: &BTreeMap::new(),
            visible_functions: &BTreeMap::new(),
            visible_module_vars: &BTreeMap::new(),
            visible_module_consts: &BTreeMap::new(),
            all_script_access: &BTreeMap::new(),
            invoke_all_functions: &BTreeMap::new(),
        })
        .expect_err("end with attr should fail");
        assert_eq!(error.code, "XML_END_ATTR_NOT_ALLOWED");

        // Test end with child content (line 1562)
        let root = parse_xml_document(r#"<script name="main"><end>bad</end></script>"#)
            .expect("xml")
            .root;
        let error = compile_script(CompileScriptOptions {
            script_path: "main.xml",
            root: &root,
            script_access: AccessLevel::Public,
            qualified_script_name: Some("main.main"),
            module_name: Some("main"),
            visible_types: &BTreeMap::new(),
            visible_functions: &BTreeMap::new(),
            visible_module_vars: &BTreeMap::new(),
            visible_module_consts: &BTreeMap::new(),
            all_script_access: &BTreeMap::new(),
            invoke_all_functions: &BTreeMap::new(),
        })
        .expect_err("end with content should fail");
        assert_eq!(error.code, "XML_END_CONTENT_FORBIDDEN");

        // Test goto script with ref args (line 1753)
        let root = parse_xml_document(
            r#"<script name="main" args="ref:int:x"><text>Main</text></script>"#,
        )
        .expect("xml")
        .root;
        let error = compile_script(CompileScriptOptions {
            script_path: "main.xml",
            root: &root,
            script_access: AccessLevel::Public,
            qualified_script_name: Some("main.main"),
            module_name: Some("main"),
            visible_types: &BTreeMap::new(),
            visible_functions: &BTreeMap::new(),
            visible_module_vars: &BTreeMap::new(),
            visible_module_consts: &BTreeMap::new(),
            all_script_access: &BTreeMap::new(),
            invoke_all_functions: &BTreeMap::new(),
        })
        .expect_err("ref args in goto script should fail");
        assert_eq!(error.code, "SCRIPT_GOTO_ARGS_REF_UNSUPPORTED");

        // Test goto with empty/missing script attribute (line 1506)
        let root = parse_xml_document(r#"<script name="main"><goto script=""/></script>"#)
            .expect("xml")
            .root;
        let error = compile_script(CompileScriptOptions {
            script_path: "main.xml",
            root: &root,
            script_access: AccessLevel::Public,
            qualified_script_name: Some("main.main"),
            module_name: Some("main"),
            visible_types: &BTreeMap::new(),
            visible_functions: &BTreeMap::new(),
            visible_module_vars: &BTreeMap::new(),
            visible_module_consts: &BTreeMap::new(),
            all_script_access: &BTreeMap::new(),
            invoke_all_functions: &BTreeMap::new(),
        })
        .expect_err("empty script attr should fail");
        assert_eq!(error.code, "XML_EMPTY_ATTR");
    }

    #[test]
    fn parse_function_return_and_type_expr_success_paths_are_covered() {
        let function_node = xml_element(
            "function",
            &[("name", "f"), ("return_type", "int")],
            vec![xml_text("return 1;")],
        );
        let parsed_return = parse_function_return(&function_node).expect("return should parse");
        let is_primitive =
            matches!(&parsed_return.type_expr, ParsedTypeExpr::Primitive(n) if n == "int");
        assert!(is_primitive, "return type should be primitive int");

        // Test non-primitive return types to cover the match branch
        let array_func = xml_element(
            "function",
            &[("name", "f"), ("return_type", "int[]")],
            vec![xml_text("return [];")],
        );
        let _parsed_array = parse_function_return(&array_func).expect("array return should parse");
        // Just verify it parses - type correctness is covered by xml_utils.rs tests

        // Note: parse_type_expr variants are already covered in xml_utils.rs tests
        // directly calling parse_type_expr is sufficient for coverage

        let span = SourceSpan::synthetic();
        let _ = parse_type_expr("int[]", &span).expect("array should parse");
        let _ = parse_type_expr("#{int}", &span).expect("map should parse");

        let reserved_return = xml_element(
            "function",
            &[("name", "f"), ("return", "int:out")],
            vec![xml_text("return 1;")],
        );
        let error = parse_function_return(&reserved_return).expect_err("return attr should fail");
        assert_eq!(error.code, "FUNCTION_RETURN_ATTR_INVALID");
        let keyword_return = xml_element(
            "function",
            &[("name", "f"), ("return_type", "int:out")],
            vec![xml_text("return 1;")],
        );
        let error = parse_function_return(&keyword_return).expect_err("invalid return type syntax");
        assert_eq!(error.code, "TYPE_PARSE_ERROR");

        let invalid_return = xml_element(
            "function",
            &[("name", "f"), ("return_type", "#{ }:out")],
            vec![xml_text("return 1;")],
        );
        let error = parse_function_return(&invalid_return).expect_err("invalid return type");
        assert_eq!(error.code, "TYPE_PARSE_ERROR");
        assert!(contains_return_statement("if x > 0 { return x; }"));
        assert!(!contains_return_statement("x = x + 1;"));
        // return preceded/followed by alphanumeric or underscore should not match
        assert!(!contains_return_statement("return_value"));
        assert!(!contains_return_statement("x_return"));
        assert!(!contains_return_statement("preturn")); // 'return' at start but preceded by 'p'
    }

    #[test]
    fn function_literal_and_invoke_validation_helpers_cover_new_paths() {
        let span = SourceSpan::synthetic();
        let mut visible_functions = BTreeMap::new();
        visible_functions.insert(
            "main.add".to_string(),
            FunctionDecl {
                name: "main.add".to_string(),
                params: vec![FunctionParam {
                    name: "x".to_string(),
                    r#type: ScriptType::Primitive {
                        name: "int".to_string(),
                    },
                    location: span.clone(),
                }],
                return_binding: FunctionReturn {
                    r#type: ScriptType::Primitive {
                        name: "int".to_string(),
                    },
                    location: span.clone(),
                },
                code: "out = x;".to_string(),
                location: span.clone(),
            },
        );

        assert_eq!(
            qualify_function_literal_name("main.add", Some("main"), &span).expect("qualified"),
            "main.add"
        );
        assert_eq!(
            qualify_function_literal_name("add", Some("main"), &span).expect("short"),
            "main.add"
        );
        let short_no_module =
            qualify_function_literal_name("add", None, &span).expect_err("short should fail");
        assert_eq!(short_no_module.code, "XML_FUNCTION_LITERAL_INVALID");

        let parsed = parse_function_literal_name(&"*main.add".chars().collect::<Vec<_>>(), 0)
            .expect("function literal parse");
        assert_eq!(parsed.0, "main.add");
        assert!(
            parse_function_literal_name(&"*bad.".chars().collect::<Vec<_>>(), 0).is_none(),
            "invalid function literal should fail parse"
        );

        // Test line 135: chars.get(index) returns None when start + 1 >= chars.len()
        // "*a" has 2 chars, start=1 gives index=2 which is >= 2 (out of bounds)
        assert!(
            parse_function_literal_name(&"*a".chars().collect::<Vec<_>>(), 1).is_none(),
            "start at last char should return None"
        );

        // Test line 150: next char after '.' is not alphanumeric or '_'
        assert!(
            parse_function_literal_name(&"*a.@".chars().collect::<Vec<_>>(), 0).is_none(),
            "dot followed by non-alphanumeric should return None"
        );

        assert!(is_function_literal_start(
            &"*add".chars().collect::<Vec<_>>(),
            0
        ));
        assert!(!is_function_literal_start(
            &"x *add".chars().collect::<Vec<_>>(),
            2
        ));

        let normalized = normalize_and_validate_function_literals(
            "f = *add; g = *main.add;",
            &span,
            Some("main"),
            &visible_functions,
        )
        .expect("function literal normalize");
        assert_eq!(normalized, "f = *main.add; g = *main.add;");

        let call_error = normalize_and_validate_function_literals(
            "*main.add(1)",
            &span,
            Some("main"),
            &visible_functions,
        )
        .expect_err("direct function literal call should fail");
        assert_eq!(call_error.code, "XML_FUNCTION_LITERAL_CALL_FORBIDDEN");

        let missing_error = normalize_and_validate_function_literals(
            "*main.missing",
            &span,
            Some("main"),
            &visible_functions,
        )
        .expect_err("missing function literal should fail");
        assert_eq!(missing_error.code, "XML_FUNCTION_LITERAL_NOT_FOUND");

        // Test line 193: short function literal without module context
        let no_module_error =
            normalize_and_validate_function_literals("*add", &span, None, &visible_functions)
                .expect_err("short function literal without module should fail");
        assert_eq!(no_module_error.code, "XML_FUNCTION_LITERAL_INVALID");

        // Test line 205-206: whitespace after function literal
        let with_whitespace = normalize_and_validate_function_literals(
            "*add ",
            &span,
            Some("main"),
            &visible_functions,
        )
        .expect("trailing whitespace should be ok");
        assert_eq!(with_whitespace, "*main.add ");

        // Test line 262-263: closing paren at top level
        let simple_paren = "invoke(fn)".chars().collect::<Vec<_>>();
        assert_eq!(
            extract_first_invoke_arg(&simple_paren, 6).as_deref(),
            Some("fn"),
            "simple paren should return the arg"
        );

        let chars = "invoke(fnRef, [1, foo(2)])".chars().collect::<Vec<_>>();
        assert_eq!(
            extract_first_invoke_arg(&chars, 6).as_deref(),
            Some("fnRef"),
            "extract first invoke arg should keep first segment"
        );
        let broken = "invoke(fnRef".chars().collect::<Vec<_>>();
        assert!(
            extract_first_invoke_arg(&broken, 6).is_none(),
            "broken invoke should not extract arg"
        );

        let mut local_var_types = BTreeMap::new();
        local_var_types.insert("fnRef".to_string(), ScriptType::Function);
        validate_invoke_first_arg(
            "invoke(fnRef, [1])",
            &span,
            &local_var_types,
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .expect("function var invoke should pass");

        let invoke_literal_error = validate_invoke_first_arg(
            "invoke(*main.add, [1])",
            &span,
            &local_var_types,
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .expect_err("invoke literal should fail");
        assert_eq!(invoke_literal_error.code, "XML_INVOKE_TARGET_VAR_REQUIRED");

        let mut non_function_vars = BTreeMap::new();
        non_function_vars.insert(
            "x".to_string(),
            ScriptType::Primitive {
                name: "int".to_string(),
            },
        );
        let invoke_non_function_error = validate_invoke_first_arg(
            "invoke(x, [1])",
            &span,
            &non_function_vars,
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .expect_err("invoke non-function var should fail");
        assert_eq!(invoke_non_function_error.code, "XML_INVOKE_TARGET_VAR_TYPE");

        // Test line 345: variable in visible_module_vars
        let mut module_vars = BTreeMap::new();
        module_vars.insert(
            "fnRef".to_string(),
            ModuleVarDecl {
                namespace: "main".to_string(),
                name: "fnRef".to_string(),
                qualified_name: "main.fnRef".to_string(),
                access: AccessLevel::Public,
                r#type: ScriptType::Function,
                initial_value_expr: None,
                location: span.clone(),
            },
        );
        validate_invoke_first_arg(
            "invoke(fnRef, [1])",
            &span,
            &BTreeMap::new(), // empty local_var_types
            &module_vars,
            &BTreeMap::new(),
        )
        .expect("function var in module vars should pass");

        // Test line 346: variable in visible_module_consts
        let mut module_consts = BTreeMap::new();
        module_consts.insert(
            "fnRef".to_string(),
            ModuleConstDecl {
                namespace: "main".to_string(),
                name: "fnRef".to_string(),
                qualified_name: "main.fnRef".to_string(),
                access: AccessLevel::Public,
                r#type: ScriptType::Function,
                initial_value_expr: None,
                location: span.clone(),
            },
        );
        validate_invoke_first_arg(
            "invoke(fnRef, [1])",
            &span,
            &BTreeMap::new(), // empty local_var_types
            &BTreeMap::new(), // empty module_vars
            &module_consts,
        )
        .expect("function var in module consts should pass");

        let fallback_literal = normalize_and_validate_function_literals(
            "*1bad",
            &span,
            Some("main"),
            &visible_functions,
        )
        .expect("invalid function literal token should stay raw");
        assert_eq!(fallback_literal, "*1bad");
        assert!(!is_function_literal_start(
            &"*".chars().collect::<Vec<_>>(),
            0
        ));

        let mut all_script_access = BTreeMap::new();
        all_script_access.insert("main.next".to_string(), AccessLevel::Public);
        let normalized_expr = normalize_expression_literals(
            "target = *add; step = @next; invoke(fnRef, [1]);",
            &span,
            "test code block",
            RhaiCompileTarget::CodeBlock,
            &all_script_access,
            Some("main"),
            Some("main.main"),
            &BTreeMap::new(),
            &visible_functions,
            &local_var_types,
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .expect("normalize expression should pass");
        assert!(normalized_expr.contains("*main.add"));
        assert!(normalized_expr.contains("@main.next"));

        // Test line 385: validate_invoke_first_arg error path through normalize_expression_literals
        let mut non_function_vars = BTreeMap::new();
        non_function_vars.insert(
            "fnRef".to_string(),
            ScriptType::Primitive {
                name: "int".to_string(),
            },
        );
        let invoke_error = normalize_expression_literals(
            "invoke(fnRef, [1])",
            &span,
            "test expression",
            RhaiCompileTarget::Expression,
            &all_script_access,
            Some("main"),
            Some("main.main"),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &non_function_vars,
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .expect_err("invoke non-function should fail");
        assert_eq!(invoke_error.code, "XML_INVOKE_TARGET_VAR_TYPE");

        // Test line 376: normalize_and_validate_function_literals error path
        let missing_fn_error = normalize_expression_literals(
            "*missing.func",
            &span,
            "test expression",
            RhaiCompileTarget::Expression,
            &all_script_access,
            Some("main"),
            Some("main.main"),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .expect_err("missing function should fail");
        assert_eq!(missing_fn_error.code, "XML_FUNCTION_LITERAL_NOT_FOUND");

        // Test line 378: rewrite_and_validate_enum_literals_in_expression error path
        // First, set up an enum type
        let mut visible_types = BTreeMap::new();
        visible_types.insert(
            "Status".to_string(),
            ScriptType::Enum {
                type_name: "Status".to_string(),
                members: vec!["Active".to_string()],
            },
        );
        let enum_error = normalize_expression_literals(
            "Status.Invalid",
            &span,
            "test expression",
            RhaiCompileTarget::Expression,
            &all_script_access,
            Some("main"),
            Some("main.main"),
            &visible_types,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .expect_err("invalid enum member should fail");
        assert_eq!(enum_error.code, "ENUM_LITERAL_MEMBER_UNKNOWN");

        let normalized_attr = normalize_attribute_expression_literals(
            "invoke(fnRef, [1])",
            &span,
            &all_script_access,
            Some("main"),
            Some("main.main"),
            &BTreeMap::new(),
            &visible_functions,
            &local_var_types,
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .expect("normalize attr should pass");
        assert_eq!(normalized_attr, "invoke(fnRef, [1])");

        let normalized_template = normalize_template_literals(
            "go ${invoke(fnRef, [1])}",
            &span,
            &all_script_access,
            Some("main"),
            Some("main.main"),
            &BTreeMap::new(),
            &visible_functions,
            &local_var_types,
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .expect("normalize template should pass");
        assert!(normalized_template.contains("invoke(fnRef, [1])"));

        let script_macro_expr = normalize_expression_literals(
            "__script__",
            &span,
            "test expression",
            RhaiCompileTarget::Expression,
            &all_script_access,
            Some("main"),
            Some("main.main"),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .expect("script macro in expression should expand");
        assert_eq!(script_macro_expr, "\"main.main\"");

        let quoted_script_macro_expr = normalize_expression_literals(
            "\"__script__\"",
            &span,
            "test expression",
            RhaiCompileTarget::Expression,
            &all_script_access,
            Some("main"),
            Some("main.main"),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .expect("quoted script macro text should remain literal");
        assert_eq!(quoted_script_macro_expr, "\"__script__\"");

        // Test escape characters in quoted strings
        // This covers the escape character handling in rewrite_script_context_macro_in_expression
        let escaped_string = normalize_expression_literals(
            "\"test\\nvalue\"", // string with escaped newline
            &span,
            "test expression",
            RhaiCompileTarget::Expression,
            &all_script_access,
            Some("main"),
            Some("main.main"),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .expect("escaped characters in string should be preserved");
        assert_eq!(escaped_string, "\"test\\nvalue\"");

        // Test string ending with backslash (edge case for index + 1 < chars.len())
        let backslash_end = normalize_expression_literals(
            "\"test\\\\\"",
            &span,
            "test expression",
            RhaiCompileTarget::Expression,
            &all_script_access,
            Some("main"),
            Some("main.main"),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .expect("string ending with backslash should be preserved");
        assert_eq!(backslash_end, "\"test\\\\\"");

        let script_macro_attr = normalize_attribute_expression_literals(
            "__script__",
            &span,
            &all_script_access,
            Some("main"),
            Some("main.main"),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .expect("script macro in attribute expression should normalize as Rhai source");
        assert_eq!(script_macro_attr, "\"main.main\"");

        let script_macro_template = normalize_template_literals(
            "expr=${__script__}; raw=__script__",
            &span,
            &all_script_access,
            Some("main"),
            Some("main.main"),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .expect("script macro in template should expand only inside capture");
        assert_eq!(
            script_macro_template,
            "expr=${\"main.main\"}; raw=__script__"
        );

        // Test __script__ with identifier chars before and after (should NOT be replaced)
        // This covers the is_identifier_char branches in rewrite_script_context_macro_in_expression
        let script_macro_adjacent_to_identifier = normalize_expression_literals(
            "prefix__script__suffix",
            &span,
            "test expression",
            RhaiCompileTarget::Expression,
            &all_script_access,
            Some("main"),
            Some("main.main"),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .expect("__script__ adjacent to identifiers should NOT be replaced");
        // prefix__script__suffix - the __script__ is part of identifier, should remain as-is
        assert_eq!(
            script_macro_adjacent_to_identifier,
            "prefix__script__suffix"
        );

        // Test __script__ at start of string with non-identifier after
        let script_macro_at_start = normalize_expression_literals(
            "__script__ + main",
            &span,
            "test expression",
            RhaiCompileTarget::Expression,
            &all_script_access,
            Some("main"),
            Some("main.main"),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .expect("__script__ at start should be replaced");
        assert_eq!(script_macro_at_start, "\"main.main\" + main");

        // Test __script__ at end of string with non-identifier before
        let script_macro_at_end = normalize_expression_literals(
            "main + __script__",
            &span,
            "test expression",
            RhaiCompileTarget::Expression,
            &all_script_access,
            Some("main"),
            Some("main.main"),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .expect("__script__ at end should be replaced");
        assert_eq!(script_macro_at_end, "main + \"main.main\"");

        // Test line 403: normalize_and_validate_function_literals error in attr
        let missing_fn_attr_error = normalize_attribute_expression_literals(
            "*missing.func",
            &span,
            &all_script_access,
            Some("main"),
            Some("main.main"),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .expect_err("missing function in attr should fail");
        assert_eq!(missing_fn_attr_error.code, "XML_FUNCTION_LITERAL_NOT_FOUND");

        // Test line 405: rewrite_and_validate_enum_literals_in_attr_expression error
        let enum_attr_error = normalize_attribute_expression_literals(
            "Status.Invalid",
            &span,
            &all_script_access,
            Some("main"),
            Some("main.main"),
            &visible_types,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .expect_err("invalid enum in attr should fail");
        assert_eq!(enum_attr_error.code, "ENUM_LITERAL_MEMBER_UNKNOWN");

        // Test line 412: validate_invoke_first_arg error in attr
        let invoke_attr_error = normalize_attribute_expression_literals(
            "invoke(x, [1])",
            &span,
            &all_script_access,
            Some("main"),
            Some("main.main"),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &non_function_vars,
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .expect_err("invoke non-function in attr should fail");
        assert_eq!(invoke_attr_error.code, "XML_INVOKE_TARGET_VAR_TYPE");

        // Test line 431: normalize_and_validate_function_literals error in template
        let missing_fn_template_error = normalize_template_literals(
            "${*missing.func}",
            &span,
            &all_script_access,
            Some("main"),
            Some("main.main"),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .expect_err("missing function in template should fail");
        assert_eq!(
            missing_fn_template_error.code,
            "XML_FUNCTION_LITERAL_NOT_FOUND"
        );

        // Test line 438: validate_invoke_first_arg error in template
        let invoke_template_error = normalize_template_literals(
            "${invoke(x, [1])}",
            &span,
            &all_script_access,
            Some("main"),
            Some("main.main"),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &non_function_vars,
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .expect_err("invoke non-function in template should fail");
        assert_eq!(invoke_template_error.code, "XML_INVOKE_TARGET_VAR_TYPE");

        let nested_chars = r#"invoke(foo("a\"b", [1], {k: (2)}), [3])"#
            .chars()
            .collect::<Vec<_>>();
        assert_eq!(
            extract_first_invoke_arg(&nested_chars, 6).as_deref(),
            Some(r#"foo("a\"b", [1], {k: (2)})"#)
        );

        validate_invoke_first_arg(
            "obj.invoke(fnRef, [1])",
            &span,
            &local_var_types,
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .expect("non-builtin invoke call should be ignored");
        validate_invoke_first_arg(
            "invoke fnRef, [1]",
            &span,
            &local_var_types,
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .expect("invoke without open paren should be ignored");
    }

    #[test]
    fn function_temp_rejects_plain_string_initializer() {
        let root = parse_xml_document(
            r#"<script name="main"><temp name="fnRef" type="function">"main.add"</temp></script>"#,
        )
        .expect("xml")
        .root;
        let mut visible_functions = BTreeMap::new();
        visible_functions.insert(
            "main.add".to_string(),
            FunctionDecl {
                name: "main.add".to_string(),
                params: Vec::new(),
                return_binding: FunctionReturn {
                    r#type: ScriptType::Primitive {
                        name: "int".to_string(),
                    },
                    location: SourceSpan::synthetic(),
                },
                code: "out = 1;".to_string(),
                location: SourceSpan::synthetic(),
            },
        );
        let error = compile_script(CompileScriptOptions {
            script_path: "main.xml",
            root: &root,
            script_access: AccessLevel::Public,
            qualified_script_name: Some("main.main"),
            module_name: Some("main"),
            visible_types: &BTreeMap::new(),
            visible_functions: &visible_functions,
            visible_module_vars: &BTreeMap::new(),
            visible_module_consts: &BTreeMap::new(),
            all_script_access: &BTreeMap::new(),
            invoke_all_functions: &BTreeMap::new(),
        })
        .expect_err("function temp with string should fail");
        assert_eq!(error.code, "XML_FUNCTION_ASSIGN_STRING_FORBIDDEN");
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
                            "temp",
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
            CompileGroupMode::new(0, false).with_script_kind(ScriptKind::Call),
        )
        .expect_err("call return with attrs should fail");
        assert_eq!(error.code, "XML_RETURN_ATTR_NOT_ALLOWED");

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
        assert_eq!(error.code, "XML_REMOVED_NODE");

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
        assert_eq!(error.code, "XML_REMOVED_NODE");

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
        assert_eq!(error.code, "XML_REMOVED_NODE");

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
        assert_eq!(error.code, "XML_REMOVED_NODE");

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
        assert_eq!(error.code, "XML_REMOVED_NODE");
    }

    #[test]
    fn compiler_error_matrix_covers_more_validation_paths() {
        let cases: Vec<(&str, BTreeMap<String, String>, &str)> = vec![
                (
                    "module child invalid",
                    map(&[
                        (
                            "x.xml",
                            "<module name=\"x\"><unknown/></module>",
                        ),
                        (
                            "main.xml",
                            r#"
    <!-- import x from x.xml -->
    <module name="main" export="script:main">
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
                            "<module name=\"x\"><type name=\"A\"><bad/></type></module>",
                        ),
                        (
                            "main.xml",
                            r#"
    <!-- import x from x.xml -->
    <module name="main" export="script:main">
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
                            "<module name=\"x\"><type name=\"A\"><field name=\"v\" type=\"int\"/><field name=\"v\" type=\"int\"/></type></module>",
                        ),
                        (
                            "main.xml",
                            r#"
    <!-- import x from x.xml -->
    <module name="main" export="script:main">
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
                            "<module name=\"x\"><function name=\"f\" return_type=\"int\">return 1;</function><function name=\"f\" return_type=\"int\">return 2;</function></module>",
                        ),
                        (
                            "main.xml",
                            r#"
    <!-- import x from x.xml -->
    <module name="main" export="script:main">
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
                        "<script name=\"main\"><temp name=\"x\" type=\"Unknown\"/></script>",
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
                    "dynamic options keyword item",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><choice text=\"c\"><dynamic-options array=\"arr\" item=\"shared\"><option text=\"a\"/></dynamic-options></choice></script>",
                    )]),
                    "NAME_RHAI_KEYWORD_RESERVED",
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
                    "input max_length negative invalid",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><input var=\"x\" text=\"p\" max_length=\"-1\"/></script>",
                    )]),
                    "XML_INPUT_MAX_LENGTH_INVALID",
                ),
                (
                    "input max_length text invalid",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><input var=\"x\" text=\"p\" max_length=\"abc\"/></script>",
                    )]),
                    "XML_INPUT_MAX_LENGTH_INVALID",
                ),
                (
                    "goto ref unsupported",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><goto script=\"next\" args=\"ref:x\"/></script>",
                    )]),
                    "XML_GOTO_REF_UNSUPPORTED",
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
                        "<script name=\"main\"><call script=\"@main.main\" args=\"ref:\"/></script>",
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
                    "script args keyword",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\" args=\"int:shared\"><text>x</text></script>",
                    )]),
                    "NAME_RHAI_KEYWORD_RESERVED",
                ),
                (
                    "loop removed",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><loop times=\"${n}\"><text>x</text></loop></script>",
                    )]),
                    "XML_REMOVED_NODE",
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
                        "<script name=\"main\"><temp name=\"n\" type=\"string\">\"\"</temp><input var=\"n\"/></script>",
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
                    "goto args parse error",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><goto script=\"s\" args=\"ref:\"/></script>",
                    )]),
                    "CALL_ARGS_PARSE_ERROR",
                ),
                (
                    "var missing name",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><temp type=\"int\">1</temp></script>",
                    )]),
                    "XML_MISSING_ATTR",
                ),
                (
                    "var missing type",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><temp name=\"x\">1</temp></script>",
                    )]),
                    "XML_MISSING_ATTR",
                ),
                (
                    "var type parse error",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><temp name=\"x\" type=\"#{ }\">1</temp></script>",
                    )]),
                    "TYPE_PARSE_ERROR",
                ),
                (
                    "for macro expansion error",
                    map(&[(
                        "main.xml",
                        "<script name=\"main\"><for temps=\"i:int:0\"><text>x</text></for></script>",
                    )]),
                    "XML_MISSING_ATTR",
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
        let reachable = collect_reachable_imports("missing.xml", &BTreeMap::new());
        assert!(reachable.contains("missing.xml"));

        let visible_empty = collect_visible_global_symbols(
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
                imports: Vec::new(),
                alias_directives: Vec::new(),
                xml_root: None,
                json_value: Some(SlValue::Number(1.0)),
            },
        );
        sources.insert(
            "b/x.json".to_string(),
            SourceFile {
                kind: SourceKind::Json,
                imports: Vec::new(),
                alias_directives: Vec::new(),
                xml_root: None,
                json_value: Some(SlValue::Number(2.0)),
            },
        );
        let duplicate_visible = collect_visible_global_symbols(
            &BTreeSet::from(["a/x.json".to_string(), "b/x.json".to_string()]),
            &sources,
        )
        .expect_err("duplicate visible global data symbol should fail");
        assert_eq!(duplicate_visible.code, "GLOBAL_DATA_SYMBOL_DUPLICATE");

        let invalid_file_name = parse_global_data_symbol("/").expect_err("invalid file name");
        assert_eq!(invalid_file_name.code, "GLOBAL_DATA_SYMBOL_INVALID");

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
                access: AccessLevel::Private,
                fields: vec![field.clone()],
                enum_members: Vec::new(),
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
                access: AccessLevel::Private,
                fields: vec![field.clone(), field],
                enum_members: Vec::new(),
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
            &ParsedTypeExpr::Map {
                key_type: Box::new(ParsedTypeExpr::Primitive("string".to_string())),
                value_type: Box::new(ParsedTypeExpr::Primitive("string".to_string())),
            },
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
            &ParsedTypeExpr::Map {
                key_type: Box::new(ParsedTypeExpr::Primitive("string".to_string())),
                value_type: Box::new(ParsedTypeExpr::Primitive("int".to_string())),
            },
            &BTreeMap::new(),
            &span,
        )
        .expect("map should resolve");
        assert_eq!(script_type_kind(&map_resolved), "map");

        let non_script_root = xml_element("module", &[("name", "x")], Vec::new());
        let compile_root_error = compile_script(CompileScriptOptions {
            script_path: "x.xml",
            root: &non_script_root,
            script_access: AccessLevel::Private,
            qualified_script_name: None,
            module_name: None,
            visible_types: &BTreeMap::new(),
            visible_functions: &BTreeMap::new(),
            visible_module_vars: &BTreeMap::new(),
            visible_module_consts: &BTreeMap::new(),
            all_script_access: &BTreeMap::new(),
            invoke_all_functions: &BTreeMap::new(),
        })
        .expect_err("compile_script should require script root");
        assert_eq!(compile_root_error.code, "XML_ROOT_INVALID");

        let missing_name_root = xml_element("script", &[], Vec::new());
        let missing_name_error = compile_script(CompileScriptOptions {
            script_path: "x.xml",
            root: &missing_name_root,
            script_access: AccessLevel::Private,
            qualified_script_name: None,
            module_name: None,
            visible_types: &BTreeMap::new(),
            visible_functions: &BTreeMap::new(),
            visible_module_vars: &BTreeMap::new(),
            visible_module_consts: &BTreeMap::new(),
            all_script_access: &BTreeMap::new(),
            invoke_all_functions: &BTreeMap::new(),
        })
        .expect_err("compile_script should require script name");
        assert_eq!(missing_name_error.code, "XML_MISSING_ATTR");

        let reserved_name_root = xml_element("script", &[("name", "__bad")], Vec::new());
        let reserved_name_error = compile_script(CompileScriptOptions {
            script_path: "x.xml",
            root: &reserved_name_root,
            script_access: AccessLevel::Private,
            qualified_script_name: None,
            module_name: None,
            visible_types: &BTreeMap::new(),
            visible_functions: &BTreeMap::new(),
            visible_module_vars: &BTreeMap::new(),
            visible_module_consts: &BTreeMap::new(),
            all_script_access: &BTreeMap::new(),
            invoke_all_functions: &BTreeMap::new(),
        })
        .expect_err("compile_script should reject reserved name");
        assert_eq!(reserved_name_error.code, "NAME_RESERVED_PREFIX");
        let keyword_name_root = xml_element("script", &[("name", "shared")], Vec::new());
        let keyword_name_error = compile_script(CompileScriptOptions {
            script_path: "x.xml",
            root: &keyword_name_root,
            script_access: AccessLevel::Private,
            qualified_script_name: None,
            module_name: None,
            visible_types: &BTreeMap::new(),
            visible_functions: &BTreeMap::new(),
            visible_module_vars: &BTreeMap::new(),
            visible_module_consts: &BTreeMap::new(),
            all_script_access: &BTreeMap::new(),
            invoke_all_functions: &BTreeMap::new(),
        })
        .expect_err("compile_script should reject keyword name");
        assert_eq!(keyword_name_error.code, "NAME_RHAI_KEYWORD_RESERVED");

        let reserved_var_root = xml_element(
            "script",
            &[("name", "main")],
            vec![XmlNode::Element(xml_element(
                "temp",
                &[("name", "__bad"), ("type", "int")],
                vec![xml_text("1")],
            ))],
        );
        let reserved_var_error = compile_script(CompileScriptOptions {
            script_path: "x.xml",
            root: &reserved_var_root,
            script_access: AccessLevel::Private,
            qualified_script_name: Some("x.main"),
            module_name: Some("x"),
            visible_types: &BTreeMap::new(),
            visible_functions: &BTreeMap::new(),
            visible_module_vars: &BTreeMap::new(),
            visible_module_consts: &BTreeMap::new(),
            all_script_access: &BTreeMap::new(),
            invoke_all_functions: &BTreeMap::new(),
        })
        .expect_err("compile_script should reject reserved var names");
        assert_eq!(reserved_var_error.code, "NAME_RESERVED_PREFIX");

        let no_module_root =
            parse_xml_document(r#"<script name="main"><call script="@next.next"/></script>"#)
                .expect("xml")
                .root;
        let no_module_scripts = BTreeMap::from([("next.next".to_string(), AccessLevel::Public)]);
        let no_module_ir = compile_script(CompileScriptOptions {
            script_path: "x.xml",
            root: &no_module_root,
            script_access: AccessLevel::Private,
            qualified_script_name: Some("main"),
            module_name: None,
            visible_types: &BTreeMap::new(),
            visible_functions: &BTreeMap::new(),
            visible_module_vars: &BTreeMap::new(),
            visible_module_consts: &BTreeMap::new(),
            all_script_access: &no_module_scripts,
            invoke_all_functions: &BTreeMap::new(),
        })
        .expect("compile without module name");
        let root_group = no_module_ir
            .groups
            .get(&no_module_ir.root_group_id)
            .expect("root group");
        assert!(matches!(
            &root_group.nodes[0],
            ScriptNode::Call {
                target_script: ScriptTarget::Literal { script_name },
                ..
            } if script_name == "next.next"
        ));

        let rich_script = map(&[(
            "main.xml",
            r#"
    <module name="main" export="script:main">
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

        let module_resolution = map(&[
            (
                "shared.xml",
                r##"
    <module name="shared" export="function:build;type:Obj">
      <type name="Obj">
        <field name="values" type="#{int[]}"/>
      </type>
      <function name="build" return_type="Obj">
        return #{values: #{a: [1]}};
      </function>
    </module>
    "##,
            ),
            (
                "main.xml",
                r#"
	    <!-- import shared from shared.xml -->
	    <module name="main" export="script:main">
	<script name="main">
	      <temp name="x" type="shared.Obj"/>
	    </script>
	</module>
	    "#,
            ),
        ]);
        let _ = compile_project_bundle_from_xml_map(&module_resolution)
            .expect("module return/field type resolution should pass");

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

        let mut for_builder = GroupBuilder::new("for.xml");
        let for_group = for_builder.next_group_id();
        let for_error = compile_group(
            &for_group,
            None,
            &xml_element(
                "script",
                &[("name", "main")],
                vec![XmlNode::Element(xml_element(
                    "for",
                    &[
                        ("temps", "i:int:0"),
                        ("condition", "i < 2"),
                        ("iteration", "i = i + 1;"),
                    ],
                    vec![XmlNode::Element(xml_element(
                        "text",
                        &[],
                        vec![xml_text("x")],
                    ))],
                ))],
            ),
            &mut for_builder,
            &BTreeMap::new(),
            &BTreeMap::new(),
            CompileGroupMode::new(0, false),
        )
        .expect_err("for should have been expanded");
        assert_eq!(for_error.code, "XML_FOR_INTERNAL");

        let mut temp_input_builder = GroupBuilder::new("temp-input.xml");
        let temp_input_group = temp_input_builder.next_group_id();
        let temp_input_error = compile_group(
            &temp_input_group,
            None,
            &xml_element(
                "script",
                &[("name", "main")],
                vec![XmlNode::Element(xml_element(
                    "temp-input",
                    &[
                        ("name", "hero"),
                        ("type", "string"),
                        ("text", "Name your hero"),
                    ],
                    vec![xml_text("\"Traveler\"")],
                ))],
            ),
            &mut temp_input_builder,
            &BTreeMap::new(),
            &BTreeMap::new(),
            CompileGroupMode::new(0, false),
        )
        .expect_err("temp-input should have been expanded");
        assert_eq!(temp_input_error.code, "XML_TEMP_INPUT_INTERNAL");

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
            max_length: None,
            location: SourceSpan::synthetic(),
        };
        let input_id = node_id(&input_node);
        assert_eq!(input_id, "i1");
        let call_node = ScriptNode::Call {
            id: "c1".to_string(),
            target_script: ScriptTarget::Literal {
                script_name: "main".to_string(),
            },
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
      <temp name="arr" type="int[]">[1,2]</temp>
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
            ScriptKind::Call,
        )
        .expect("empty script args should be accepted");
        assert!(empty_args.is_empty());
        let args_with_empty_segment = parse_script_args(
            &xml_element("script", &[("args", "int:a,,int:b")], Vec::new()),
            &BTreeMap::new(),
            ScriptKind::Call,
        )
        .expect("empty arg segment should be ignored");
        assert_eq!(args_with_empty_segment.len(), 2);
        let args_bad_start = parse_script_args(
            &xml_element("script", &[("args", ":a")], Vec::new()),
            &BTreeMap::new(),
            ScriptKind::Call,
        )
        .expect_err("bad args should fail");
        assert_eq!(args_bad_start.code, "SCRIPT_ARGS_PARSE_ERROR");
        let args_bad_end = parse_script_args(
            &xml_element("script", &[("args", "int:")], Vec::new()),
            &BTreeMap::new(),
            ScriptKind::Call,
        )
        .expect_err("bad args should fail");
        assert_eq!(args_bad_end.code, "SCRIPT_ARGS_PARSE_ERROR");
        let args_empty_name = parse_script_args(
            &xml_element("script", &[("args", "int:   ")], Vec::new()),
            &BTreeMap::new(),
            ScriptKind::Call,
        )
        .expect_err("empty script arg name should fail");
        assert_eq!(args_empty_name.code, "SCRIPT_ARGS_PARSE_ERROR");

        let empty_fn_args = parse_function_args(&xml_element(
            "function",
            &[("name", "f"), ("args", "   "), ("return_type", "int")],
            vec![xml_text("return 1;")],
        ))
        .expect("empty function args should be accepted");
        assert!(empty_fn_args.is_empty());
        let fn_args_bad_start = parse_function_args(&xml_element(
            "function",
            &[("name", "f"), ("args", ":a"), ("return_type", "int")],
            vec![xml_text("return 1;")],
        ))
        .expect_err("bad function args should fail");
        assert_eq!(fn_args_bad_start.code, "FUNCTION_ARGS_PARSE_ERROR");
        let fn_args_bad_end = parse_function_args(&xml_element(
            "function",
            &[("name", "f"), ("args", "int:"), ("return_type", "int")],
            vec![xml_text("return 1;")],
        ))
        .expect_err("bad function args should fail");
        assert_eq!(fn_args_bad_end.code, "FUNCTION_ARGS_PARSE_ERROR");
        let fn_args_dup = parse_function_args(&xml_element(
            "function",
            &[
                ("name", "f"),
                ("args", "int:a,int:a"),
                ("return_type", "int"),
            ],
            vec![xml_text("return 1;")],
        ))
        .expect_err("duplicate function args should fail");
        assert_eq!(fn_args_dup.code, "FUNCTION_ARGS_DUPLICATE");
        let fn_args_no_colon = parse_function_args(&xml_element(
            "function",
            &[("name", "f"), ("args", "int"), ("return_type", "int")],
            vec![xml_text("return 1;")],
        ))
        .expect_err("function arg without colon should fail");
        assert_eq!(fn_args_no_colon.code, "FUNCTION_ARGS_PARSE_ERROR");

        let ret_attr_invalid = parse_function_return(&xml_element(
            "function",
            &[("name", "f"), ("return", "int")],
            vec![xml_text("x")],
        ))
        .expect_err("return attr should fail");
        assert_eq!(ret_attr_invalid.code, "FUNCTION_RETURN_ATTR_INVALID");
        let ret_bad_edge = parse_function_return(&xml_element(
            "function",
            &[("name", "f"), ("return_type", "int:")],
            vec![xml_text("x")],
        ))
        .expect_err("return parse should fail");
        assert_eq!(ret_bad_edge.code, "TYPE_PARSE_ERROR");

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
            &xml_element("temp", &[("name", "")], Vec::new()),
            &mut declared,
        );
        assert!(declared.is_empty());
        collect_declared_var_names(&xml_element("temp", &[], Vec::new()), &mut declared);
        assert!(declared.is_empty());
        validate_reserved_prefix_in_user_var_declarations(&xml_element(
            "temp",
            &[("name", "")],
            Vec::new(),
        ))
        .expect("empty var name should be ignored");
        validate_reserved_prefix_in_user_var_declarations(&xml_element("temp", &[], Vec::new()))
            .expect("var without name should be ignored");

        let mut context = MacroExpansionContext {
            used_var_names: BTreeSet::from([format!("{}{}_first", FOR_FIRST_TEMP_VAR_PREFIX, 0)]),
            for_counter: 0,
        };
        let generated = next_for_first_flag_var_name(&mut context);
        assert!(generated.ends_with("_first"));

        assert_eq!(
            crate::defaults::slvalue_from_json(JsonValue::Null),
            SlValue::String("null".to_string())
        );

        // Test build_runtime_module_global_rewrite_map: qualified_name without namespace (no '.')
        let span = SourceSpan::synthetic();
        let no_namespace_vars = BTreeMap::from([(
            "localVar".to_string(),
            ModuleVarDecl {
                namespace: "".to_string(),
                name: "localVar".to_string(),
                qualified_name: "localVar".to_string(),
                access: AccessLevel::Private,
                r#type: ScriptType::Primitive {
                    name: "int".to_string(),
                },
                initial_value_expr: None,
                location: span.clone(),
            },
        )]);
        let no_namespace_map =
            build_runtime_module_global_rewrite_map(&no_namespace_vars, &BTreeMap::new());
        assert!(
            no_namespace_map.is_empty(),
            "no namespace vars should be skipped"
        );

        // Test build_runtime_function_symbol_map with empty map
        let empty_fn_map = build_runtime_function_symbol_map(&BTreeMap::new());
        assert!(empty_fn_map.is_empty());
    }

    #[test]
    fn normalize_template_literals_error_paths_are_covered() {
        // Test lines 135-136: normalize_template_literals error propagation
        use sl_core::ScriptType;

        // Test 1: <text> node with invalid enum member in template (line 496)
        let compile_error = |xml: &str, expected_code: &str| {
            let root = parse_xml_document(xml).expect("xml").root;
            let known_scripts = BTreeMap::new();
            let enum_type = ScriptType::Enum {
                type_name: "Status".to_string(),
                members: vec!["Active".to_string(), "Inactive".to_string()],
            };
            let mut visible_types = BTreeMap::new();
            visible_types.insert("Status".to_string(), enum_type);

            let error = compile_script(CompileScriptOptions {
                script_path: "main.xml",
                root: &root,
                script_access: AccessLevel::Public,
                qualified_script_name: Some("main.main"),
                module_name: Some("main"),
                visible_types: &visible_types,
                visible_functions: &BTreeMap::new(),
                visible_module_vars: &BTreeMap::new(),
                visible_module_consts: &BTreeMap::new(),
                all_script_access: &known_scripts,
                invoke_all_functions: &BTreeMap::new(),
            })
            .expect_err("compile should fail");
            assert_eq!(error.code, expected_code);
        };

        // <text> with invalid enum member - triggers line 496 error path
        compile_error(
            r#"<script name="main"><text>Value: ${Status.Invalid}</text></script>"#,
            "ENUM_LITERAL_MEMBER_UNKNOWN",
        );

        // <debug> with invalid enum member - triggers line 519 error path
        compile_error(
            r#"<script name="main"><debug>Status: ${Status.Invalid}</debug></script>"#,
            "ENUM_LITERAL_MEMBER_UNKNOWN",
        );

        // <choice text="..."> with invalid enum member - triggers line 655 error path
        compile_error(
            r#"<script name="main"><choice text="${Status.Invalid}"><option text="a"/></choice></script>"#,
            "ENUM_LITERAL_MEMBER_UNKNOWN",
        );

        // <option text="..."> with invalid enum member - triggers line 714 error path
        compile_error(
            r#"<script name="main"><choice text="Pick"><option text="${Status.Invalid}"/></choice></script>"#,
            "ENUM_LITERAL_MEMBER_UNKNOWN",
        );

        // Test 2: <text> node with invalid script literal - triggers line 496 error path
        let all_scripts = BTreeMap::new();
        let compile_error_for_literal = |xml: &str, expected_code: &str| {
            let root = parse_xml_document(xml).expect("xml").root;
            let error = compile_script(CompileScriptOptions {
                script_path: "main.xml",
                root: &root,
                script_access: AccessLevel::Public,
                qualified_script_name: Some("main.main"),
                module_name: Some("main"),
                visible_types: &BTreeMap::new(),
                visible_functions: &BTreeMap::new(),
                visible_module_vars: &BTreeMap::new(),
                visible_module_consts: &BTreeMap::new(),
                all_script_access: &all_scripts,
                invoke_all_functions: &BTreeMap::new(),
            })
            .expect_err("compile should fail");
            assert_eq!(error.code, expected_code);
        };

        // <text> with invalid script literal (short form without module) - triggers line 496
        compile_error_for_literal(
            r#"<script name="main"><text>@missing</text></script>"#,
            "XML_SCRIPT_TARGET_NOT_FOUND",
        );

        // <debug> with invalid script literal - triggers line 519
        compile_error_for_literal(
            r#"<script name="main"><debug>@missing</debug></script>"#,
            "XML_SCRIPT_TARGET_NOT_FOUND",
        );

        // <choice text="..."> with invalid script literal - triggers line 655
        compile_error_for_literal(
            r#"<script name="main"><choice text="@missing"><option text="a"/></choice></script>"#,
            "XML_SCRIPT_TARGET_NOT_FOUND",
        );

        // <dynamic-options><option text="..."> with invalid enum member - triggers line 804
        compile_error(
            r#"<script name="main"><choice text="Pick"><dynamic-options array="arr" item="it"><option text="${Status.Invalid}"/></dynamic-options></choice></script>"#,
            "ENUM_LITERAL_MEMBER_UNKNOWN",
        );

        // Test line 1570: dynamic-options with invalid Rhai syntax in array attribute
        // This triggers normalize_attribute_expression_literals error path
        compile_error(
            r#"<script name="main"><choice text="Pick"><dynamic-options array="if if if" item="it"><option text="a"/></dynamic-options></choice></script>"#,
            "XML_RHAI_SYNTAX_INVALID",
        );
    }

    #[test]
    fn script_target_validation_helpers_cover_literal_and_variable_paths() {
        let span = SourceSpan::synthetic();
        assert_eq!(
            qualify_script_literal_name("main.next", Some("main"), &span).expect("qualified"),
            "main.next"
        );
        assert_eq!(
            qualify_script_literal_name("next", Some("main"), &span).expect("short with module"),
            "main.next"
        );
        let no_module_error = qualify_script_literal_name("next", None, &span)
            .expect_err("short literal without module should fail");
        assert_eq!(no_module_error.code, "XML_SCRIPT_TARGET_INVALID");

        // Test parse_script_literal_name error paths (lines 94-96, 108-110)
        // First char not alphabetic or underscore: "@1abc", "@!abc"
        assert!(
            parse_script_literal_name(&"@1abc".chars().collect::<Vec<_>>(), 0).is_none(),
            "digit first char should return None"
        );
        assert!(
            parse_script_literal_name(&"@!abc".chars().collect::<Vec<_>>(), 0).is_none(),
            "special char first should return None"
        );
        // Dot followed by non-alphanumeric: "@main.", "@main.!abc"
        assert!(
            parse_script_literal_name(&"@main.".chars().collect::<Vec<_>>(), 0).is_none(),
            "dot at end should return None"
        );
        assert!(
            parse_script_literal_name(&"@main.!abc".chars().collect::<Vec<_>>(), 0).is_none(),
            "dot followed by special char should return None"
        );

        // Test line 94: @ at end of string (no chars after @)
        assert!(
            parse_script_literal_name(&"@".chars().collect::<Vec<_>>(), 0).is_none(),
            "@ at end should return None"
        );

        // Test line 172: @ followed by invalid script name (not at boundary)
        // "@abc" at word boundary - parse succeeds but invalid chars after @
        let invalid_at_boundary = normalize_and_validate_script_literals_in_expression(
            "x = @1abc;", // @ followed by digit (invalid start)
            &span,
            Some("main"),
            None,
        )
        .expect("@ followed by digit should be processed");
        // @ should stay as-is because it's not a valid script literal start
        assert!(
            invalid_at_boundary.contains("@1abc"),
            "@ with digit should stay raw"
        );

        // Test string escape sequences (lines 145-147) and invalid @ suffix (line 172)
        // String with escape sequence: "hello\"world"
        let with_escape = normalize_and_validate_script_literals_in_expression(
            r#"msg = "hello\"world";"#,
            &span,
            Some("main"),
            None,
        )
        .expect("string with escape should be processed");
        assert!(
            with_escape.contains(r#"hello\"world"#),
            "escape should be preserved"
        );

        // String with @ at end that's not a script literal
        let at_in_string = normalize_and_validate_script_literals_in_expression(
            r#"msg = "hello@";"#,
            &span,
            Some("main"),
            None,
        )
        .expect("@ in string should stay as-is");
        assert!(
            at_in_string.contains(r#""hello@""#),
            "@ in string should be preserved"
        );

        let access_map = BTreeMap::from([
            ("main.next".to_string(), AccessLevel::Public),
            ("shared.hidden".to_string(), AccessLevel::Private),
            ("battle-loop.main".to_string(), AccessLevel::Public),
        ]);
        validate_script_literal_access("main.next", &access_map, Some("main"), &span)
            .expect("public literal should pass");
        let not_found =
            validate_script_literal_access("missing.next", &access_map, Some("main"), &span)
                .expect_err("unknown literal should fail");
        assert_eq!(not_found.code, "XML_SCRIPT_TARGET_NOT_FOUND");
        let denied =
            validate_script_literal_access("shared.hidden", &access_map, Some("main"), &span)
                .expect_err("private cross-module should fail");
        assert_eq!(denied.code, "XML_SCRIPT_TARGET_ACCESS_DENIED");

        let normalized = normalize_and_validate_script_literals_in_expression(
            "dst = @main.next; alt = @battle-loop.main;",
            &span,
            Some("main"),
            Some(&access_map),
        )
        .expect("script literals in expression should validate");
        assert_eq!(normalized, "dst = @main.next; alt = @battle-loop.main;");
        let boundary = normalize_and_validate_script_literals_in_expression(
            "obj.@next + @next",
            &span,
            Some("main"),
            Some(&access_map),
        )
        .expect("non-boundary script token should stay raw");
        assert_eq!(boundary, "obj.@next + @main.next");

        let short_literal_error = normalize_and_validate_script_literals_in_expression(
            "dst = @next;",
            &span,
            None,
            Some(&access_map),
        )
        .expect_err("short literal without module should fail in expressions");
        assert_eq!(short_literal_error.code, "XML_SCRIPT_TARGET_INVALID");

        let node = xml_element("call", &[("script", "@main.next")], Vec::new());
        let literal_target = parse_script_target_attr(
            "@battle-loop.main",
            &node,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &access_map,
            Some("main"),
        )
        .expect("hyphenated literal target should parse");
        assert!(matches!(
            literal_target,
            ScriptTarget::Literal { script_name } if script_name == "battle-loop.main"
        ));
        let short_literal_target_error = parse_script_target_attr(
            "@next",
            &node,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &access_map,
            None,
        )
        .expect_err("short literal target without module should fail");
        assert_eq!(short_literal_target_error.code, "XML_SCRIPT_TARGET_INVALID");

        let template_error = parse_script_target_attr(
            "${next}",
            &node,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &access_map,
            Some("main"),
        )
        .expect_err("template target should be rejected");
        assert_eq!(template_error.code, "XML_SCRIPT_TARGET_TEMPLATE_REMOVED");

        let invalid_literal = parse_script_target_attr(
            "@bad.",
            &node,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &access_map,
            Some("main"),
        )
        .expect_err("invalid literal should fail");
        assert_eq!(invalid_literal.code, "XML_SCRIPT_TARGET_INVALID");

        let invalid_plain = parse_script_target_attr(
            "main.next",
            &node,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &access_map,
            Some("main"),
        )
        .expect_err("plain dotted target should fail");
        assert_eq!(invalid_plain.code, "XML_SCRIPT_TARGET_INVALID");

        let unknown_var = parse_script_target_attr(
            "nextScene",
            &node,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &access_map,
            Some("main"),
        )
        .expect_err("unknown script variable should fail");
        assert_eq!(unknown_var.code, "XML_SCRIPT_TARGET_VAR_UNKNOWN");

        let typed_vars = BTreeMap::from([(
            "nextScene".to_string(),
            ScriptType::Primitive {
                name: "string".to_string(),
            },
        )]);
        let bad_var_type = parse_script_target_attr(
            "nextScene",
            &node,
            &typed_vars,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &access_map,
            Some("main"),
        )
        .expect_err("non-script variable should fail");
        assert_eq!(bad_var_type.code, "XML_SCRIPT_TARGET_VAR_TYPE");

        let script_vars = BTreeMap::from([("nextScene".to_string(), ScriptType::Script)]);
        let var_target = parse_script_target_attr(
            "nextScene",
            &node,
            &script_vars,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &access_map,
            Some("main"),
        )
        .expect("script variable target should pass");
        assert!(matches!(
            var_target,
            ScriptTarget::Variable { var_name } if var_name == "nextScene"
        ));
    }

    #[test]
    fn script_literal_validation_is_applied_in_all_supported_expression_positions() {
        let root = parse_xml_document(
            r#"
<script name="main">
  <temp name="dst" type="script">@main.next</temp>
  <code>dst = @main.next;</code>
  <if when="@main.next != ''"><text>a</text><else><text>b</text></else></if>
  <while when="@main.next != ''"><text>w</text></while>
  <choice text="pick">
    <option text="A" when="@main.next != ''"><text>x</text></option>
    <dynamic-options array="[1]" item="it">
      <option text="${it}" when="@main.next != ''"><text>d</text></option>
    </dynamic-options>
  </choice>
  <call script="@main.next" args="@main.next"/>
  <goto script="@main.next" args="@main.next"/>
</script>
"#,
        )
        .expect("xml")
        .root;

        let all_scripts = BTreeMap::from([("main.next".to_string(), AccessLevel::Public)]);
        let compiled = compile_script(CompileScriptOptions {
            script_path: "main.xml",
            root: &root,
            script_access: AccessLevel::Public,
            qualified_script_name: Some("main.main"),
            module_name: Some("main"),
            visible_types: &BTreeMap::new(),
            visible_functions: &BTreeMap::new(),
            visible_module_vars: &BTreeMap::new(),
            visible_module_consts: &BTreeMap::new(),
            all_script_access: &all_scripts,
            invoke_all_functions: &BTreeMap::new(),
        })
        .expect("compile should pass");

        let root_group = compiled
            .groups
            .get(&compiled.root_group_id)
            .expect("root group");
        assert!(!root_group.nodes.is_empty());
    }

    #[test]
    fn enum_literals_in_attribute_expressions_are_normalized_for_rhai() {
        let root = parse_xml_document(
            r#"
<script name="main">
  <if when="ids.LocationId.A == 'A'"><text>x</text></if>
  <call script="@main.next" args="ids.LocationId.A"/>
  <goto script="@main.next" args="ids.LocationId.A"/>
</script>
"#,
        )
        .expect("xml")
        .root;
        let all_scripts = BTreeMap::from([("main.next".to_string(), AccessLevel::Public)]);
        let visible_types = BTreeMap::from([(
            "ids.LocationId".to_string(),
            ScriptType::Enum {
                type_name: "LocationId".to_string(),
                members: vec!["A".to_string(), "B".to_string()],
            },
        )]);
        let compiled = compile_script(CompileScriptOptions {
            script_path: "main.xml",
            root: &root,
            script_access: AccessLevel::Public,
            qualified_script_name: Some("main.main"),
            module_name: Some("main"),
            visible_types: &visible_types,
            visible_functions: &BTreeMap::new(),
            visible_module_vars: &BTreeMap::new(),
            visible_module_consts: &BTreeMap::new(),
            all_script_access: &all_scripts,
            invoke_all_functions: &BTreeMap::new(),
        })
        .expect("compile should pass");
        let root_group = compiled
            .groups
            .get(&compiled.root_group_id)
            .expect("root group");
        assert!(
            root_group.nodes.iter().any(|node| matches!(
                node,
                ScriptNode::If { when_expr, .. } if when_expr == "\"A\" == \"A\""
            )),
            "if when expression should rewrite enum literal into normalized Rhai source"
        );
        assert!(
            root_group.nodes.iter().any(|node| matches!(
                node,
                ScriptNode::Call { args, .. } if args.first().map(|arg| arg.value_expr.as_str()) == Some("\"A\"")
            )),
            "call arg should rewrite enum literal into normalized Rhai source"
        );
        assert!(
            root_group.nodes.iter().any(|node| matches!(
                node,
                ScriptNode::Goto { args, .. } if args.first().map(|arg| arg.value_expr.as_str()) == Some("\"A\"")
            )),
            "goto arg should rewrite enum literal into normalized Rhai source"
        );
    }

    #[test]
    fn script_literal_validation_error_paths_are_reported_at_each_node_kind() {
        let compile_error = |xml: &str, expected_code: &str| {
            let root = parse_xml_document(xml).expect("xml").root;
            let known_scripts = BTreeMap::from([("main.next".to_string(), AccessLevel::Public)]);
            let error = compile_script(CompileScriptOptions {
                script_path: "main.xml",
                root: &root,
                script_access: AccessLevel::Public,
                qualified_script_name: Some("main.main"),
                module_name: Some("main"),
                visible_types: &BTreeMap::new(),
                visible_functions: &BTreeMap::new(),
                visible_module_vars: &BTreeMap::new(),
                visible_module_consts: &BTreeMap::new(),
                all_script_access: &known_scripts,
                invoke_all_functions: &BTreeMap::new(),
            })
            .expect_err("compile should fail");
            assert_eq!(error.code, expected_code);
        };

        compile_error(
            r#"<script name="main"><temp name="dst" type="script">@missing.next</temp></script>"#,
            "XML_SCRIPT_TARGET_NOT_FOUND",
        );
        compile_error(
            r#"<script name="main"><temp name="dst" type="script">"main.next"</temp></script>"#,
            "XML_SCRIPT_ASSIGN_STRING_FORBIDDEN",
        );
        compile_error(
            r#"<script name="main"><code>dst = @missing.next;</code></script>"#,
            "XML_SCRIPT_TARGET_NOT_FOUND",
        );
        compile_error(
            r#"<script name="main"><if when="@missing.next != ''"><text>x</text></if></script>"#,
            "XML_SCRIPT_TARGET_NOT_FOUND",
        );
        compile_error(
            r#"<script name="main"><while when="@missing.next != ''"><text>x</text></while></script>"#,
            "XML_SCRIPT_TARGET_NOT_FOUND",
        );
        compile_error(
            r#"<script name="main"><choice text="c"><option text="a" when="@missing.next != ''"><text>x</text></option></choice></script>"#,
            "XML_SCRIPT_TARGET_NOT_FOUND",
        );
        compile_error(
            r#"<script name="main"><choice text="c"><dynamic-options array="[1]" item="it"><option text="${it}" when="@missing.next != ''"><text>x</text></option></dynamic-options></choice></script>"#,
            "XML_SCRIPT_TARGET_NOT_FOUND",
        );
        compile_error(
            r#"<script name="main"><call script="@main.next" args="@missing.next"/></script>"#,
            "XML_SCRIPT_TARGET_NOT_FOUND",
        );
        compile_error(
            r#"<script name="main"><goto script="@main.next" args="@missing.next"/></script>"#,
            "XML_SCRIPT_TARGET_NOT_FOUND",
        );
        compile_error(
            r#"<script name="main"><call script="@missing.next"/></script>"#,
            "XML_SCRIPT_TARGET_NOT_FOUND",
        );
        compile_error(
            r#"<script name="main"><goto script="@missing.next"/></script>"#,
            "XML_SCRIPT_TARGET_NOT_FOUND",
        );
    }

    #[test]
    fn temp_var_enum_type_requires_init_or_valid_member() {
        // Test lines 1085-1105: enum type temp variable validation
        use sl_core::ScriptType;

        let compile_error_with_enum = |xml: &str, expected_code: &str| {
            let root = parse_xml_document(xml).expect("xml").root;
            let known_scripts = BTreeMap::new();
            let enum_type = ScriptType::Enum {
                type_name: "Status".to_string(),
                members: vec!["Active".to_string(), "Inactive".to_string()],
            };
            let mut visible_types = BTreeMap::new();
            visible_types.insert("Status".to_string(), enum_type);

            let error = compile_script(CompileScriptOptions {
                script_path: "main.xml",
                root: &root,
                script_access: AccessLevel::Public,
                qualified_script_name: Some("main.main"),
                module_name: Some("main"),
                visible_types: &visible_types,
                visible_functions: &BTreeMap::new(),
                visible_module_vars: &BTreeMap::new(),
                visible_module_consts: &BTreeMap::new(),
                all_script_access: &known_scripts,
                invoke_all_functions: &BTreeMap::new(),
            })
            .expect_err("compile should fail");
            assert_eq!(error.code, expected_code);
        };

        let compile_ok_with_enum = |xml: &str| {
            let root = parse_xml_document(xml).expect("xml").root;
            let known_scripts = BTreeMap::new();
            let enum_type = ScriptType::Enum {
                type_name: "Status".to_string(),
                members: vec!["Active".to_string(), "Inactive".to_string()],
            };
            let mut visible_types = BTreeMap::new();
            visible_types.insert("Status".to_string(), enum_type);

            compile_script(CompileScriptOptions {
                script_path: "main.xml",
                root: &root,
                script_access: AccessLevel::Public,
                qualified_script_name: Some("main.main"),
                module_name: Some("main"),
                visible_types: &visible_types,
                visible_functions: &BTreeMap::new(),
                visible_module_vars: &BTreeMap::new(),
                visible_module_consts: &BTreeMap::new(),
                all_script_access: &known_scripts,
                invoke_all_functions: &BTreeMap::new(),
            })
            .expect("compile should succeed");
        };

        // Test temp enum without initializer (line 1085)
        compile_error_with_enum(
            r#"<script name="main"><temp name="s" type="Status"/></script>"#,
            "ENUM_INIT_REQUIRED",
        );

        // Test temp enum with invalid member (line 1098)
        compile_error_with_enum(
            r#"<script name="main"><temp name="s" type="Status">Status.Unknown</temp></script>"#,
            "ENUM_LITERAL_MEMBER_UNKNOWN",
        );

        // Test temp enum with valid member - success path (lines 1097-1105)
        compile_ok_with_enum(
            r#"<script name="main"><temp name="s" type="Status">Status.Active</temp></script>"#,
        );

        // enum-key map initializer uses same literal shape as string-key map, but keys are validated
        compile_error_with_enum(
            r##"<script name="main"><temp name="tbl" type="#{Status=>int}">#{Unknown: 1}</temp></script>"##,
            "ENUM_MAP_KEY_UNKNOWN",
        );
        compile_ok_with_enum(
            r##"<script name="main"><temp name="tbl" type="#{Status=>int}">#{Active: 1}</temp></script>"##,
        );
    }

    #[test]
    fn rhai_compile_error_paths_are_covered() {
        // Test lines 75-81: Rhai compilation error handling
        let span = SourceSpan::synthetic();
        let empty_map = BTreeMap::new();

        // Test invalid Rhai syntax - should trigger error path at line 75
        let error = preprocess_and_compile_rhai_source(
            "this is not valid rhai !!!",
            &span,
            "test context",
            RhaiInputMode::CodeBlock,
            RhaiCompileTarget::CodeBlock,
            &empty_map,
            &empty_map,
        )
        .expect_err("invalid rhai should fail");
        assert_eq!(error.code, "XML_RHAI_SYNTAX_INVALID");

        // Test invalid attribute expression - should also trigger error path
        let error2 = preprocess_and_compile_rhai_source(
            "@#$%^&*(",
            &span,
            "test context",
            RhaiInputMode::AttributeExpr,
            RhaiCompileTarget::Expression,
            &empty_map,
            &empty_map,
        )
        .expect_err("invalid expression should fail");
        assert_eq!(error2.code, "XML_RHAI_SYNTAX_INVALID");
    }
}
