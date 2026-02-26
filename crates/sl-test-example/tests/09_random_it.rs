#[test]
fn example_09_random_matches_testcase() {
    let example_dir = sl_test_example::example_dir("09-random");
    let case_path = sl_test_example::testcase_path("09-random");
    sl_tool::assert_case(&example_dir, &case_path).expect("example testcase should pass");
}
