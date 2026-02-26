#[test]
fn example_11_choice_fallover_continue_matches_testcase() {
    let example_dir = sl_test_example::example_dir("11-choice-fallover-continue");
    let case_path = sl_test_example::testcase_path("11-choice-fallover-continue");
    sl_tool::assert_case(&example_dir, &case_path).expect("example testcase should pass");
}
