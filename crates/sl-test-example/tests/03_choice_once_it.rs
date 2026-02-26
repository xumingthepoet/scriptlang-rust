#[test]
fn example_03_choice_once_matches_testcase() {
    let example_dir = sl_test_example::example_dir("03-choice-once");
    let case_path = sl_test_example::testcase_path("03-choice-once");
    sl_tool::assert_case(&example_dir, &case_path).expect("example testcase should pass");
}
