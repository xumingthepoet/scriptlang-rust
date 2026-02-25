use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn examples_root() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("examples")
        .join("scripts-rhai")
}

fn run_agent(args: &[&str]) -> std::process::Output {
    let bin = env!("CARGO_BIN_EXE_sl-cli");
    Command::new(bin)
        .arg("agent")
        .args(args)
        .output()
        .expect("agent command should run")
}

fn parse_state_out(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .find_map(|line| line.strip_prefix("STATE_OUT:").map(|v| v.to_string()))
        .filter(|value| value != "NONE")
}

#[test]
fn agent_choice_flow_reaches_end() {
    let scenario = examples_root().join("06-snapshot-flow");
    let state_1 = std::env::temp_dir().join("sl-cli-agent-choice-1.json");
    let state_2 = std::env::temp_dir().join("sl-cli-agent-choice-2.json");

    let start = run_agent(&[
        "start",
        "--scripts-dir",
        scenario.to_str().expect("path should be utf-8"),
        "--state-out",
        state_1.to_str().expect("path should be utf-8"),
    ]);
    assert!(start.status.success(), "start failed");
    let start_stdout = String::from_utf8_lossy(&start.stdout);
    assert!(start_stdout.contains("RESULT:OK"));
    assert!(start_stdout.contains("EVENT:CHOICES"));
    assert!(parse_state_out(&start_stdout).is_some());

    let choose = run_agent(&[
        "choose",
        "--state-in",
        state_1.to_str().expect("path should be utf-8"),
        "--choice",
        "0",
        "--state-out",
        state_2.to_str().expect("path should be utf-8"),
    ]);
    assert!(choose.status.success(), "choose failed");
    let choose_stdout = String::from_utf8_lossy(&choose.stdout);
    assert!(choose_stdout.contains("RESULT:OK"));
    assert!(choose_stdout.contains("EVENT:END"));
    assert!(choose_stdout.contains("STATE_OUT:NONE"));
}

#[test]
fn agent_input_flow_uses_state_roundtrip() {
    let scenario = examples_root().join("16-input-name");
    let state_1 = std::env::temp_dir().join("sl-cli-agent-input-1.json");
    let state_2 = std::env::temp_dir().join("sl-cli-agent-input-2.json");
    let state_3 = std::env::temp_dir().join("sl-cli-agent-input-3.json");

    let start = run_agent(&[
        "start",
        "--scripts-dir",
        scenario.to_str().expect("path should be utf-8"),
        "--state-out",
        state_1.to_str().expect("path should be utf-8"),
    ]);
    assert!(start.status.success(), "start failed");
    let start_stdout = String::from_utf8_lossy(&start.stdout);
    assert!(start_stdout.contains("EVENT:INPUT"));
    assert!(start_stdout.contains("INPUT_DEFAULT_JSON"));

    let input_1 = run_agent(&[
        "input",
        "--state-in",
        state_1.to_str().expect("path should be utf-8"),
        "--text",
        "",
        "--state-out",
        state_2.to_str().expect("path should be utf-8"),
    ]);
    assert!(input_1.status.success(), "first input failed");
    let input_1_stdout = String::from_utf8_lossy(&input_1.stdout);
    assert!(input_1_stdout.contains("EVENT:INPUT"));
    assert!(parse_state_out(&input_1_stdout).is_some());

    let input_2 = run_agent(&[
        "input",
        "--state-in",
        state_2.to_str().expect("path should be utf-8"),
        "--text",
        "Guild",
        "--state-out",
        state_3.to_str().expect("path should be utf-8"),
    ]);
    assert!(input_2.status.success(), "second input failed");
    let input_2_stdout = String::from_utf8_lossy(&input_2.stdout);
    assert!(input_2_stdout.contains("RESULT:OK"));
    assert!(input_2_stdout.contains("EVENT:END"));
}

