#[test]
fn example_07_battle_duel_matches_testcase() {
    let example_dir = sl_test_example::example_dir("07-battle-duel");
    let case_path = sl_test_example::testcase_path("07-battle-duel");
    sl_tool::assert_case(&example_dir, &case_path).expect("example testcase should pass");
}
