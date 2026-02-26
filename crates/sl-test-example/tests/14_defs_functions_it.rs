#[test]
fn example_14_defs_functions_matches_testcase() {
    let example_dir = sl_test_example::example_dir("14-defs-functions");
    let case_path = sl_test_example::testcase_path("14-defs-functions");
    sl_tool::assert_case(&example_dir, &case_path).expect("example testcase should pass");
}
