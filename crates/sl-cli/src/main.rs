use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand};
use serde::{Deserialize, Serialize};
use sl_api::{
    create_engine_from_xml, resume_engine_from_xml, CreateEngineFromXmlOptions,
    ResumeEngineFromXmlOptions,
};
use sl_core::{EngineOutput, ScriptLangError, SlValue, SnapshotV3};
use sl_runtime::DEFAULT_COMPILER_VERSION;
use walkdir::WalkDir;

const PLAYER_STATE_SCHEMA: &str = "player-state.v3";

#[derive(Debug, Parser)]
#[command(name = "scriptlang-player")]
#[command(about = "ScriptLang Rust agent CLI")]
struct Cli {
    #[command(subcommand)]
    command: Mode,
}

#[derive(Debug, Subcommand)]
enum Mode {
    Agent(AgentArgs),
    Tui(TuiArgs),
}

#[derive(Debug, Args)]
struct AgentArgs {
    #[command(subcommand)]
    command: AgentCommand,
}

#[derive(Debug, Subcommand)]
enum AgentCommand {
    Start(StartArgs),
    Choose(ChooseArgs),
    Input(InputArgs),
}

#[derive(Debug, Args)]
struct StartArgs {
    #[arg(long = "scripts-dir")]
    scripts_dir: String,
    #[arg(long = "entry-script")]
    entry_script: Option<String>,
    #[arg(long = "state-out")]
    state_out: String,
}

#[derive(Debug, Args)]
struct ChooseArgs {
    #[arg(long = "state-in")]
    state_in: String,
    #[arg(long = "choice")]
    choice: usize,
    #[arg(long = "state-out")]
    state_out: String,
}

#[derive(Debug, Args)]
struct InputArgs {
    #[arg(long = "state-in")]
    state_in: String,
    #[arg(long = "text")]
    text: String,
    #[arg(long = "state-out")]
    state_out: String,
}

#[derive(Debug, Args)]
struct TuiArgs {
    #[arg(long = "scripts-dir")]
    scripts_dir: String,
    #[arg(long = "entry-script")]
    entry_script: Option<String>,
    #[arg(long = "state-file")]
    state_file: Option<String>,
}

