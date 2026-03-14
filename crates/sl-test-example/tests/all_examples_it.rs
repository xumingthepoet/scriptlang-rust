fn assert_example(name: &str) {
    let example_dir = sl_test_example::example_dir(name);
    let case_path = sl_test_example::testcase_path(name);
    sl_test_example::assert_case(&example_dir, &case_path).expect("example testcase should pass");
}

fn read_scripts_xml_from_example(name: &str) -> std::collections::BTreeMap<String, String> {
    let example_dir = sl_test_example::example_dir(name);
    let mut scripts = std::collections::BTreeMap::new();
    for entry in walkdir::WalkDir::new(&example_dir)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if !path.to_string_lossy().ends_with(".xml") {
            continue;
        }
        let relative = path
            .strip_prefix(&example_dir)
            .expect("example path should strip prefix")
            .to_string_lossy()
            .replace('\\', "/");
        let content = std::fs::read_to_string(path).expect("example xml should be readable");
        scripts.insert(relative, content);
    }
    scripts
}

#[test]
fn example_01_text_code_matches_testcase() {
    assert_example("01-text-code");
}

#[test]
fn example_02_if_while_matches_testcase() {
    assert_example("02-if-while");
}

#[test]
fn example_03_choice_loop_matches_testcase() {
    assert_example("03-choice-loop");
}

#[test]
fn example_04_call_ref_return_matches_testcase() {
    assert_example("04-call-ref-return");
}

#[test]
fn example_05_return_transfer_matches_testcase() {
    assert_example("05-return-transfer");
}

#[test]
fn example_06_snapshot_flow_matches_testcase() {
    assert_example("06-snapshot-flow");
}

#[test]
fn example_07_battle_duel_matches_testcase() {
    assert_example("07-battle-duel");
}

#[test]
fn example_08_json_globals_matches_testcase() {
    assert_example("08-json-globals");
}

#[test]
fn example_09_random_matches_testcase() {
    assert_example("09-random");
}

#[test]
fn example_10_once_static_matches_testcase() {
    assert_example("10-once-static");
}

#[test]
fn example_11_choice_fallover_continue_matches_testcase() {
    assert_example("11-choice-fallover-continue");
}

#[test]
fn example_12_while_break_continue_matches_testcase() {
    assert_example("12-while-break-continue");
}

#[test]
fn example_13_for_macro_matches_testcase() {
    assert_example("13-for-macro");
}

#[test]
fn example_14_module_functions_matches_testcase() {
    assert_example("14-module-functions");
}

#[test]
fn example_15_entry_override_recursive_matches_testcase() {
    assert_example("15-entry-override-recursive");
}

#[test]
fn example_16_input_name_matches_testcase() {
    assert_example("16-input-name");
}

#[test]
fn example_17_module_global_shadowing_matches_testcase() {
    assert_example("17-module-global-shadowing");
}

#[test]
fn example_18_group_container_matches_testcase() {
    assert_example("18-group-container");
}

#[test]
fn example_19_dynamic_choice_mixed_matches_testcase() {
    assert_example("19-dynamic-choice-mixed");
}

#[test]
fn example_20_dynamic_choice_nested_matches_testcase() {
    assert_example("20-dynamic-choice-nested");
}

#[test]
fn example_21_directory_import_matches_testcase() {
    assert_example("21-directory-import");
}

#[test]
fn example_27_dynamic_transfer_target_matches_testcase() {
    assert_example("27-dynamic-transfer-target");
}

#[test]
fn example_28_map_coverage_matches_testcase() {
    assert_example("28-map-coverage");
}

#[test]
fn example_29_alias_directives_matches_testcase() {
    assert_example("29-alias-directives");
}

#[test]
fn example_30_invoke_function_alias_module_var_matches_testcase() {
    assert_example("30-invoke-function-alias-module-var");
}

#[test]
fn example_31_invoke_map_short_function_ref_matches_testcase() {
    assert_example("31-invoke-map-short-function-ref");
}

#[test]
fn example_32_temp_input_macro_matches_testcase() {
    assert_example("32-temp-input-macro");
}

#[test]
fn example_33_script_context_macro_matches_testcase() {
    assert_example("33-script-context-macro");
}

#[test]
fn example_34_invoke_private_capability_matches_testcase() {
    assert_example("34-invoke-private-capability");
}

#[test]
fn example_35_script_private_capability_matches_testcase() {
    assert_example("35-script-private-capability");
}

#[test]
fn example_36_terminal_structure_check_reports_compile_error() {
    let scripts_xml = read_scripts_xml_from_example("36-terminal-structure-check");
    let error = sl_api::compile_artifact_from_xml_map(&scripts_xml, Some("main.main".to_string()))
        .expect_err("invalid terminal structure should fail at compile time");
    assert_eq!(error.code, "XML_SCRIPT_TERMINATOR_REQUIRED");
}

#[test]
fn example_37_lint_function_script_literal_compiles() {
    let scripts_xml = read_scripts_xml_from_example("37-lint-function-script-literal");
    sl_api::compile_artifact_from_xml_map(&scripts_xml, Some("main.main".to_string()))
        .expect("lint regression example should compile");
}

#[test]
fn example_38_invalid_qualified_enum_name_reports_compile_error() {
    let scripts_xml = read_scripts_xml_from_example("38-invalid-qualified-enum-name");
    let error = sl_api::compile_artifact_from_xml_map(&scripts_xml, Some("main.main".to_string()))
        .expect_err("qualified enum declaration name should fail at compile time");
    assert_eq!(error.code, "NAME_IDENTIFIER_INVALID");
}

#[test]
fn example_22_access_control_matches_testcase() {
    assert_example("22-access-control");
}

#[test]
fn example_23_const_basics_matches_testcase() {
    assert_example("23-const-basics");
}

#[test]
fn example_24_invoke_dynamic_matches_testcase() {
    assert_example("24-invoke-dynamic");
}

#[test]
fn example_25_function_invoke_matches_testcase() {
    assert_example("25-function-invoke");
}

#[test]
fn example_26_enum_flow_matches_testcase() {
    assert_example("26-enum-flow");
}
