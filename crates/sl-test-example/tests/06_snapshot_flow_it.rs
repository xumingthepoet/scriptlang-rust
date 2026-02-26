#[test]
fn example_06_snapshot_flow_matches_testcase() {
    let example_dir = sl_test_example::example_dir("06-snapshot-flow");
    let case_path = sl_test_example::testcase_path("06-snapshot-flow");
    sl_tool::assert_case(&example_dir, &case_path).expect("example testcase should pass");
}
