use std::path::Path;

use sl_api::EngineOutput;
use sl_api::ScriptLangEngine;
use sl_api::ScriptLangError;
use sl_api::DEFAULT_COMPILER_VERSION;

use crate::{
    create_engine_for_scenario, emit_boundary_with_saved_state, load_player_state,
    load_source_by_ref, load_source_by_scripts_dir, parse_rand_sequence, resume_engine_for_state,
    run_to_boundary, AgentArgs, AgentCommand, ChooseArgs, CompileArgs, InputArgs, RandConfig,
    ReplayArgs, StartArgs,
};

pub(super) fn run_agent(args: AgentArgs) -> Result<i32, ScriptLangError> {
    match args.command {
        AgentCommand::Start(args) => run_start(args),
        AgentCommand::Choose(args) => run_choose(args),
        AgentCommand::Input(args) => run_input(args),
        AgentCommand::Replay(args) => run_replay(args),
        AgentCommand::Compile(args) => run_compile(args),
    }
}

pub(super) fn run_start(args: StartArgs) -> Result<i32, ScriptLangError> {
    let scenario = load_source_by_scripts_dir(
        &args.scripts_dir,
        args.entry_script.as_deref().unwrap_or("main.main"),
    )?;
    let random_sequence = parse_rand_sequence(args.rand.as_deref())?;
    let mut engine = create_engine_for_scenario(
        &scenario,
        &scenario.entry_script,
        RandConfig {
            sequence: random_sequence,
            sequence_index: Some(0),
            seed_state: None,
        },
    )?;

    let boundary = run_to_boundary(&mut engine, args.show_debug)?;
    emit_boundary_with_saved_state(
        &engine,
        boundary,
        &args.state_out,
        &scenario.id,
        DEFAULT_COMPILER_VERSION,
    )
}

pub(super) fn run_choose(args: ChooseArgs) -> Result<i32, ScriptLangError> {
    let random_sequence = parse_rand_sequence(args.rand.as_deref())?;
    run_state_transition(
        &args.state_in,
        &args.state_out,
        random_sequence,
        args.show_debug,
        |engine| engine.choose(args.choice),
    )
}

pub(super) fn run_input(args: InputArgs) -> Result<i32, ScriptLangError> {
    let random_sequence = parse_rand_sequence(args.rand.as_deref())?;
    run_state_transition(
        &args.state_in,
        &args.state_out,
        random_sequence,
        args.show_debug,
        |engine| engine.submit_input(&args.text),
    )
}