#[test]
fn tui_supports_commands_and_quit() {
    let bin = env!("CARGO_BIN_EXE_sl-cli");
    let scenario = examples_root().join("03-choice-once");
    let state_file = std::env::temp_dir().join("sl-cli-tui-state.json");
    let _ = fs::remove_file(&state_file);

    let mut child = Command::new(bin)
        .arg("tui")
        .arg("--scripts-dir")
        .arg(scenario.to_str().expect("path should be utf-8"))
        .arg("--state-file")
        .arg(state_file.to_str().expect("path should be utf-8"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("tui should spawn");

    {
        let stdin = child.stdin.as_mut().expect("stdin should be piped");
        stdin
            .write_all(b":help\n:save\n:restart\n:load\n:quit\n")
            .expect("should write commands");
    }

    let output = child.wait_with_output().expect("tui should complete");
    assert!(output.status.success(), "tui should exit with success");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("ScriptLang TUI"));
    assert!(stdout.contains("commands: :help :save :load :restart :quit"));
    assert!(stdout.contains("saved:"));
    assert!(stdout.contains("restarted"));
    assert!(stdout.contains("loaded:"));
    assert!(stdout.contains("bye"));
}

#[test]
fn agent_start_missing_dir_returns_error_envelope() {
    let output = run_agent(&[
        "start",
        "--scripts-dir",
        "/path/does/not/exist",
        "--state-out",
        "/tmp/none.json",
    ]);
    assert!(
        !output.status.success(),
        "start should fail for missing dir"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("RESULT:ERROR"));
    assert!(stdout.contains("ERROR_CODE:CLI_SOURCE_NOT_FOUND"));
}

#[test]
fn agent_choose_with_invalid_index_returns_error() {
    let scenario = examples_root().join("06-snapshot-flow");
    let state = std::env::temp_dir().join("sl-cli-agent-invalid-choice-state.json");

    let start = run_agent(&[
        "start",
        "--scripts-dir",
        scenario.to_str().expect("path should be utf-8"),
        "--state-out",
        state.to_str().expect("path should be utf-8"),
    ]);
    assert!(start.status.success(), "start failed");

    let choose = run_agent(&[
        "choose",
        "--state-in",
        state.to_str().expect("path should be utf-8"),
        "--choice",
        "99",
        "--state-out",
        "/tmp/unreachable.json",
    ]);
    assert!(
        !choose.status.success(),
        "choose with invalid index should fail"
    );
    let stdout = String::from_utf8_lossy(&choose.stdout);
    assert!(stdout.contains("RESULT:ERROR"));
    assert!(stdout.contains("ERROR_CODE:ENGINE_CHOICE_INDEX"));
}

#[test]
fn tui_invalid_choice_input_returns_parse_error() {
    let bin = env!("CARGO_BIN_EXE_sl-cli");
    let scenario = examples_root().join("06-snapshot-flow");

    let mut child = Command::new(bin)
        .arg("tui")
        .arg("--scripts-dir")
        .arg(scenario.to_str().expect("path should be utf-8"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("tui should spawn");

    {
        let stdin = child.stdin.as_mut().expect("stdin should be piped");
        stdin
            .write_all(b"not-a-number\n")
            .expect("should write invalid choice");
    }

    let output = child.wait_with_output().expect("tui should complete");
    assert!(!output.status.success(), "invalid choice should fail");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("RESULT:ERROR"));
    assert!(stdout.contains("ERROR_CODE:TUI_CHOICE_PARSE"));
}

#[test]
fn agent_choose_can_emit_next_boundary_and_state_out() {
    let scenario = examples_root().join("03-choice-once");
    let state_1 = std::env::temp_dir().join("sl-cli-agent-choose-boundary-1.json");
    let state_2 = std::env::temp_dir().join("sl-cli-agent-choose-boundary-2.json");

    let start = run_agent(&[
        "start",
        "--scripts-dir",
        scenario.to_str().expect("path should be utf-8"),
        "--state-out",
        state_1.to_str().expect("path should be utf-8"),
    ]);
    assert!(start.status.success(), "start should pass");
    let choose = run_agent(&[
        "choose",
        "--state-in",
        state_1.to_str().expect("path should be utf-8"),
        "--choice",
        "0",
        "--state-out",
        state_2.to_str().expect("path should be utf-8"),
    ]);
    assert!(choose.status.success(), "choose should pass");
    let stdout = String::from_utf8_lossy(&choose.stdout);
    assert!(stdout.contains("RESULT:OK"));
    assert!(stdout.contains("EVENT:CHOICES"));
    assert!(stdout.contains("STATE_OUT:"));
}

#[test]
fn tui_input_path_and_end_are_covered() {
    let bin = env!("CARGO_BIN_EXE_sl-cli");
    let scenario = examples_root().join("16-input-name");

    let mut child = Command::new(bin)
        .arg("tui")
        .arg("--scripts-dir")
        .arg(scenario.to_str().expect("path should be utf-8"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("tui should spawn");

    {
        let stdin = child.stdin.as_mut().expect("stdin should be piped");
        stdin
            .write_all(b"\nGuild\n")
            .expect("should send input responses");
    }

    let output = child.wait_with_output().expect("tui should finish");
    assert!(output.status.success(), "tui should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[END]"));
}

#[test]
fn tui_text_only_scenario_reaches_end() {
    let bin = env!("CARGO_BIN_EXE_sl-cli");
    let scenario = examples_root().join("01-text-code");

    let output = Command::new(bin)
        .arg("tui")
        .arg("--scripts-dir")
        .arg(scenario.to_str().expect("path should be utf-8"))
        .output()
        .expect("tui should run");
    assert!(output.status.success(), "text-only tui should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[END]"));
}

#[test]
fn tui_choice_loop_continue_path_is_exercised() {
    let bin = env!("CARGO_BIN_EXE_sl-cli");
    let scenario = examples_root().join("06-snapshot-flow");

    let mut child = Command::new(bin)
        .arg("tui")
        .arg("--scripts-dir")
        .arg(scenario.to_str().expect("path should be utf-8"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("tui should spawn");

    {
        let stdin = child.stdin.as_mut().expect("stdin should be piped");
        stdin
            .write_all(b":help\n0\n")
            .expect("should write choice flow inputs");
    }

    let output = child.wait_with_output().expect("tui should finish");
    assert!(output.status.success(), "tui should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[END]"));
}

#[test]
fn tui_input_loop_continue_path_is_exercised() {
    let bin = env!("CARGO_BIN_EXE_sl-cli");
    let scenario = examples_root().join("16-input-name");

    let mut child = Command::new(bin)
        .arg("tui")
        .arg("--scripts-dir")
        .arg(scenario.to_str().expect("path should be utf-8"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("tui should spawn");

    {
        let stdin = child.stdin.as_mut().expect("stdin should be piped");
        stdin
            .write_all(b":help\n\n:help\nGuild\n")
            .expect("should write input flow inputs");
    }

    let output = child.wait_with_output().expect("tui should finish");
    assert!(output.status.success(), "tui should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[END]"));
}
