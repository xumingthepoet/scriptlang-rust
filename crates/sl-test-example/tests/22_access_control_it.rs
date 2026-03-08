#[test]
fn example_22_access_control_matches_testcase() {
    let example_dir = sl_test_example::example_dir("22-access-control");
    let case_path = sl_test_example::testcase_path("22-access-control");
    sl_test_example::assert_case(&example_dir, &case_path).expect("example testcase should pass");
}