#[derive(Debug, Clone)]
struct LoadedScenario {
    id: String,
    scripts_xml: BTreeMap<String, String>,
    entry_script: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlayerStateV3 {
    schema_version: String,
    scenario_id: String,
    compiler_version: String,
    snapshot: SnapshotV3,
}

#[derive(Debug, Clone)]
enum BoundaryEvent {
    Choices,
    Input,
    End,
}

#[derive(Debug, Clone)]
struct BoundaryResult {
    event: BoundaryEvent,
    texts: Vec<String>,
    choices: Vec<(usize, String)>,
    choice_prompt_text: Option<String>,
    input_prompt_text: Option<String>,
    input_default_text: Option<String>,
}

fn main() {
    let cli = Cli::parse();
    let exit_code = match run(cli) {
        Ok(code) => code,
        Err(error) => emit_error(error),
    };

    std::process::exit(exit_code);
}

fn run(cli: Cli) -> Result<i32, ScriptLangError> {
    match cli.command {
        Mode::Agent(args) => run_agent(args),
        Mode::Tui(args) => run_tui(args),
    }
}

fn run_agent(args: AgentArgs) -> Result<i32, ScriptLangError> {
    match args.command {
        AgentCommand::Start(args) => run_start(args),
        AgentCommand::Choose(args) => run_choose(args),
        AgentCommand::Input(args) => run_input(args),
    }
}

fn run_tui(args: TuiArgs) -> Result<i32, ScriptLangError> {
    let entry_script = args.entry_script.unwrap_or_else(|| "main".to_string());
    let state_file = args
        .state_file
        .unwrap_or_else(|| ".scriptlang/save.json".to_string());
    let scenario = load_source_by_scripts_dir(&args.scripts_dir, &entry_script)?;
    let mut engine = create_engine_from_xml(CreateEngineFromXmlOptions {
        scripts_xml: scenario.scripts_xml.clone(),
        entry_script: Some(entry_script.clone()),
        entry_args: None,
        host_functions: None,
        random_seed: None,
        compiler_version: Some(DEFAULT_COMPILER_VERSION.to_string()),
    })?;

    println!("ScriptLang TUI");
    println!("commands: :help :save :load :restart :quit");

    loop {
        match engine.next()? {
            EngineOutput::Text { text } => {
                println!();
                println!("{}", text);
            }
            EngineOutput::Choices { items, prompt_text } => {
                println!();
                if let Some(prompt_text) = prompt_text {
                    println!("{}", prompt_text);
                }
                for item in &items {
                    println!("  [{}] {}", item.index, item.text);
                }
                loop {
                    let raw = prompt_input("> ")?;
                    if handle_tui_command(
                        raw.as_str(),
                        &state_file,
                        &scenario,
                        &entry_script,
                        &mut engine,
                    )? {
                        continue;
                    }
                    let choice = raw.parse::<usize>().map_err(|_| {
                        ScriptLangError::new(
                            "TUI_CHOICE_PARSE",
                            format!("Invalid choice index: {}", raw),
                        )
                    })?;
                    engine.choose(choice)?;
                    break;
                }
            }
            EngineOutput::Input {
                prompt_text,
                default_text,
            } => {
                println!();
                println!("{}", prompt_text);
                println!("(default: {})", default_text);
                loop {
                    let raw = prompt_input("> ")?;
                    if handle_tui_command(
                        raw.as_str(),
                        &state_file,
                        &scenario,
                        &entry_script,
                        &mut engine,
                    )? {
                        continue;
                    }
                    engine.submit_input(&raw)?;
                    break;
                }
            }
            EngineOutput::End => {
                println!();
                println!("[END]");
                return Ok(0);
            }
        }
    }
}

fn handle_tui_command(
    raw: &str,
    state_file: &str,
    scenario: &LoadedScenario,
    entry_script: &str,
    engine: &mut sl_runtime::ScriptLangEngine,
) -> Result<bool, ScriptLangError> {
    match raw {
        ":help" => {
            println!("commands: :help :save :load :restart :quit");
            Ok(true)
        }
        ":save" => {
            let snapshot = engine.snapshot()?;
            let state = PlayerStateV3 {
                schema_version: PLAYER_STATE_SCHEMA.to_string(),
                scenario_id: scenario.id.clone(),
                compiler_version: DEFAULT_COMPILER_VERSION.to_string(),
                snapshot,
            };
            save_player_state(Path::new(state_file), &state)?;
            println!("saved: {}", state_file);
            Ok(true)
        }
        ":load" => {
            let state = load_player_state(Path::new(state_file))?;
            let loaded = load_source_by_ref(&state.scenario_id)?;
            let resumed = resume_engine_from_xml(ResumeEngineFromXmlOptions {
                scripts_xml: loaded.scripts_xml,
                snapshot: state.snapshot,
                host_functions: None,
                compiler_version: Some(state.compiler_version),
            })?;
            *engine = resumed;
            println!("loaded: {}", state_file);
            Ok(true)
        }
        ":restart" => {
            let mut restarted = create_engine_from_xml(CreateEngineFromXmlOptions {
                scripts_xml: scenario.scripts_xml.clone(),
                entry_script: Some(entry_script.to_string()),
                entry_args: None,
                host_functions: None,
                random_seed: None,
                compiler_version: Some(DEFAULT_COMPILER_VERSION.to_string()),
            })?;
            std::mem::swap(engine, &mut restarted);
            println!("restarted");
            Ok(true)
        }
        ":quit" => {
            println!("bye");
            std::process::exit(0);
        }
        _ => Ok(false),
    }
}

fn prompt_input(prefix: &str) -> Result<String, ScriptLangError> {
    print!("{}", prefix);
    io::stdout()
        .flush()
        .map_err(|error| ScriptLangError::new("TUI_IO", error.to_string()))?;
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .map_err(|error| ScriptLangError::new("TUI_IO", error.to_string()))?;
    Ok(input.trim_end_matches(&['\r', '\n'][..]).to_string())
}

fn run_start(args: StartArgs) -> Result<i32, ScriptLangError> {
    let scenario = load_source_by_scripts_dir(
        &args.scripts_dir,
        args.entry_script.as_deref().unwrap_or("main"),
    )?;

    let mut engine = create_engine_from_xml(CreateEngineFromXmlOptions {
        scripts_xml: scenario.scripts_xml.clone(),
        entry_script: Some(scenario.entry_script.clone()),
        entry_args: None,
        host_functions: None,
        random_seed: None,
        compiler_version: Some(DEFAULT_COMPILER_VERSION.to_string()),
    })?;

    let boundary = run_to_boundary(&mut engine)?;

    if matches!(
        boundary.event,
        BoundaryEvent::Choices | BoundaryEvent::Input
    ) {
        let snapshot = engine.snapshot()?;
        let state = PlayerStateV3 {
            schema_version: PLAYER_STATE_SCHEMA.to_string(),
            scenario_id: scenario.id,
            compiler_version: DEFAULT_COMPILER_VERSION.to_string(),
            snapshot,
        };
        save_player_state(Path::new(&args.state_out), &state)?;
        emit_boundary(boundary, Some(args.state_out));
        return Ok(0);
    }

    emit_boundary(boundary, None);
    Ok(0)
}

fn run_choose(args: ChooseArgs) -> Result<i32, ScriptLangError> {
    let state = load_player_state(Path::new(&args.state_in))?;
    let scenario = load_source_by_ref(&state.scenario_id)?;

    let mut engine = resume_engine_from_xml(ResumeEngineFromXmlOptions {
        scripts_xml: scenario.scripts_xml.clone(),
        snapshot: state.snapshot,
        host_functions: None,
        compiler_version: Some(state.compiler_version.clone()),
    })?;

    engine.choose(args.choice)?;
    let boundary = run_to_boundary(&mut engine)?;

    if matches!(
        boundary.event,
        BoundaryEvent::Choices | BoundaryEvent::Input
    ) {
        let next_state = PlayerStateV3 {
            schema_version: PLAYER_STATE_SCHEMA.to_string(),
            scenario_id: state.scenario_id,
            compiler_version: state.compiler_version,
            snapshot: engine.snapshot()?,
        };
        save_player_state(Path::new(&args.state_out), &next_state)?;
        emit_boundary(boundary, Some(args.state_out));
        return Ok(0);
    }

    emit_boundary(boundary, None);
    Ok(0)
}

fn run_input(args: InputArgs) -> Result<i32, ScriptLangError> {
    let state = load_player_state(Path::new(&args.state_in))?;
    let scenario = load_source_by_ref(&state.scenario_id)?;

    let mut engine = resume_engine_from_xml(ResumeEngineFromXmlOptions {
        scripts_xml: scenario.scripts_xml.clone(),
        snapshot: state.snapshot,
        host_functions: None,
        compiler_version: Some(state.compiler_version.clone()),
    })?;

    engine.submit_input(&args.text)?;
    let boundary = run_to_boundary(&mut engine)?;

    if matches!(
        boundary.event,
        BoundaryEvent::Choices | BoundaryEvent::Input
    ) {
        let next_state = PlayerStateV3 {
            schema_version: PLAYER_STATE_SCHEMA.to_string(),
            scenario_id: state.scenario_id,
            compiler_version: state.compiler_version,
            snapshot: engine.snapshot()?,
        };
        save_player_state(Path::new(&args.state_out), &next_state)?;
        emit_boundary(boundary, Some(args.state_out));
        return Ok(0);
    }

    emit_boundary(boundary, None);
    Ok(0)
}

fn run_to_boundary(
    engine: &mut sl_runtime::ScriptLangEngine,
) -> Result<BoundaryResult, ScriptLangError> {
    let mut texts = Vec::new();

    loop {
        match engine.next()? {
            EngineOutput::Text { text } => texts.push(text),
            EngineOutput::Choices { items, prompt_text } => {
                return Ok(BoundaryResult {
                    event: BoundaryEvent::Choices,
                    texts,
                    choices: items
                        .into_iter()
                        .map(|item| (item.index, item.text))
                        .collect(),
                    choice_prompt_text: prompt_text,
                    input_prompt_text: None,
                    input_default_text: None,
                })
            }
            EngineOutput::Input {
                prompt_text,
                default_text,
            } => {
                return Ok(BoundaryResult {
                    event: BoundaryEvent::Input,
                    texts,
                    choices: Vec::new(),
                    choice_prompt_text: None,
                    input_prompt_text: Some(prompt_text),
                    input_default_text: Some(default_text),
                })
            }
            EngineOutput::End => {
                return Ok(BoundaryResult {
                    event: BoundaryEvent::End,
                    texts,
                    choices: Vec::new(),
                    choice_prompt_text: None,
                    input_prompt_text: None,
                    input_default_text: None,
                })
            }
        }
    }
}

fn emit_boundary(boundary: BoundaryResult, state_out: Option<String>) {
    println!("RESULT:OK");
    match boundary.event {
        BoundaryEvent::Choices => println!("EVENT:CHOICES"),
        BoundaryEvent::Input => println!("EVENT:INPUT"),
        BoundaryEvent::End => println!("EVENT:END"),
    }

    for text in boundary.texts {
        println!(
            "TEXT_JSON:{}",
            serde_json::to_string(&text).unwrap_or_else(|_| "\"\"".to_string())
        );
    }

    if let Some(prompt) = boundary.choice_prompt_text {
        println!(
            "PROMPT_JSON:{}",
            serde_json::to_string(&prompt).unwrap_or_else(|_| "\"\"".to_string())
        );
    }

    if let Some(prompt) = boundary.input_prompt_text {
        println!(
            "PROMPT_JSON:{}",
            serde_json::to_string(&prompt).unwrap_or_else(|_| "\"\"".to_string())
        );
    }

    for (index, text) in boundary.choices {
        println!(
            "CHOICE:{}|{}",
            index,
            serde_json::to_string(&text).unwrap_or_else(|_| "\"\"".to_string())
        );
    }

    if let Some(default_text) = boundary.input_default_text {
        println!(
            "INPUT_DEFAULT_JSON:{}",
            serde_json::to_string(&default_text).unwrap_or_else(|_| "\"\"".to_string())
        );
    }

    println!(
        "STATE_OUT:{}",
        state_out.unwrap_or_else(|| "NONE".to_string())
    );
}

fn emit_error(error: ScriptLangError) -> i32 {
    println!("RESULT:ERROR");
    println!("ERROR_CODE:{}", error.code);
    println!(
        "ERROR_MSG_JSON:{}",
        serde_json::to_string(&error.message).unwrap_or_else(|_| "\"Unknown error\"".to_string())
    );
    1
}

fn load_source_by_scripts_dir(
    scripts_dir: &str,
    entry_script: &str,
) -> Result<LoadedScenario, ScriptLangError> {
    let scripts_root = resolve_scripts_dir(scripts_dir)?;
    let scripts_xml = read_scripts_xml_from_dir(&scripts_root)?;
    let scenario_id = make_scripts_dir_scenario_id(&scripts_root);

    Ok(LoadedScenario {
        id: scenario_id,
        scripts_xml,
        entry_script: entry_script.to_string(),
    })
}

fn load_source_by_ref(scenario_ref: &str) -> Result<LoadedScenario, ScriptLangError> {
    let prefix = "scripts-dir:";
    if !scenario_ref.starts_with(prefix) {
        return Err(ScriptLangError::new(
            "CLI_SOURCE_REF_INVALID",
            format!("Unsupported scenario ref: {}", scenario_ref),
        ));
    }

    let raw = scenario_ref.trim_start_matches(prefix);
    load_source_by_scripts_dir(raw, "main")
}

fn resolve_scripts_dir(scripts_dir: &str) -> Result<PathBuf, ScriptLangError> {
    let path = PathBuf::from(scripts_dir);
    let absolute = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .map_err(|error| ScriptLangError::new("CLI_SOURCE_PATH", error.to_string()))?
            .join(path)
    };

