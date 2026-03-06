#[test]
fn example_21_dynamic_transfer_target_matches_testcase() {
    let example_dir = sl_test_example::example_dir("21-dynamic-transfer-target");
    let case_path = sl_test_example::testcase_path("21-dynamic-transfer-target");
    sl_test_example::assert_case(&example_dir, &case_path).expect("example testcase should pass");
}
