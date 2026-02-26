#[test]
fn example_10_once_static_matches_testcase() {
    let example_dir = sl_test_example::example_dir("10-once-static");
    let case_path = sl_test_example::testcase_path("10-once-static");
    sl_tool::assert_case(&example_dir, &case_path).expect("example testcase should pass");
}
