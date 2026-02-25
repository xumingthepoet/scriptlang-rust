#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn map(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    #[test]
    fn compile_basic_script_project() {
        let files = map(&[(
            "main.script.xml",
            r#"
<script name="main">
  <text>Hello</text>
  <choice text="Pick">
    <option text="A"><text>A1</text></option>
  </choice>
</script>
"#,
        )]);

        let result = compile_project_bundle_from_xml_map(&files).expect("project should compile");
        assert!(result.scripts.contains_key("main"));
        let main = result.scripts.get("main").expect("main script");
        assert!(!main.groups.is_empty());
    }

    #[test]
    fn loop_macro_expands_to_var_and_while() {
        let files = map(&[(
            "main.script.xml",
            r#"
<script name="main">
  <var name="i" type="int">0</var>
  <loop times="2">
    <code>i = i + 1;</code>
  </loop>
</script>
"#,
        )]);

        let result = compile_project_bundle_from_xml_map(&files).expect("project should compile");
        let main = result.scripts.get("main").expect("main script");
        let root = main.groups.get(&main.root_group_id).expect("root group");
        assert!(root
            .nodes
            .iter()
            .any(|node| matches!(node, ScriptNode::Var { .. })));
        assert!(root
            .nodes
            .iter()
            .any(|node| matches!(node, ScriptNode::While { .. })));
    }

    fn read_sources_recursive(
        root: &Path,
        current: &Path,
        out: &mut BTreeMap<String, String>,
    ) -> Result<(), std::io::Error> {
        for entry in fs::read_dir(current)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                read_sources_recursive(root, &path, out)?;
                continue;
            }
            let relative = path
                .strip_prefix(root)
                .expect("path should be under root")
                .to_string_lossy()
                .replace('\\', "/");
            let text = fs::read_to_string(&path)?;
            out.insert(relative, text);
        }
        Ok(())
    }

    fn sources_from_example_dir(name: &str) -> BTreeMap<String, String> {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = manifest_dir
            .join("..")
            .join("..")
            .join("examples")
            .join("scripts-rhai")
            .join(name);
        let mut out = BTreeMap::new();
        read_sources_recursive(&root, &root, &mut out).expect("example sources should read");
        out
    }

    #[test]
    fn compile_bundle_supports_all_example_scenarios() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let examples_root = manifest_dir
            .join("..")
            .join("..")
            .join("examples")
            .join("scripts-rhai");

        let mut dirs = fs::read_dir(&examples_root)
            .expect("examples root should exist")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect::<Vec<_>>();
        dirs.sort();

        assert!(!dirs.is_empty(), "examples should not be empty");

        for directory in dirs {
            let mut files = BTreeMap::new();
            read_sources_recursive(&directory, &directory, &mut files)
                .expect("read sources should pass");
            let compiled =
                compile_project_bundle_from_xml_map(&files).expect("example should compile");
            assert!(!compiled.scripts.is_empty());
        }
    }

    #[test]
    fn compile_scripts_from_xml_map_returns_script_only_bundle() {
        let files = sources_from_example_dir("15-entry-override-recursive");
        let scripts = compile_project_scripts_from_xml_map(&files).expect("compile should pass");
        assert!(scripts.contains_key("main"));
        assert!(scripts.contains_key("alt"));
    }

    #[test]
    fn compile_bundle_rejects_unsupported_source_extension() {
        let files = BTreeMap::from([("x.txt".to_string(), "bad".to_string())]);
        let error = compile_project_bundle_from_xml_map(&files)
            .expect_err("unsupported extension should fail");
        assert_eq!(error.code, "SOURCE_KIND_UNSUPPORTED");
    }

    #[test]
    fn compile_bundle_rejects_missing_include_and_cycle() {
        let missing_include = map(&[(
            "main.script.xml",
            r#"
<!-- include: missing.script.xml -->
<script name="main"></script>
"#,
        )]);
        let missing = compile_project_bundle_from_xml_map(&missing_include)
            .expect_err("missing include should fail");
        assert_eq!(missing.code, "INCLUDE_NOT_FOUND");

        let cycle = map(&[
            (
                "a.script.xml",
                r#"
<!-- include: b.script.xml -->
<script name="a"></script>
"#,
            ),
            (
                "b.script.xml",
                r#"
<!-- include: a.script.xml -->
<script name="b"></script>
"#,
            ),
        ]);
        let cycle_error =
            compile_project_bundle_from_xml_map(&cycle).expect_err("include cycle should fail");
        assert_eq!(cycle_error.code, "INCLUDE_CYCLE");
    }

    #[test]
    fn compile_bundle_rejects_invalid_root_and_duplicate_script_names() {
        let invalid_root = map(&[("main.script.xml", "<defs name=\"x\"></defs>")]);
        let root_error =
            compile_project_bundle_from_xml_map(&invalid_root).expect_err("invalid root");
        assert_eq!(root_error.code, "XML_ROOT_INVALID");

        let duplicate_script_name = map(&[
            ("a.script.xml", "<script name=\"main\"></script>"),
            ("b.script.xml", "<script name=\"main\"></script>"),
        ]);
        let duplicate_error = compile_project_bundle_from_xml_map(&duplicate_script_name)
            .expect_err("duplicate script names should fail");
        assert_eq!(duplicate_error.code, "SCRIPT_NAME_DUPLICATE");
    }

    #[test]
    fn default_values_from_script_params_respects_declared_types() {
        let params = vec![
            ScriptParam {
                name: "hp".to_string(),
                r#type: ScriptType::Primitive {
                    name: "int".to_string(),
                },
                is_ref: false,
                location: SourceSpan::synthetic(),
            },
            ScriptParam {
                name: "name".to_string(),
                r#type: ScriptType::Primitive {
                    name: "string".to_string(),
                },
                is_ref: false,
                location: SourceSpan::synthetic(),
            },
        ];
        let defaults = default_values_from_script_params(&params);
        assert_eq!(defaults.get("hp"), Some(&SlValue::Number(0.0)));
        assert_eq!(defaults.get("name"), Some(&SlValue::String(String::new())));
    }

    fn xml_text(value: &str) -> XmlNode {
        XmlNode::Text(XmlTextNode {
            value: value.to_string(),
            location: SourceSpan::synthetic(),
        })
    }

    fn xml_element(name: &str, attrs: &[(&str, &str)], children: Vec<XmlNode>) -> XmlElementNode {
        XmlElementNode {
            name: name.to_string(),
            attributes: attrs
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
            children,
            location: SourceSpan::synthetic(),
        }
    }

    #[test]
    fn source_kind_and_path_helpers_cover_common_cases() {
        assert!(matches!(
            detect_source_kind("a.script.xml"),
            Some(SourceKind::ScriptXml)
        ));
        assert!(matches!(
            detect_source_kind("a.defs.xml"),
            Some(SourceKind::DefsXml)
        ));
        assert!(matches!(
            detect_source_kind("a.json"),
            Some(SourceKind::Json)
        ));
        assert!(detect_source_kind("a.txt").is_none());

        assert_eq!(
            resolve_include_path("nested/main.script.xml", "../shared.defs.xml"),
            "shared.defs.xml"
        );
        assert_eq!(
            resolve_include_path("/", "shared/main.script.xml"),
            "shared/main.script.xml"
        );
        assert_eq!(
            normalize_virtual_path("./a/./b/../c\\d.script.xml"),
            "a/c/d.script.xml"
        );
        assert_eq!(stable_base("a*b?c"), "a_b_c");
    }

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

    #[test]
    fn parse_type_and_call_argument_helpers_cover_valid_and_invalid_inputs() {
        let span = SourceSpan::synthetic();
        assert!(matches!(
            parse_type_expr("int", &span).expect("primitive"),
            ParsedTypeExpr::Primitive(_)
        ));
        assert!(matches!(
            parse_type_expr("int[]", &span).expect("array"),
            ParsedTypeExpr::Array(_)
        ));
        assert!(matches!(
            parse_type_expr("#{int}", &span).expect("map"),
            ParsedTypeExpr::Map(_)
        ));
        assert!(matches!(
            parse_type_expr("CustomType", &span).expect("custom"),
            ParsedTypeExpr::Custom(_)
        ));
        let invalid_type = parse_type_expr("Map<int,string>", &span).expect_err("invalid");
        assert_eq!(invalid_type.code, "TYPE_PARSE_ERROR");
        let empty_map_type = parse_type_expr("#{   }", &span).expect_err("empty map type");
        assert_eq!(empty_map_type.code, "TYPE_PARSE_ERROR");

        let args = parse_args(Some("1, ref:hp, a + 1".to_string())).expect("args");
        assert_eq!(args.len(), 3);
        assert!(args[1].is_ref);

        let bad_args = parse_args(Some("ref:   ".to_string())).expect_err("bad args");
        assert_eq!(bad_args.code, "CALL_ARGS_PARSE_ERROR");
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
    }

    #[test]
    fn resolve_visible_defs_builds_function_signatures() {
        let span = SourceSpan::synthetic();
        let defs = DefsDeclarations {
            type_decls: vec![ParsedTypeDecl {
                name: "Obj".to_string(),
                qualified_name: "shared.Obj".to_string(),
                fields: vec![ParsedTypeFieldDecl {
                    name: "value".to_string(),
                    type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                    location: span.clone(),
                }],
                location: span.clone(),
            }],
            function_decls: vec![ParsedFunctionDecl {
                name: "make".to_string(),
                qualified_name: "shared.make".to_string(),
                params: vec![ParsedFunctionParamDecl {
                    name: "seed".to_string(),
                    type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                    location: span.clone(),
                }],
                return_binding: ParsedFunctionParamDecl {
                    name: "ret".to_string(),
                    type_expr: ParsedTypeExpr::Custom("Obj".to_string()),
                    location: span.clone(),
                },
                code: "ret = #{value: seed};".to_string(),
                location: span.clone(),
            }],
        };

        let reachable = BTreeSet::from(["shared.defs.xml".to_string()]);
        let defs_by_path = BTreeMap::from([("shared.defs.xml".to_string(), defs)]);

        let (types, functions) =
            resolve_visible_defs(&reachable, &defs_by_path).expect("defs should resolve");
        assert!(types.contains_key("Obj"));
        let function = functions.get("make").expect("function should exist");
        assert_eq!(function.params.len(), 1);
        assert!(matches!(
            function.return_binding.r#type,
            ScriptType::Object { .. }
        ));
    }

    #[test]
    fn resolve_visible_defs_handles_namespace_collisions_and_alias_edges() {
        let span = SourceSpan::synthetic();

        let duplicate_qualified = DefsDeclarations {
            type_decls: vec![ParsedTypeDecl {
                name: "T".to_string(),
                qualified_name: "shared.T".to_string(),
                fields: vec![ParsedTypeFieldDecl {
                    name: "v".to_string(),
                    type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                    location: span.clone(),
                }],
                location: span.clone(),
            }],
            function_decls: Vec::new(),
        };
        let duplicate_defs_by_path = BTreeMap::from([
            ("a.defs.xml".to_string(), duplicate_qualified.clone()),
            ("b.defs.xml".to_string(), duplicate_qualified),
        ]);
        let duplicate_reachable =
            BTreeSet::from(["a.defs.xml".to_string(), "b.defs.xml".to_string()]);
        let duplicate_error = resolve_visible_defs(&duplicate_reachable, &duplicate_defs_by_path)
            .expect_err("duplicate qualified type should fail");
        assert_eq!(duplicate_error.code, "TYPE_DECL_DUPLICATE");

        let defs_by_path = BTreeMap::from([
            (
                "a.defs.xml".to_string(),
                DefsDeclarations {
                    type_decls: Vec::new(),
                    function_decls: vec![ParsedFunctionDecl {
                        name: "doit".to_string(),
                        qualified_name: "a.doit".to_string(),
                        params: Vec::new(),
                        return_binding: ParsedFunctionParamDecl {
                            name: "out".to_string(),
                            type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                            location: span.clone(),
                        },
                        code: "out = 1;".to_string(),
                        location: span.clone(),
                    }],
                },
            ),
            (
                "b.defs.xml".to_string(),
                DefsDeclarations {
                    type_decls: Vec::new(),
                    function_decls: vec![ParsedFunctionDecl {
                        name: "doit".to_string(),
                        qualified_name: "b.doit".to_string(),
                        params: Vec::new(),
                        return_binding: ParsedFunctionParamDecl {
                            name: "out".to_string(),
                            type_expr: ParsedTypeExpr::Primitive("int".to_string()),
                            location: span.clone(),
                        },
                        code: "out = 2;".to_string(),
                        location: span.clone(),
                    }],
                },
            ),
        ]);
        let reachable = BTreeSet::from(["a.defs.xml".to_string(), "b.defs.xml".to_string()]);
        let (_types, functions) =
            resolve_visible_defs(&reachable, &defs_by_path).expect("defs should resolve");
        assert!(functions.contains_key("a.doit"));
        assert!(functions.contains_key("b.doit"));
        assert!(!functions.contains_key("doit"));
    }

    #[test]
    fn resolve_named_type_with_aliases_reports_missing_aliased_target() {
        let error = resolve_named_type_with_aliases(
            "Alias",
            &BTreeMap::new(),
            &BTreeMap::from([("Alias".to_string(), "missing.Type".to_string())]),
            &mut BTreeMap::new(),
            &mut HashSet::new(),
        )
        .expect_err("missing aliased target should fail");
        assert_eq!(error.code, "TYPE_UNKNOWN");
    }

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
        assert!(matches!(
            parse_type_expr("int[]", &span).expect("array should parse"),
            ParsedTypeExpr::Array(_)
        ));
        assert!(matches!(
            parse_type_expr("#{int}", &span).expect("map should parse"),
            ParsedTypeExpr::Map(_)
        ));
    }

    #[test]
    fn compile_group_recurses_for_if_while_and_choice_children() {
        let mut builder = GroupBuilder::new("recursive.script.xml");
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
    fn inline_bool_and_attr_helpers_cover_errors() {
        let node = xml_element("text", &[("value", "x")], vec![xml_text("ignored")]);
        let error = parse_inline_required(&node).expect_err("value attr forbidden");
        assert_eq!(error.code, "XML_ATTR_NOT_ALLOWED");

        let empty = xml_element("text", &[], vec![xml_text("   ")]);
        let error = parse_inline_required(&empty).expect_err("empty inline forbidden");
        assert_eq!(error.code, "XML_EMPTY_NODE_CONTENT");

        let with_child = xml_element(
            "function",
            &[],
            vec![XmlNode::Element(xml_element("x", &[], Vec::new()))],
        );
        let error = parse_inline_required_no_element_children(&with_child)
            .expect_err("child element forbidden");
        assert_eq!(error.code, "XML_FUNCTION_CHILD_NODE_INVALID");

        let bool_node = xml_element("text", &[("once", "maybe")], vec![xml_text("x")]);
        let error = parse_bool_attr(&bool_node, "once", false).expect_err("invalid bool attr");
        assert_eq!(error.code, "XML_ATTR_BOOL_INVALID");

        let miss_attr = get_required_non_empty_attr(&xml_element("x", &[], vec![]), "name")
            .expect_err("missing attr");
        assert_eq!(miss_attr.code, "XML_MISSING_ATTR");
        let empty_attr =
            get_required_non_empty_attr(&xml_element("x", &[("name", " ")], vec![]), "name")
                .expect_err("empty attr");
        assert_eq!(empty_attr.code, "XML_EMPTY_ATTR");

        assert!(has_any_child_content(&xml_element(
            "x",
            &[],
            vec![xml_text(" t ")]
        )));
        assert!(!has_any_child_content(&xml_element(
            "x",
            &[],
            vec![xml_text("   ")]
        )));
        assert!(split_by_top_level_comma("a, f(1,2), #{int}, #{a:1,b:2}").len() >= 4);
    }

    #[test]
    fn defs_and_type_resolution_helpers_cover_duplicate_and_recursive_errors() {
        let bad_defs = map(&[("x.defs.xml", "<script name=\"x\"></script>")]);
        let error = compile_project_bundle_from_xml_map(&bad_defs).expect_err("bad defs root");
        assert_eq!(error.code, "XML_ROOT_INVALID");

        let duplicate_types = map(&[
            (
                "a.defs.xml",
                r#"<defs name="a"><type name="T"><field name="v" type="int"/></type></defs>"#,
            ),
            (
                "b.defs.xml",
                r#"<defs name="b"><type name="T"><field name="v" type="int"/></type></defs>"#,
            ),
            (
                "main.script.xml",
                r#"
<!-- include: a.defs.xml -->
<!-- include: b.defs.xml -->
<script name="main"><var name="v" type="T"/></script>
"#,
            ),
        ]);
        let error = compile_project_bundle_from_xml_map(&duplicate_types)
            .expect_err("ambiguous unqualified type should fail");
        assert_eq!(error.code, "TYPE_UNKNOWN");

        let recursive = map(&[
            (
                "x.defs.xml",
                r#"<defs name="x"><type name="A"><field name="b" type="B"/></type><type name="B"><field name="a" type="A"/></type></defs>"#,
            ),
            (
                "main.script.xml",
                r#"
<!-- include: x.defs.xml -->
<script name="main"><var name="v" type="A"/></script>
"#,
            ),
        ]);
        let error = compile_project_bundle_from_xml_map(&recursive)
            .expect_err("recursive type declarations should fail");
        assert_eq!(error.code, "TYPE_UNKNOWN");
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
        let mut builder = GroupBuilder::new("main.script.xml");
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
        let mut builder = GroupBuilder::new("main.script.xml");
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
        let mut builder = GroupBuilder::new("main.script.xml");
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
        let mut builder = GroupBuilder::new("main.script.xml");
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
            vec![XmlNode::Element(xml_element("unknown", &[], Vec::new()))],
        );
        let mut builder = GroupBuilder::new("main.script.xml");
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
    }

    #[test]
    fn compiler_error_matrix_covers_more_validation_paths() {
        let cases: Vec<(&str, BTreeMap<String, String>, &str)> = vec![
            (
                "json parse error",
                map(&[
                    ("bad.json", "{"),
                    ("main.script.xml", "<script name=\"main\"><text>x</text></script>"),
                ]),
                "JSON_PARSE_ERROR",
            ),
            (
                "defs child invalid",
                map(&[
                    (
                        "x.defs.xml",
                        "<defs name=\"x\"><unknown/></defs>",
                    ),
                    (
                        "main.script.xml",
                        r#"
<!-- include: x.defs.xml -->
<script name="main"><text>x</text></script>
"#,
                    ),
                ]),
                "XML_DEFS_CHILD_INVALID",
            ),
            (
                "type field child invalid",
                map(&[
                    (
                        "x.defs.xml",
                        "<defs name=\"x\"><type name=\"A\"><bad/></type></defs>",
                    ),
                    (
                        "main.script.xml",
                        r#"
<!-- include: x.defs.xml -->
<script name="main"><text>x</text></script>
"#,
                    ),
                ]),
                "XML_TYPE_CHILD_INVALID",
            ),
            (
                "type field duplicate",
                map(&[
                    (
                        "x.defs.xml",
                        "<defs name=\"x\"><type name=\"A\"><field name=\"v\" type=\"int\"/><field name=\"v\" type=\"int\"/></type></defs>",
                    ),
                    (
                        "main.script.xml",
                        r#"
<!-- include: x.defs.xml -->
<script name="main"><text>x</text></script>
"#,
                    ),
                ]),
                "TYPE_FIELD_DUPLICATE",
            ),
            (
                "function duplicate",
                map(&[
                    (
                        "x.defs.xml",
                        "<defs name=\"x\"><function name=\"f\" return=\"int:r\">r=1;</function><function name=\"f\" return=\"int:r\">r=2;</function></defs>",
                    ),
                    (
                        "main.script.xml",
                        r#"
<!-- include: x.defs.xml -->
<script name="main"><text>x</text></script>
"#,
                    ),
                ]),
                "FUNCTION_DECL_DUPLICATE",
            ),
            (
                "unknown custom type in var",
                map(&[(
                    "main.script.xml",
                    "<script name=\"main\"><var name=\"x\" type=\"Unknown\"/></script>",
                )]),
                "TYPE_UNKNOWN",
            ),
            (
                "choice child invalid",
                map(&[(
                    "main.script.xml",
                    "<script name=\"main\"><choice text=\"c\"><bad/></choice></script>",
                )]),
                "XML_CHOICE_CHILD_INVALID",
            ),
            (
                "choice fall_over with when forbidden",
                map(&[(
                    "main.script.xml",
                    "<script name=\"main\"><choice text=\"c\"><option text=\"a\" fall_over=\"true\" when=\"true\"/></choice></script>",
                )]),
                "XML_OPTION_FALL_OVER_WHEN_FORBIDDEN",
            ),
            (
                "choice fall_over duplicate",
                map(&[(
                    "main.script.xml",
                    "<script name=\"main\"><choice text=\"c\"><option text=\"a\" fall_over=\"true\"/><option text=\"b\" fall_over=\"true\"/></choice></script>",
                )]),
                "XML_OPTION_FALL_OVER_DUPLICATE",
            ),
            (
                "choice fall_over not last",
                map(&[(
                    "main.script.xml",
                    "<script name=\"main\"><choice text=\"c\"><option text=\"a\" fall_over=\"true\"/><option text=\"b\"/></choice></script>",
                )]),
                "XML_OPTION_FALL_OVER_NOT_LAST",
            ),
            (
                "input default unsupported",
                map(&[(
                    "main.script.xml",
                    "<script name=\"main\"><input var=\"x\" text=\"p\" default=\"d\"/></script>",
                )]),
                "XML_INPUT_DEFAULT_UNSUPPORTED",
            ),
            (
                "input content forbidden",
                map(&[(
                    "main.script.xml",
                    "<script name=\"main\"><input var=\"x\" text=\"p\">x</input></script>",
                )]),
                "XML_INPUT_CONTENT_FORBIDDEN",
            ),
            (
                "return ref unsupported",
                map(&[(
                    "main.script.xml",
                    "<script name=\"main\"><return script=\"next\" args=\"ref:x\"/></script>",
                )]),
                "XML_RETURN_REF_UNSUPPORTED",
            ),
            (
                "removed node",
                map(&[(
                    "main.script.xml",
                    "<script name=\"main\"><set/></script>",
                )]),
                "XML_REMOVED_NODE",
            ),
            (
                "else at top level",
                map(&[(
                    "main.script.xml",
                    "<script name=\"main\"><else/></script>",
                )]),
                "XML_ELSE_POSITION",
            ),
            (
                "break outside while",
                map(&[(
                    "main.script.xml",
                    "<script name=\"main\"><break/></script>",
                )]),
                "XML_BREAK_OUTSIDE_WHILE",
            ),
            (
                "continue outside while or option",
                map(&[(
                    "main.script.xml",
                    "<script name=\"main\"><continue/></script>",
                )]),
                "XML_CONTINUE_OUTSIDE_WHILE_OR_OPTION",
            ),
            (
                "call args parse error",
                map(&[(
                    "main.script.xml",
                    "<script name=\"main\"><call script=\"s\" args=\"ref:\"/></script>",
                )]),
                "CALL_ARGS_PARSE_ERROR",
            ),
            (
                "script args reserved prefix",
                map(&[(
                    "main.script.xml",
                    "<script name=\"main\" args=\"int:__sl_x\"><text>x</text></script>",
                )]),
                "NAME_RESERVED_PREFIX",
            ),
            (
                "loop times template unsupported",
                map(&[(
                    "main.script.xml",
                    "<script name=\"main\"><loop times=\"${n}\"><text>x</text></loop></script>",
                )]),
                "XML_LOOP_TIMES_TEMPLATE_UNSUPPORTED",
            ),
        ];

        for (name, files, expected_code) in cases {
            let result = compile_project_bundle_from_xml_map(&files);
            assert!(result.is_err(), "case should fail: {}", name);
            let error = result.expect_err("error should exist");
            assert_eq!(error.code, expected_code, "case: {}", name);
        }
    }

    #[test]
    fn compiler_private_helpers_cover_remaining_paths() {
        assert_eq!(
            resolve_include_path("scripts/main.script.xml", "/shared.defs.xml"),
            "shared.defs.xml"
        );
        let reachable = collect_reachable_files("missing.script.xml", &BTreeMap::new());
        assert!(reachable.contains("missing.script.xml"));

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
        assert!(matches!(array_ty, ScriptType::Array { .. }));
        let map_ty = resolve_type_expr_with_lookup(
            &ParsedTypeExpr::Map(Box::new(ParsedTypeExpr::Primitive("string".to_string()))),
            &BTreeMap::new(),
            &mut resolved_for_lookup,
            &mut visiting_for_lookup,
            &span,
        )
        .expect("map lookup should resolve");
        assert!(matches!(map_ty, ScriptType::Map { .. }));

        let array = resolve_type_expr(
            &ParsedTypeExpr::Array(Box::new(ParsedTypeExpr::Primitive("int".to_string()))),
            &BTreeMap::new(),
            &span,
        )
        .expect("array should resolve");
        assert!(matches!(array, ScriptType::Array { .. }));
        let map_resolved = resolve_type_expr(
            &ParsedTypeExpr::Map(Box::new(ParsedTypeExpr::Primitive("int".to_string()))),
            &BTreeMap::new(),
            &span,
        )
        .expect("map should resolve");
        assert!(matches!(map_resolved, ScriptType::Map { .. }));

        let non_script_root = xml_element("defs", &[("name", "x")], Vec::new());
        let compile_root_error = compile_script(
            "x.script.xml",
            &non_script_root,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &[],
            &BTreeSet::new(),
        )
        .expect_err("compile_script should require script root");
        assert_eq!(compile_root_error.code, "XML_ROOT_INVALID");

        let rich_script = map(&[(
            "main.script.xml",
            r#"
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
"#,
        )]);
        let compiled =
            compile_project_bundle_from_xml_map(&rich_script).expect("compile should pass");
        let main = compiled.scripts.get("main").expect("main script");
        let root_group = main.groups.get(&main.root_group_id).expect("root group");
        assert!(root_group
            .nodes
            .iter()
            .any(|node| matches!(node, ScriptNode::While { .. })));
        assert!(root_group
            .nodes
            .iter()
            .any(|node| matches!(node, ScriptNode::Input { .. })
                || matches!(node, ScriptNode::Call { .. })
                || matches!(node, ScriptNode::If { .. })));

        let defs_resolution = map(&[
            (
                "shared.defs.xml",
                r##"
<defs name="shared">
  <type name="Obj">
    <field name="values" type="#{int[]}"/>
  </type>
  <function name="build" return="Obj:r">
    r = #{values: #{a: [1]}};
  </function>
</defs>
"##,
            ),
            (
                "main.script.xml",
                r#"
<!-- include: shared.defs.xml -->
<script name="main">
  <var name="x" type="Obj"/>
</script>
"#,
            ),
        ]);
        let _ = compile_project_bundle_from_xml_map(&defs_resolution)
            .expect("defs return/field type resolution should pass");

        let mut builder_ok = GroupBuilder::new("manual.script.xml");
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

        let mut loop_builder = GroupBuilder::new("loop.script.xml");
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
            slvalue_from_json(JsonValue::Null),
            SlValue::String("null".to_string())
        );
    }
}
