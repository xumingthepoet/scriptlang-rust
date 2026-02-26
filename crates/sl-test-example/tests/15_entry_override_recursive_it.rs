#[test]
fn example_15_entry_override_recursive_matches_testcase() {
    let example_dir = sl_test_example::example_dir("15-entry-override-recursive");
    let case_path = sl_test_example::testcase_path("15-entry-override-recursive");
    sl_tool::assert_case(&example_dir, &case_path).expect("example testcase should pass");
}
