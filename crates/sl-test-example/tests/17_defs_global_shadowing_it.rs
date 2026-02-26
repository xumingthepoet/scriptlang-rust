#[test]
fn example_17_defs_global_shadowing_matches_testcase() {
    let example_dir = sl_test_example::example_dir("17-defs-global-shadowing");
    let case_path = sl_test_example::testcase_path("17-defs-global-shadowing");
    sl_tool::assert_case(&example_dir, &case_path).expect("example testcase should pass");
}
