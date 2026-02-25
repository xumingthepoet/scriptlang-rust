fn compile_script(
    script_path: &str,
    root: &XmlElementNode,
    visible_types: &BTreeMap<String, ScriptType>,
    visible_functions: &BTreeMap<String, FunctionDecl>,
    visible_json_globals: &[String],
    all_json_symbols: &BTreeSet<String>,
) -> Result<ScriptIr, ScriptLangError> {
    if root.name != "script" {
        return Err(ScriptLangError::with_span(
            "XML_ROOT_INVALID",
            "Script file root must be <script>.",
            root.location.clone(),
        ));
    }

    let script_name = get_required_non_empty_attr(root, "name")?;
    assert_name_not_reserved(&script_name, "script", root.location.clone())?;

    let params = parse_script_args(root, visible_types)?;
    validate_reserved_prefix_in_user_var_declarations(root)?;

    let mut reserved_names = params
        .iter()
        .map(|param| param.name.clone())
        .collect::<Vec<_>>();
    reserved_names.sort();

    let expanded_root = expand_script_macros(root, &reserved_names)?;

    let mut builder = GroupBuilder::new(script_path);
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

    let ir = ScriptIr {
        script_path: script_path.to_string(),
        script_name,
        params,
        root_group_id,
        groups: builder.groups,
        visible_json_globals: visible_json_globals.to_vec(),
        visible_functions: visible_functions.clone(),
    };

    validate_json_symbol_visibility(&ir, all_json_symbols)?;
    Ok(ir)
}

fn validate_json_symbol_visibility(
    script_ir: &ScriptIr,
    all_json_symbols: &BTreeSet<String>,
) -> Result<(), ScriptLangError> {
    let visible_json = script_ir
        .visible_json_globals
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let hidden_json = all_json_symbols
        .difference(&visible_json)
        .cloned()
        .collect::<BTreeSet<_>>();

    if hidden_json.is_empty() {
        return Ok(());
    }

    let function_roots = collect_visible_function_roots(&script_ir.visible_functions);
    let mut script_allowed = collect_script_declared_names(script_ir);
    script_allowed.extend(function_roots.iter().cloned());
    script_allowed.insert("random".to_string());

    for group in script_ir.groups.values() {
        for node in &group.nodes {
            match node {
                ScriptNode::Text {
                    value, location, ..
                } => {
                    for expr in extract_text_interpolations(value) {
                        ensure_no_hidden_json_symbol(
                            &expr,
                            &hidden_json,
                            &script_allowed,
                            &script_ir.script_name,
                            "text interpolation",
                            location,
                        )?;
                    }
                }
                ScriptNode::Code { code, location, .. } => {
                    ensure_no_hidden_json_symbol(
                        code,
                        &hidden_json,
                        &script_allowed,
                        &script_ir.script_name,
                        "code block",
                        location,
                    )?;
                }
                ScriptNode::Var { declaration, .. } => {
                    if let Some(expr) = &declaration.initial_value_expr {
                        ensure_no_hidden_json_symbol(
                            expr,
                            &hidden_json,
                            &script_allowed,
                            &script_ir.script_name,
                            "var initializer",
                            &declaration.location,
                        )?;
                    }
                }
                ScriptNode::If {
                    when_expr,
                    location,
                    ..
                } => {
                    ensure_no_hidden_json_symbol(
                        when_expr,
                        &hidden_json,
                        &script_allowed,
                        &script_ir.script_name,
                        "if condition",
                        location,
                    )?;
                }
                ScriptNode::While {
                    when_expr,
                    location,
                    ..
                } => {
                    ensure_no_hidden_json_symbol(
                        when_expr,
                        &hidden_json,
                        &script_allowed,
                        &script_ir.script_name,
                        "while condition",
                        location,
                    )?;
                }
                ScriptNode::Choice { options, .. } => {
                    for option in options {
                        if let Some(when_expr) = &option.when_expr {
                            ensure_no_hidden_json_symbol(
                                when_expr,
                                &hidden_json,
                                &script_allowed,
                                &script_ir.script_name,
                                "choice option condition",
                                &option.location,
                            )?;
                        }
                    }
                }
                ScriptNode::Call { args, location, .. }
                | ScriptNode::Return { args, location, .. } => {
                    for arg in args {
                        ensure_no_hidden_json_symbol(
                            &arg.value_expr,
                            &hidden_json,
                            &script_allowed,
                            &script_ir.script_name,
                            "call/return argument",
                            location,
                        )?;
                    }
                }
                ScriptNode::Input { .. }
                | ScriptNode::Break { .. }
                | ScriptNode::Continue { .. } => {}
            }
        }
    }

    for (name, decl) in &script_ir.visible_functions {
        if !name.contains('.') {
            continue;
        }

        let mut function_allowed = BTreeSet::new();
        function_allowed.extend(function_roots.iter().cloned());
        function_allowed.insert("random".to_string());
        function_allowed.insert(decl.return_binding.name.clone());
        for param in &decl.params {
            function_allowed.insert(param.name.clone());
        }
        function_allowed.extend(extract_local_bindings(&decl.code));

        ensure_no_hidden_json_symbol(
            &decl.code,
            &hidden_json,
            &function_allowed,
            &script_ir.script_name,
            "defs function code",
            &decl.location,
        )?;
    }

    Ok(())
}

