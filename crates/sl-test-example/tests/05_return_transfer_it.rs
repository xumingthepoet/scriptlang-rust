#[test]
fn example_05_return_transfer_matches_testcase() {
    let example_dir = sl_test_example::example_dir("05-return-transfer");
    let case_path = sl_test_example::testcase_path("05-return-transfer");
    sl_tool::assert_case(&example_dir, &case_path).expect("example testcase should pass");
}
