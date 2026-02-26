#[test]
fn example_18_group_container_matches_testcase() {
    let example_dir = sl_test_example::example_dir("18-group-container");
    let case_path = sl_test_example::testcase_path("18-group-container");
    sl_tool::assert_case(&example_dir, &case_path).expect("example testcase should pass");
}
