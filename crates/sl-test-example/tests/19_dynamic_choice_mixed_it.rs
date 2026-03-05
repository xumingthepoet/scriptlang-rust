#[test]
fn example_19_dynamic_choice_mixed_matches_testcase() {
    let example_dir = sl_test_example::example_dir("19-dynamic-choice-mixed");
    let case_path = sl_test_example::testcase_path("19-dynamic-choice-mixed");
    sl_test_example::assert_case(&example_dir, &case_path).expect("example testcase should pass");
}
