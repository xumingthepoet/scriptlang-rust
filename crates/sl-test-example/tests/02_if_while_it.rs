#[test]
fn example_02_if_while_matches_testcase() {
    let example_dir = sl_test_example::example_dir("02-if-while");
    let case_path = sl_test_example::testcase_path("02-if-while");
    sl_tool::assert_case(&example_dir, &case_path).expect("example testcase should pass");
}
