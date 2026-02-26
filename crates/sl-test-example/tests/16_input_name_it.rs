#[test]
fn example_16_input_name_matches_testcase() {
    let example_dir = sl_test_example::example_dir("16-input-name");
    let case_path = sl_test_example::testcase_path("16-input-name");
    sl_tool::assert_case(&example_dir, &case_path).expect("example testcase should pass");
}
