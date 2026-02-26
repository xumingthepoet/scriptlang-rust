#[test]
fn example_08_json_globals_matches_testcase() {
    let example_dir = sl_test_example::example_dir("08-json-globals");
    let case_path = sl_test_example::testcase_path("08-json-globals");
    sl_tool::assert_case(&example_dir, &case_path).expect("example testcase should pass");
}
