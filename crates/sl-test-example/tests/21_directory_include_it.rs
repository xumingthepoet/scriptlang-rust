#[test]
fn example_21_directory_include_matches_testcase() {
    let example_dir = sl_test_example::example_dir("21-directory-include");
    let case_path = sl_test_example::testcase_path("21-directory-include");
    sl_test_example::assert_case(&example_dir, &case_path).expect("example testcase should pass");
}
