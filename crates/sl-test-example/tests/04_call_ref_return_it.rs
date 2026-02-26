#[test]
fn example_04_call_ref_return_matches_testcase() {
    let example_dir = sl_test_example::example_dir("04-call-ref-return");
    let case_path = sl_test_example::testcase_path("04-call-ref-return");
    sl_tool::assert_case(&example_dir, &case_path).expect("example testcase should pass");
}
