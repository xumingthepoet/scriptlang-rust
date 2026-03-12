fn assert_example(name: &str) {
    let example_dir = sl_test_example::example_dir(name);
    let case_path = sl_test_example::testcase_path(name);
    sl_test_example::assert_case(&example_dir, &case_path).expect("example testcase should pass");
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
