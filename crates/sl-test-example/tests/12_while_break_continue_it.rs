#[test]
fn example_12_while_break_continue_matches_testcase() {
    let example_dir = sl_test_example::example_dir("12-while-break-continue");
    let case_path = sl_test_example::testcase_path("12-while-break-continue");
    sl_tool::assert_case(&example_dir, &case_path).expect("example testcase should pass");
}
