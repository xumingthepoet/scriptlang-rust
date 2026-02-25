use super::*;
use sl_core::SnapshotV3;
use std::collections::BTreeMap;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_path(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should be monotonic")
        .as_nanos();
    std::env::temp_dir().join(format!("scriptlang-rs-{}-{}", name, nanos))
}

fn write_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent should be created");
    }
    fs::write(path, content).expect("file should be written");
}

fn example_scripts_dir(example: &str) -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("examples")
        .join("scripts-rhai")
        .join(example)
        .to_string_lossy()
        .to_string()
}

#[test]
fn load_source_by_ref_validates_ref_prefix() {
    let error = load_source_by_ref("unknown:main").expect_err("invalid ref should fail");
    assert_eq!(error.code, "CLI_SOURCE_REF_INVALID");
}

#[test]
fn resolve_scripts_dir_validates_existence_and_directory() {
    let missing = temp_path("missing-dir");
    let missing_err = resolve_scripts_dir(missing.to_string_lossy().as_ref())
        .expect_err("missing path should fail");
    assert_eq!(missing_err.code, "CLI_SOURCE_NOT_FOUND");

    let file_path = temp_path("plain-file");
    write_file(&file_path, "x");
    let file_err = resolve_scripts_dir(file_path.to_string_lossy().as_ref())
        .expect_err("file path should fail");
    assert_eq!(file_err.code, "CLI_SOURCE_NOT_DIR");

    let cwd = std::env::current_dir().expect("cwd");
    let relative_root = temp_path("relative-scripts-dir");
    fs::create_dir_all(&relative_root).expect("root");
    let relative_child = relative_root.join("child");
    fs::create_dir_all(&relative_child).expect("child");
    write_file(
        &relative_child.join("main.script.xml"),
        "<script name=\"main\"></script>",
    );

    std::env::set_current_dir(&relative_root).expect("switch cwd");
    let resolved = resolve_scripts_dir("child").expect("relative dir should resolve");
    assert!(resolved.ends_with("child"));
    std::env::set_current_dir(cwd).expect("restore cwd");
}

#[test]
fn read_scripts_xml_from_dir_filters_supported_extensions() {
    let root = temp_path("scripts-dir");
    fs::create_dir_all(&root).expect("root should be created");
    write_file(
        &root.join("main.script.xml"),
        "<script name=\"main\"></script>",
    );
    write_file(&root.join("defs.defs.xml"), "<defs name=\"d\"></defs>");
    write_file(&root.join("data.json"), "{\"ok\":true}");
    write_file(&root.join("skip.txt"), "ignored");

    let scripts = read_scripts_xml_from_dir(&root).expect("scan should pass");
    assert_eq!(scripts.len(), 3);
    assert!(scripts.contains_key("main.script.xml"));
    assert!(scripts.contains_key("defs.defs.xml"));
    assert!(scripts.contains_key("data.json"));
}

#[test]
fn read_scripts_xml_from_dir_errors_when_no_source_files() {
    let root = temp_path("empty-scripts-dir");
    fs::create_dir_all(&root).expect("root should be created");
    write_file(&root.join("readme.txt"), "not source");

    let error = read_scripts_xml_from_dir(&root).expect_err("empty source set should return error");
    assert_eq!(error.code, "CLI_SOURCE_EMPTY");
}

#[test]
fn save_and_load_player_state_roundtrip_and_schema_validation() {
    let state_path = temp_path("player-state.json");
    let state = PlayerStateV3 {
        schema_version: PLAYER_STATE_SCHEMA.to_string(),
        scenario_id: "scripts-dir:/tmp/demo".to_string(),
        compiler_version: DEFAULT_COMPILER_VERSION.to_string(),
        snapshot: SnapshotV3 {
            schema_version: "snapshot.v3".to_string(),
            compiler_version: DEFAULT_COMPILER_VERSION.to_string(),
            runtime_frames: Vec::new(),
            rng_state: 1,
            pending_boundary: sl_core::PendingBoundaryV3::Choice {
                node_id: "n1".to_string(),
                items: Vec::new(),
                prompt_text: None,
            },
            once_state_by_script: BTreeMap::new(),
        },
    };
    save_player_state(&state_path, &state).expect("save should pass");
    let loaded = load_player_state(&state_path).expect("load should pass");
    assert_eq!(loaded.schema_version, PLAYER_STATE_SCHEMA);
    assert_eq!(loaded.scenario_id, state.scenario_id);

    let bad_path = temp_path("bad-player-state.json");
    let mut bad_json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&state_path).expect("read state"))
            .expect("state json should parse");
    bad_json["schemaVersion"] = serde_json::Value::String("player-state.bad".to_string());
    write_file(
        &bad_path,
        &serde_json::to_string(&bad_json).expect("json should serialize"),
    );
    let error = load_player_state(&bad_path).expect_err("bad schema should fail");
    assert_eq!(error.code, "CLI_STATE_SCHEMA");

    let not_found = temp_path("missing-player-state.json");
    let error = load_player_state(&not_found).expect_err("missing file should fail");
    assert_eq!(error.code, "CLI_STATE_NOT_FOUND");

    let write_root_error =
        save_player_state(Path::new("/"), &state).expect_err("writing root should fail");
    assert_eq!(write_root_error.code, "CLI_STATE_WRITE");
}

