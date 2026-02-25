use std::fs;
use std::process::Command;

#[test]
fn agent_start_runs_all_rhai_examples() {
    let bin = env!("CARGO_BIN_EXE_sl-cli");
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let examples_root = manifest_dir
        .join("..")
        .join("..")
        .join("examples")
        .join("scripts-rhai");

    let mut directories = fs::read_dir(&examples_root)
        .expect("examples root must exist")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    directories.sort();

    assert!(!directories.is_empty(), "expected copied Rhai examples");

    for directory in directories {
        let state_out = std::env::temp_dir().join(format!(
            "scriptlang-rs-smoke-{}.json",
            directory.file_name().unwrap_or_default().to_string_lossy()
        ));

        let output = Command::new(bin)
            .arg("agent")
            .arg("start")
            .arg("--scripts-dir")
            .arg(&directory)
            .arg("--state-out")
            .arg(&state_out)
            .output()
            .expect("cli should execute");

        if !output.status.success() {
            panic!(
                "scenario {} failed\nstdout:\n{}\nstderr:\n{}",
                directory.display(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("RESULT:OK"),
            "stdout missing RESULT:OK for {}",
            directory.display()
        );
        assert!(
            stdout.contains("EVENT:"),
            "stdout missing EVENT for {}",
            directory.display()
        );
    }
}