fn collect_script_declared_names(script_ir: &ScriptIr) -> BTreeSet<String> {
    let mut out = script_ir
        .params
        .iter()
        .map(|param| param.name.clone())
        .collect::<BTreeSet<_>>();

    for group in script_ir.groups.values() {
        for node in &group.nodes {
            if let ScriptNode::Var { declaration, .. } = node {
                out.insert(declaration.name.clone());
            }
        }
    }
    out
}

fn collect_visible_function_roots(
    visible_functions: &BTreeMap<String, FunctionDecl>,
) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for name in visible_functions.keys() {
        let root = name.split('.').next().unwrap_or(name);
        out.insert(root.to_string());
    }
    out
}

fn extract_text_interpolations(template: &str) -> Vec<String> {
    text_interpolation_regex()
        .captures_iter(template)
        .filter_map(|caps| caps.get(1).map(|entry| entry.as_str().trim().to_string()))
        .filter(|expr| !expr.is_empty())
        .collect()
}

fn extract_local_bindings(source: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for caps in local_binding_regex().captures_iter(source) {
        if let Some(name) = caps.get(1) {
            out.insert(name.as_str().to_string());
        }
    }
    out
}

fn text_interpolation_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"\$\{([^{}]+)\}").expect("template regex must compile"))
}

fn local_binding_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"\b(?:let|const)\s+([$A-Za-z_][$0-9A-Za-z_]*)")
            .expect("local bind regex")
    })
}

fn ensure_no_hidden_json_symbol(
    source: &str,
    hidden_json: &BTreeSet<String>,
    allowed_names: &BTreeSet<String>,
    script_name: &str,
    context: &str,
    location: &SourceSpan,
) -> Result<(), ScriptLangError> {
    if let Some(symbol) = find_hidden_json_symbol(source, hidden_json, allowed_names) {
        return Err(ScriptLangError::with_span(
            "JSON_SYMBOL_NOT_VISIBLE",
            format!(
                "JSON symbol \"{}\" is referenced in {} of script \"{}\" but is not visible via include.",
                symbol, context, script_name
            ),
            location.clone(),
        ));
    }
    Ok(())
}

fn find_hidden_json_symbol(
    source: &str,
    hidden_json: &BTreeSet<String>,
    allowed_names: &BTreeSet<String>,
) -> Option<String> {
    let sanitized = sanitize_rhai_source(source);
    hidden_json
        .iter()
        .filter(|symbol| !allowed_names.contains(*symbol))
        .find(|symbol| contains_root_identifier(&sanitized, symbol))
        .cloned()
}

#[cfg(test)]
mod json_symbols_tests {
    use super::*;
    use crate::compiler_test_support::*;