#[test]
fn run_to_boundary_and_load_source_helpers_work_with_examples() {
    let scripts_dir = example_scripts_dir("06-snapshot-flow");
    let loaded = load_source_by_scripts_dir(&scripts_dir, "main").expect("source should be loaded");
    assert!(loaded.id.starts_with("scripts-dir:"));
    assert_eq!(loaded.entry_script, "main");

    let mut engine = create_engine_from_xml(CreateEngineFromXmlOptions {
        scripts_xml: loaded.scripts_xml.clone(),
        entry_script: Some(loaded.entry_script.clone()),
        entry_args: None,
        host_functions: None,
        random_seed: Some(1),
        compiler_version: Some(DEFAULT_COMPILER_VERSION.to_string()),
    })
    .expect("engine should build");

    let boundary = run_to_boundary(&mut engine).expect("boundary should be emitted");
    assert_eq!(boundary.event, BoundaryEvent::Choices);
    assert!(!boundary.choices.is_empty());

    let loaded_by_ref = load_source_by_ref(&loaded.id).expect("load by ref should pass");
    assert_eq!(loaded_by_ref.entry_script, "main");
}

#[test]
fn emit_error_returns_non_zero_exit_code() {
    let code = emit_error(ScriptLangError::new("ERR", "failed"));
    assert_eq!(code, 1);
}

#[test]
fn load_source_by_scripts_dir_works() {
    let root = temp_path("scripts-dir-test");
    fs::create_dir_all(&root).expect("root");
    write_file(
        &root.join("main.script.xml"),
        r#"<script name="main"><text>Hello</text></script>"#,
    );

    let loaded =
        load_source_by_scripts_dir(&root.to_string_lossy(), "main").expect("load should pass");
    assert!(loaded.id.starts_with("scripts-dir:"));
    assert_eq!(loaded.entry_script, "main");
    assert!(loaded.scripts_xml.contains_key("main.script.xml"));
}

#[test]
fn load_source_by_scripts_dir_with_nested_entry() {
    let root = temp_path("scripts-dir-nested");
    fs::create_dir_all(&root).expect("root");
    write_file(
        &root.join("game.script.xml"),
        r#"<script name="game"><text>Game</text></script>"#,
    );

    let loaded =
        load_source_by_scripts_dir(&root.to_string_lossy(), "game").expect("load should pass");
    assert_eq!(loaded.entry_script, "game");
}

#[test]
fn load_source_by_ref_validates_prefix() {
    let error = load_source_by_ref("invalid").expect_err("no prefix should fail");
    assert_eq!(error.code, "CLI_SOURCE_REF_INVALID");

    let error = load_source_by_ref("").expect_err("empty ref should fail");
    assert_eq!(error.code, "CLI_SOURCE_REF_INVALID");
}

#[test]
fn make_scripts_dir_scenario_id_generates_consistent_ids() {
    let root = temp_path("scenario-id-test");
    fs::create_dir_all(&root).expect("root");

    let id1 = make_scripts_dir_scenario_id(&root);
    let id2 = make_scripts_dir_scenario_id(&root);
    assert_eq!(id1, id2);
    assert!(id1.starts_with("scripts-dir:"));
}

