#[test]
fn example_20_dynamic_choice_nested_matches_testcase() {
    let example_dir = sl_test_example::example_dir("20-dynamic-choice-nested");
    let case_path = sl_test_example::testcase_path("20-dynamic-choice-nested");
    sl_test_example::assert_case(&example_dir, &case_path).expect("example testcase should pass");
}