pub(super) fn run_compile(args: CompileArgs) -> Result<i32, ScriptLangError> {
    use sl_api::compile_artifact_from_xml_map;
    use sl_api::write_artifact_json;

    // 1. 加载源文件
    let scenario = load_source_by_scripts_dir(
        &args.scripts_dir,
        args.entry_script.as_deref().unwrap_or("main.main"),
    )?;

    // 2. 编译（在内存中进行）
    let artifact =
        compile_artifact_from_xml_map(&scenario.scripts_xml, Some(scenario.entry_script.clone()))?;

    // 3. 根据 dry_run 决定是否写入
    if args.dry_run {
        // Dry-run 模式：只打印摘要信息
        println!("Compilation successful (dry-run):");
        println!("  Entry: {}", artifact.entry_script);
        println!("  Scripts: {}", artifact.scripts.len());
        println!(
            "  Defs global declarations: {}",
            artifact.defs_global_declarations.len()
        );
    } else {
        // 正常模式：写入文件
        let output_path = args.output.ok_or_else(|| {
            ScriptLangError::new(
                "CLI_OUTPUT_REQUIRED",
                "--output is required when not using --dry-run".to_string(),
            )
        })?;
        write_artifact_json(std::path::Path::new(&output_path), &artifact)?;
        println!("Artifact written to: {}", output_path);
    }

    Ok(0)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ReplayAction {
    Choose(usize),
    Input(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReplayStopAt {
    Choices,
    Input,
    End,
}

impl ReplayStopAt {
    fn as_label(self) -> &'static str {
        match self {
            Self::Choices => "CHOICES",
            Self::Input => "INPUT",
            Self::End => "END",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReplayResult {
    lines: Vec<String>,
    actions_used: usize,
    actions_total: usize,
    stop_at: ReplayStopAt,
}

pub(super) fn run_replay(args: ReplayArgs) -> Result<i32, ScriptLangError> {
    let entry_script = args.entry_script.unwrap_or("main.main".to_string());
    let scenario = load_source_by_scripts_dir(&args.scripts_dir, &entry_script)?;
    let random_sequence = parse_rand_sequence(args.rand.as_deref())?;
    let mut engine = create_engine_for_scenario(
        &scenario,
        &entry_script,
        RandConfig {
            sequence: random_sequence,
            sequence_index: Some(0),
            seed_state: None,
        },
    )?;
    let actions = parse_replay_steps(&args.step)?;
    let result = run_replay_sequence(&mut engine, &actions, args.show_debug)?;

    println!("RESULT:OK");
    println!("MODE:REPLAY");
    for line in result.lines {
        println!("{}", line);
    }
    println!("ACTIONS_USED: {}", result.actions_used);
    println!("ACTIONS_TOTAL: {}", result.actions_total);
    println!("STOP_AT: {}", result.stop_at.as_label());
    Ok(0)
}

fn parse_replay_steps(steps: &[String]) -> Result<Vec<ReplayAction>, ScriptLangError> {
    let mut actions = Vec::with_capacity(steps.len());
    for step in steps {
        let (kind, payload) = step.split_once(':').ok_or_else(|| {
            ScriptLangError::new(
                "CLI_REPLAY_STEP_INVALID",
                format!(
                    "Invalid replay step \"{}\". Expected format \"choose:<index>\" or \"input:<text>\".",
                    step
                ),
            )
        })?;
        match kind {
            "choose" => {
                let index = payload.parse::<usize>().map_err(|_| {
                    ScriptLangError::new(
                        "CLI_REPLAY_STEP_INVALID",
                        format!("Invalid choose index in replay step \"{}\".", step),
                    )
                })?;
                actions.push(ReplayAction::Choose(index));
            }
            "input" => actions.push(ReplayAction::Input(payload.to_string())),
            _ => {
                return Err(ScriptLangError::new(
                    "CLI_REPLAY_STEP_INVALID",
                    format!(
                        "Unsupported replay step kind \"{}\" in \"{}\". Expected choose/input.",
                        kind, step
                    ),
                ))
            }
        }
    }
    Ok(actions)
}

fn run_replay_sequence(
    engine: &mut ScriptLangEngine,
    actions: &[ReplayAction],
    show_debug: bool,
) -> Result<ReplayResult, ScriptLangError> {
    let mut lines = Vec::new();
    let mut action_index = 0usize;

    loop {
        match engine.next_output()? {
            EngineOutput::Text { text, tag } => {
                lines.push(format!("TEXT: {}", text));
                if let Some(tag) = tag {
                    lines.push(format!("TEXT_TAG: {}", tag));
                }
            }
            EngineOutput::Debug { text } => {
                if show_debug {
                    lines.push(format!("DEBUG: {}", text));
                }
            }
            EngineOutput::Choices { items, prompt_text } => {
                lines.push(format!("CHOICES: {}", prompt_text.unwrap_or_default()));
                for item in items {
                    lines.push(format!("- [{}] {}", item.index, item.text));
                }
                let Some(action) = actions.get(action_index) else {
                    return Ok(ReplayResult {
                        lines,
                        actions_used: action_index,
                        actions_total: actions.len(),
                        stop_at: ReplayStopAt::Choices,
                    });
                };
                match action {
                    ReplayAction::Choose(index) => {
                        lines.push(format!("APPLY: choose:{}", index));
                        engine.choose(*index)?;
                        action_index += 1;
                    }
                    ReplayAction::Input(text) => {
                        return Err(ScriptLangError::new(
                            "CLI_REPLAY_ACTION_KIND_MISMATCH",
                            format!(
                                "Replay action mismatch at index {}. Expected choose action, got input:{}.",
                                action_index, text
                            ),
                        ))
                    }
                }
            }
            EngineOutput::Input {
                prompt_text,
                default_text,
            } => {
                lines.push(format!("INPUT: {}", prompt_text));
                lines.push(format!("DEFAULT: {}", default_text));
                let Some(action) = actions.get(action_index) else {
                    return Ok(ReplayResult {
                        lines,
                        actions_used: action_index,
                        actions_total: actions.len(),
                        stop_at: ReplayStopAt::Input,
                    });
                };
                match action {
                    ReplayAction::Input(text) => {
                        lines.push(format!("APPLY: input:{}", text));
                        engine.submit_input(text)?;
                        action_index += 1;
                    }
                    ReplayAction::Choose(index) => {
                        return Err(ScriptLangError::new(
                            "CLI_REPLAY_ACTION_KIND_MISMATCH",
                            format!(
                                "Replay action mismatch at index {}. Expected input action, got choose:{}.",
                                action_index, index
                            ),
                        ))
                    }
                }
            }
            EngineOutput::End => {
                lines.push("END".to_string());
                if action_index != actions.len() {
                    return Err(ScriptLangError::new(
                        "CLI_REPLAY_UNUSED_ACTIONS",
                        format!(
                            "Replay ended with unused actions. used={} total={}",
                            action_index,
                            actions.len()
                        ),
                    ));
                }
                return Ok(ReplayResult {
                    lines,
                    actions_used: action_index,
                    actions_total: actions.len(),
                    stop_at: ReplayStopAt::End,
                });
            }
        }
    }
}

fn run_state_transition(
    state_in: &str,
    state_out: &str,
    random_sequence: Option<Vec<u32>>,
    show_debug: bool,
    transition: impl FnOnce(&mut sl_api::ScriptLangEngine) -> Result<(), ScriptLangError>,
) -> Result<i32, ScriptLangError> {
    let state = load_player_state(Path::new(state_in))?;
    let scenario = load_source_by_ref(&state.scenario_id)?;
    let mut engine = resume_engine_for_state(&scenario, &state, random_sequence)?;
    transition(&mut engine)?;
    let boundary = run_to_boundary(&mut engine, show_debug)?;
    emit_boundary_with_saved_state(
        &engine,
        boundary,
        state_out,
        &state.scenario_id,
        &state.compiler_version,
    )
}

#[cfg(test)]
mod agent_tests {
    use super::*;

    use crate::cli_test_support::{example_scripts_dir, temp_path, write_file};
    use std::fs;

    #[test]
    fn run_agent_dispatches_input_command() {
        let scripts_dir = example_scripts_dir("16-input-name");
        let state_in = temp_path("agent-input-state-in.json");
        let state_out = temp_path("agent-input-state-out.json");

        run_start(StartArgs {
            scripts_dir,
            entry_script: Some("main.main".to_string()),
            state_out: state_in.to_string_lossy().to_string(),
            rand: None,
            show_debug: false,
        })
        .expect("start should pass");

        let code = run_agent(AgentArgs {
            command: AgentCommand::Input(InputArgs {
                state_in: state_in.to_string_lossy().to_string(),
                text: "Guild".to_string(),
                state_out: state_out.to_string_lossy().to_string(),
                rand: None,
                show_debug: false,
            }),
        })
        .expect("input dispatch should pass");

        assert_eq!(code, 0);
    }

    #[test]
    fn run_replay_text_only_to_end() {
        let scripts_dir = example_scripts_dir("01-text-code");
        let args = ReplayArgs {
            scripts_dir,
            entry_script: Some("main.main".to_string()),
            step: Vec::new(),
            rand: None,
            show_debug: false,
        };
        let code = run_replay(args).expect("replay should pass");
        assert_eq!(code, 0);
    }

    #[test]
    fn run_replay_hides_or_shows_debug_lines_by_flag() {
        let root = temp_path("agent-replay-debug-flag");
        fs::create_dir_all(&root).expect("root should be created");
        write_file(
            &root.join("main.xml"),
            r#"<module name="main" default_access="public">
<script name="main"><debug>dbg=${1+1}</debug><text>ok</text></script>
</module>"#,
        );
        let scenario = load_source_by_scripts_dir(root.to_string_lossy().as_ref(), "main.main")
            .expect("scenario");
        let mut engine = create_engine_for_scenario(&scenario, "main.main", RandConfig::default())
            .expect("engine");
        let hidden = run_replay_sequence(&mut engine, &[], false).expect("hidden replay");
        assert!(hidden.lines.iter().all(|line| !line.starts_with("DEBUG: ")));

        let mut engine = create_engine_for_scenario(&scenario, "main.main", RandConfig::default())
            .expect("engine");
        let shown = run_replay_sequence(&mut engine, &[], true).expect("shown replay");
        assert!(shown.lines.iter().any(|line| line == "DEBUG: dbg=2"));
    }

    #[test]
    fn run_replay_consumes_choose_and_input_steps() {
        let root = temp_path("agent-replay-choose-input");
        fs::create_dir_all(&root).expect("root should be created");
        write_file(
            &root.join("main.xml"),
            r#"
<module name="main" default_access="public">
<script name="main">
  <choice text="Pick">
    <option text="A"><text>A</text></option>
  </choice>
  <temp name="name" type="string">"Traveler"</temp>
  <input var="name" text="Name"/>
  <text>${name}</text>
</script>
</module>"#,
        );

        let mut engine = create_engine_for_scenario(
            &load_source_by_scripts_dir(root.to_string_lossy().as_ref(), "main.main")
                .expect("scenario should load"),
            "main.main",
            RandConfig::default(),
        )
        .expect("engine should create");
        let actions = parse_replay_steps(&["choose:0".to_string(), "input:Guild".to_string()])
            .expect("steps should parse");
        let result = run_replay_sequence(&mut engine, &actions, false).expect("replay should pass");
        assert_eq!(result.actions_used, 2);
        assert_eq!(result.actions_total, 2);
        assert_eq!(result.stop_at, ReplayStopAt::End);
        assert!(result.lines.iter().any(|line| line == "APPLY: choose:0"));
        assert!(result.lines.iter().any(|line| line == "APPLY: input:Guild"));
        assert!(result.lines.iter().any(|line| line == "END"));
    }

    #[test]
    fn run_replay_stops_at_next_boundary_when_steps_exhausted() {
        let root = temp_path("agent-replay-next-boundary");
        fs::create_dir_all(&root).expect("root should be created");
        write_file(
            &root.join("main.xml"),
            r#"
<module name="main" default_access="public">
<script name="main">
  <choice text="Pick">
    <option text="A"><text>A</text></option>
  </choice>
  <temp name="name" type="string">"Traveler"</temp>
  <input var="name" text="Name"/>
</script>
</module>"#,
        );
        let scenario = load_source_by_scripts_dir(root.to_string_lossy().as_ref(), "main.main")
            .expect("scenario");
        let mut engine = create_engine_for_scenario(&scenario, "main.main", RandConfig::default())
            .expect("engine");
        let actions = parse_replay_steps(&["choose:0".to_string()]).expect("steps should parse");
        let result = run_replay_sequence(&mut engine, &actions, false).expect("replay should pass");
        assert_eq!(result.actions_used, 1);
        assert_eq!(result.actions_total, 1);
        assert_eq!(result.stop_at, ReplayStopAt::Input);
    }

    #[test]
    fn run_replay_reports_invalid_step_format() {
        let invalid = parse_replay_steps(&["xxx".to_string()]).expect_err("invalid should fail");
        assert_eq!(invalid.code, "CLI_REPLAY_STEP_INVALID");

        let bad_choose =
            parse_replay_steps(&["choose:abc".to_string()]).expect_err("choose parse should fail");
        assert_eq!(bad_choose.code, "CLI_REPLAY_STEP_INVALID");
    }

    #[test]
    fn run_replay_reports_action_kind_mismatch() {
        let scripts_dir = example_scripts_dir("16-input-name");
        let scenario = load_source_by_scripts_dir(&scripts_dir, "main.main").expect("scenario");
        let mut engine = create_engine_for_scenario(&scenario, "main.main", RandConfig::default())
            .expect("engine");
        let actions = parse_replay_steps(&["choose:0".to_string()]).expect("steps should parse");
        let error =
            run_replay_sequence(&mut engine, &actions, false).expect_err("mismatch should fail");
        assert_eq!(error.code, "CLI_REPLAY_ACTION_KIND_MISMATCH");
    }

    #[test]
    fn run_replay_reports_unused_actions_if_end_reached_early() {
        let scripts_dir = example_scripts_dir("01-text-code");
        let scenario = load_source_by_scripts_dir(&scripts_dir, "main.main").expect("scenario");
        let mut engine = create_engine_for_scenario(&scenario, "main.main", RandConfig::default())
            .expect("engine");
        let actions = parse_replay_steps(&["input:Guild".to_string()]).expect("steps should parse");
        let error =
            run_replay_sequence(&mut engine, &actions, false).expect_err("unused should fail");
        assert_eq!(error.code, "CLI_REPLAY_UNUSED_ACTIONS");
    }

    #[test]
    fn start_choose_keeps_rand_sequence_progress_in_state() {
        let root = temp_path("agent-rand-sequence-progress");
        fs::create_dir_all(&root).expect("root should be created");
        write_file(
            &root.join("main.xml"),
            r#"
<module name="main" default_access="public">
<script name="main">
  <choice text="Pick">
    <option text="A">
      <temp name="r" type="int">random(10)</temp>
      <text>${r}</text>
      <temp name="name" type="string">"Traveler"</temp>
      <input var="name" text="Name"/>
    </option>
  </choice>
</script>
</module>"#,
        );

        let state_1 = temp_path("agent-rand-state-1.json");
        run_start(StartArgs {
            scripts_dir: root.to_string_lossy().to_string(),
            entry_script: Some("main.main".to_string()),
            state_out: state_1.to_string_lossy().to_string(),
            rand: Some("12,3".to_string()),
            show_debug: false,
        })
        .expect("start should pass");

        let state_2 = temp_path("agent-rand-state-2.json");
        run_choose(ChooseArgs {
            state_in: state_1.to_string_lossy().to_string(),
            choice: 0,
            state_out: state_2.to_string_lossy().to_string(),
            rand: None,
            show_debug: false,
        })
        .expect("choose should pass");

        let state = load_player_state(state_2.as_path()).expect("state should load");
        assert_eq!(state.random_mode, crate::PlayerRandomMode::Sequence);
        assert_eq!(state.random_sequence, vec![12, 3]);
        assert_eq!(state.random_sequence_index, Some(1));
    }

    #[test]
    fn choose_rand_argument_overrides_state_random_sequence() {
        let root = temp_path("agent-rand-override");
        fs::create_dir_all(&root).expect("root should be created");
        write_file(
            &root.join("main.xml"),
            r#"
<module name="main" default_access="public">
<script name="main">
  <choice text="Pick">
    <option text="A">
      <temp name="r" type="int">random(10)</temp>
      <text>${r}</text>
      <temp name="name" type="string">"Traveler"</temp>
      <input var="name" text="Name"/>
    </option>
  </choice>
</script>
</module>"#,
        );

        let state_1 = temp_path("agent-rand-override-state-1.json");
        run_start(StartArgs {
            scripts_dir: root.to_string_lossy().to_string(),
            entry_script: Some("main.main".to_string()),
            state_out: state_1.to_string_lossy().to_string(),
            rand: Some("12,3".to_string()),
            show_debug: false,
        })
        .expect("start should pass");

        let state_2 = temp_path("agent-rand-override-state-2.json");
        run_choose(ChooseArgs {
            state_in: state_1.to_string_lossy().to_string(),
            choice: 0,
            state_out: state_2.to_string_lossy().to_string(),
            rand: Some("9,8,7".to_string()),
            show_debug: false,
        })
        .expect("choose should pass");

        let state = load_player_state(state_2.as_path()).expect("state should load");
        assert_eq!(state.random_mode, crate::PlayerRandomMode::Sequence);
        assert_eq!(state.random_sequence, vec![9, 8, 7]);
        assert_eq!(state.random_sequence_index, Some(1));
    }
}