#[test]
fn run_entry_and_command_helpers_paths_are_covered() {
    let choice_scenario = example_scripts_dir("06-snapshot-flow");
    let input_scenario = example_scripts_dir("16-input-name");
    let text_scenario = example_scripts_dir("01-text-code");

    let start_state = temp_path("run-start-state.json");
    let choose_state = temp_path("run-choose-state.json");
    let input_state_1 = temp_path("run-input-state-1.json");
    let input_state_2 = temp_path("run-input-state-2.json");
    let tui_state = temp_path("run-tui-state.json");
    let tui_state_str = tui_state.to_string_lossy().to_string();

    let start_code = run(Cli {
        command: Mode::Agent(AgentArgs {
            command: AgentCommand::Start(StartArgs {
                scripts_dir: choice_scenario.clone(),
                entry_script: Some("main".to_string()),
                state_out: start_state.to_string_lossy().to_string(),
            }),
        }),
    })
    .expect("agent start should pass");
    assert_eq!(start_code, 0);

    let choose_code = run_agent(AgentArgs {
        command: AgentCommand::Choose(ChooseArgs {
            state_in: start_state.to_string_lossy().to_string(),
            choice: 0,
            state_out: choose_state.to_string_lossy().to_string(),
        }),
    })
    .expect("agent choose should pass");
    assert_eq!(choose_code, 0);

    let input_start_code = agent::run_start(StartArgs {
        scripts_dir: input_scenario.clone(),
        entry_script: Some("main".to_string()),
        state_out: input_state_1.to_string_lossy().to_string(),
    })
    .expect("input scenario start should pass");
    assert_eq!(input_start_code, 0);

    let input_code = agent::run_input(InputArgs {
        state_in: input_state_1.to_string_lossy().to_string(),
        text: "Guild".to_string(),
        state_out: input_state_2.to_string_lossy().to_string(),
    })
    .expect("agent input should pass");
    assert_eq!(input_code, 0);

    let tui_code = run_tui(TuiArgs {
        scripts_dir: text_scenario,
        entry_script: Some("main".to_string()),
        state_file: Some(tui_state_str.clone()),
    })
    .expect("tui should pass in line mode");
    assert_eq!(tui_code, 0);

    let loaded = load_source_by_scripts_dir(&choice_scenario, "main").expect("load source");
    let mut engine = create_engine_from_xml(CreateEngineFromXmlOptions {
        scripts_xml: loaded.scripts_xml.clone(),
        entry_script: Some("main".to_string()),
        entry_args: None,
        host_functions: None,
        random_seed: Some(1),
        compiler_version: Some(DEFAULT_COMPILER_VERSION.to_string()),
    })
    .expect("engine should build");
    let _ = run_to_boundary(&mut engine).expect("boundary");

    let mut emitted = Vec::new();
    let mut emit = |line: String| emitted.push(line);
    let action = handle_tui_command(
        ":help",
        &tui_state_str,
        &loaded,
        "main",
        &mut engine,
        &mut emit,
    )
    .expect("command should pass");
    assert_eq!(action, TuiCommandAction::Continue);
    let context = TuiCommandContext {
        state_file: &tui_state_str,
        scenario: &loaded,
        entry_script: "main",
    };
    let action = handle_line_cmd(":quit", &context, &mut engine, &mut emit)
        .expect("line command should pass");
    assert_eq!(action, TuiCommandAction::Quit);
}

#[test]
fn parse_error_and_error_mapper_helpers_are_covered() {
    let parse_code = run_cli_from_args(["scriptlang-player", "agent", "unknown"]);
    assert_ne!(parse_code, 0);

    let io_error = std::io::Error::other("io");
    assert_eq!(map_tui_io(io_error).code, "TUI_IO");

    let io_error = std::io::Error::other("path");
    assert_eq!(map_cli_source_path(io_error).code, "CLI_SOURCE_PATH");

    let strip_error = Path::new("/a")
        .strip_prefix("/b")
        .expect_err("strip prefix");
    assert_eq!(map_cli_source_scan(strip_error).code, "CLI_SOURCE_SCAN");

    let io_error = std::io::Error::other("read");
    assert_eq!(map_cli_source_read(io_error).code, "CLI_SOURCE_READ");

    let io_error = std::io::Error::other("write");
    assert_eq!(map_cli_state_write(io_error).code, "CLI_STATE_WRITE");

    let io_error = std::io::Error::other("read");
    assert_eq!(map_cli_state_read(io_error).code, "CLI_STATE_READ");

    let invalid = serde_json::from_str::<serde_json::Value>("{").expect_err("invalid json");
    assert_eq!(map_cli_state_invalid(invalid).code, "CLI_STATE_INVALID");
}