    #[test]
    fn parse_json_symbol_and_global_collection_errors_are_reported() {
        assert_eq!(
            parse_json_global_symbol("game.json").expect("symbol"),
            "game"
        );
        let invalid = parse_json_global_symbol("bad-name.json").expect_err("invalid");
        assert_eq!(invalid.code, "JSON_SYMBOL_INVALID");
    
        let reserved = parse_json_global_symbol("__sl_reserved.json").expect_err("reserved");
        assert_eq!(reserved.code, "NAME_RESERVED_PREFIX");
    
        let duplicate = compile_project_bundle_from_xml_map(&map(&[
            ("a/x.json", r#"{"v":1}"#),
            ("b/x.json", r#"{"v":2}"#),
            (
                "main.script.xml",
                r#"
    <!-- include: a/x.json -->
    <!-- include: b/x.json -->
    <script name="main"><text>x</text></script>
    "#,
            ),
        ]))
        .expect_err("duplicate symbol should fail");
        assert_eq!(duplicate.code, "JSON_SYMBOL_DUPLICATE");
    
        let missing_sources = BTreeMap::from([(
            "broken.json".to_string(),
            SourceFile {
                kind: SourceKind::Json,
                includes: Vec::new(),
                xml_root: None,
                json_value: None,
            },
        )]);
        let missing = collect_global_json(&missing_sources).expect_err("missing value");
        assert_eq!(missing.code, "JSON_MISSING_VALUE");
    }

    #[test]
    fn json_symbol_visibility_helpers_cover_context_edges() {
        let hidden_json = BTreeSet::from(["game".to_string()]);
        let allowed = BTreeSet::new();
    
        assert_eq!(
            find_hidden_json_symbol("value = game.hp;", &hidden_json, &allowed),
            Some("game".to_string())
        );
        assert_eq!(
            find_hidden_json_symbol("value = obj.game;", &hidden_json, &allowed),
            None
        );
        assert_eq!(
            find_hidden_json_symbol("value = #{game: 1};", &hidden_json, &allowed),
            None
        );
        assert_eq!(
            find_hidden_json_symbol(r#"value = "game"; // game"#, &hidden_json, &allowed),
            None
        );
        assert_eq!(
            find_hidden_json_symbol("/* game */ value = 1;", &hidden_json, &allowed),
            None
        );
    
        let locals = extract_local_bindings("let game = 1; const score = game + 1;");
        assert!(locals.contains("game"));
        assert!(locals.contains("score"));
    
        let allowed_game = BTreeSet::from(["game".to_string()]);
        assert_eq!(
            find_hidden_json_symbol("value = game.hp;", &hidden_json, &allowed_game),
            None
        );
    }

    #[test]
    fn compile_bundle_rejects_hidden_json_usage_without_include_in_script() {
        let files = map(&[
            ("game.json", r#"{ "hp": 5 }"#),
            (
                "main.script.xml",
                r#"<script name="main"><text>${game.hp}</text></script>"#,
            ),
        ]);
    
        let error = compile_project_bundle_from_xml_map(&files)
            .expect_err("missing json include should fail at compile time");
        assert_eq!(error.code, "JSON_SYMBOL_NOT_VISIBLE");
    }

    #[test]
    fn compile_bundle_rejects_hidden_json_usage_without_include_in_defs() {
        let files = map(&[
            ("game.json", r#"{ "hp": 5 }"#),
            (
                "shared.defs.xml",
                r#"
    <defs name="shared">
      <function name="boost" return="int:out">
        out = game.hp;
      </function>
    </defs>
    "#,
            ),
            (
                "main.script.xml",
                r#"
    <!-- include: shared.defs.xml -->
    <script name="main">
      <var name="hp" type="int">1</var>
      <code>hp = shared.boost();</code>
    </script>
    "#,
            ),
        ]);
    
        let error = compile_project_bundle_from_xml_map(&files)
            .expect_err("defs code should fail when json is not visible");
        assert_eq!(error.code, "JSON_SYMBOL_NOT_VISIBLE");
    }

    #[test]
    fn compile_bundle_allows_visible_or_shadowed_json_symbols() {
        let visible = map(&[
            ("game.json", r#"{ "hp": 5 }"#),
            (
                "main.script.xml",
                r#"
    <!-- include: game.json -->
    <script name="main">
      <text>${game.hp}</text>
    </script>
    "#,
            ),
        ]);
        compile_project_bundle_from_xml_map(&visible).expect("visible json symbol should compile");
    
        let shadowed = map(&[
            ("game.json", r#"{ "hp": 5 }"#),
            (
                "main.script.xml",
                r#"
    <script name="main">
      <var name="game" type="int">1</var>
      <code>game = game + 1;</code>
    </script>
    "#,
            ),
        ]);
        compile_project_bundle_from_xml_map(&shadowed)
            .expect("shadowed local name should compile without include");
    }

    #[test]
    fn json_symbol_visibility_validation_covers_all_script_node_paths() {
        let files = map(&[
            ("game.json", r#"{ "hp": 5 }"#),
            ("secret.json", r#"{ "v": 9 }"#),
            (
                "helpers.defs.xml",
                r#"
    <defs name="helpers">
      <function name="boost" args="int:x" return="int:out">
        let local = x + game.hp;
        out = local;
      </function>
    </defs>
    "#,
            ),
            (
                "next.script.xml",
                r#"
    <script name="next" args="int:n">
      <text>${n}</text>
    </script>
    "#,
            ),
            (
                "main.script.xml",
                r#"
    <!-- include: game.json -->
    <!-- include: helpers.defs.xml -->
    <script name="main">
      <var name="hp" type="int">1</var>
      <var name="name" type="string">&quot;A&quot;</var>
      <if when="hp > 0">
        <text>ok</text>
      </if>
      <while when="hp > 0">
        <code>hp = hp - 1;</code>
        <continue/>
        <break/>
      </while>
      <choice text="c">
        <option text="o1" when="hp >= 0">
          <text>x</text>
        </option>
      </choice>
      <input var="name" text="in"/>
      <code>hp = helpers.boost(hp);</code>
      <call script="next" args="hp"/>
      <return script="next" args="hp"/>
    </script>
    "#,
            ),
        ]);
    
        compile_project_bundle_from_xml_map(&files)
            .expect("validation should pass when hidden json is not referenced");
    }

}
