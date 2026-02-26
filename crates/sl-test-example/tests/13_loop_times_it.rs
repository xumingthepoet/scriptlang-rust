#[test]
fn example_13_loop_times_matches_testcase() {
    let example_dir = sl_test_example::example_dir("13-loop-times");
    let case_path = sl_test_example::testcase_path("13-loop-times");
    sl_tool::assert_case(&example_dir, &case_path).expect("example testcase should pass");
}