    if !absolute.exists() {
        return Err(ScriptLangError::new(
            "CLI_SOURCE_NOT_FOUND",
            format!("scripts-dir does not exist: {}", absolute.display()),
        ));
    }

    if !absolute.is_dir() {
        return Err(ScriptLangError::new(
            "CLI_SOURCE_NOT_DIR",
            format!("scripts-dir is not a directory: {}", absolute.display()),
        ));
    }

    Ok(absolute)
}

fn read_scripts_xml_from_dir(
    scripts_dir: &Path,
) -> Result<BTreeMap<String, String>, ScriptLangError> {
    let mut scripts = BTreeMap::new();

    for entry in WalkDir::new(scripts_dir)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        let Some(path_str) = path.to_str() else {
            continue;
        };

        if !(path_str.ends_with(".script.xml")
            || path_str.ends_with(".defs.xml")
            || path_str.ends_with(".json"))
        {
            continue;
        }

        let relative = path
            .strip_prefix(scripts_dir)
            .map_err(|error| ScriptLangError::new("CLI_SOURCE_SCAN", error.to_string()))?
            .to_string_lossy()
            .replace('\\', "/");

        let content = fs::read_to_string(path)
            .map_err(|error| ScriptLangError::new("CLI_SOURCE_READ", error.to_string()))?;
        scripts.insert(relative, content);
    }

    if scripts.is_empty() {
        return Err(ScriptLangError::new(
            "CLI_SOURCE_EMPTY",
            format!(
                "No .script.xml/.defs.xml/.json files under {}",
                scripts_dir.display()
            ),
        ));
    }

    Ok(scripts)
}

fn make_scripts_dir_scenario_id(scripts_dir: &Path) -> String {
    format!("scripts-dir:{}", scripts_dir.display())
}

fn save_player_state(path: &Path, state: &PlayerStateV3) -> Result<(), ScriptLangError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .map_err(|error| ScriptLangError::new("CLI_STATE_WRITE", error.to_string()))?;

    let payload = serde_json::to_string(state)
        .map_err(|error| ScriptLangError::new("CLI_STATE_WRITE", error.to_string()))?;
    fs::write(path, payload)
        .map_err(|error| ScriptLangError::new("CLI_STATE_WRITE", error.to_string()))
}

fn load_player_state(path: &Path) -> Result<PlayerStateV3, ScriptLangError> {
    if !path.exists() {
        return Err(ScriptLangError::new(
            "CLI_STATE_NOT_FOUND",
            format!("State file does not exist: {}", path.display()),
        ));
    }

    let raw = fs::read_to_string(path)
        .map_err(|error| ScriptLangError::new("CLI_STATE_READ", error.to_string()))?;

    let state: PlayerStateV3 = serde_json::from_str(&raw)
        .map_err(|error| ScriptLangError::new("CLI_STATE_INVALID", error.to_string()))?;

    if state.schema_version != PLAYER_STATE_SCHEMA {
        return Err(ScriptLangError::new(
            "CLI_STATE_SCHEMA",
            format!("Unsupported player state schema: {}", state.schema_version),
        ));
    }

    Ok(state)
}

#[allow(dead_code)]
fn _silence_unused(_value: SlValue) {}
