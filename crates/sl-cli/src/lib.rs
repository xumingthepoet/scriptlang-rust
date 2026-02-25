#![allow(unexpected_cfgs)]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
#[cfg(not(coverage))]
use std::io::IsTerminal;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
#[cfg(not(coverage))]
use std::time::Duration;

use clap::{Args, Parser, Subcommand};
#[cfg(not(coverage))]
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
#[cfg(not(coverage))]
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
#[cfg(not(coverage))]
use crossterm::ExecutableCommand;
#[cfg(not(coverage))]
use ratatui::backend::CrosstermBackend;
#[cfg(not(coverage))]
use ratatui::layout::{Constraint, Direction, Layout};
#[cfg(not(coverage))]
use ratatui::style::{Color, Modifier, Style};
#[cfg(not(coverage))]
use ratatui::text::{Line, Span};
#[cfg(not(coverage))]
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
#[cfg(not(coverage))]
use ratatui::{Frame, Terminal};
use serde::{Deserialize, Serialize};
use sl_api::{
    create_engine_from_xml, resume_engine_from_xml, CreateEngineFromXmlOptions,
    ResumeEngineFromXmlOptions,
};
use sl_core::{EngineOutput, ScriptLangError, SnapshotV3};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[cfg(not(coverage))]
#[derive(Debug, Clone)]
enum TuiBoundary {
    Choices {
        prompt: Option<String>,
        items: Vec<(usize, String)>,
        selected: usize,
    },
    Input {
        prompt: String,
        default_text: String,
    },
    End,
}

#[cfg(not(coverage))]
#[derive(Debug)]
struct TuiUiState {
    logs: Vec<String>,
    input: String,
    status: String,
    boundary: TuiBoundary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TuiCommandAction {
    NotHandled,
    Continue,
    RefreshBoundary,
    Quit,
}

struct TuiCommandContext<'a> {
    state_file: &'a str,
    scenario: &'a LoadedScenario,
    entry_script: &'a str,
}

#[cfg(not(coverage))]
struct TuiTerminal {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
}

#[cfg(not(coverage))]
impl TuiTerminal {
    fn new() -> Result<Self, ScriptLangError> {
        enable_raw_mode().map_err(|error| ScriptLangError::new("TUI_IO", error.to_string()))?;
        io::stdout()
            .execute(EnterAlternateScreen)
            .map_err(|error| ScriptLangError::new("TUI_IO", error.to_string()))?;
        let backend = CrosstermBackend::new(io::stdout());
        let terminal = Terminal::new(backend)
            .map_err(|error| ScriptLangError::new("TUI_IO", error.to_string()))?;
        Ok(Self { terminal })
    }

    fn terminal_mut(&mut self) -> &mut Terminal<CrosstermBackend<io::Stdout>> {
        &mut self.terminal
    }
}

#[cfg(not(coverage))]
impl Drop for TuiTerminal {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = io::stdout().execute(LeaveAlternateScreen);
    }
}

pub fn run_cli_from_args<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(error) => return error.exit_code(),
    };
    match run(cli) {
        Ok(code) => code,
        Err(error) => emit_error(error),
    }
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

#[cfg(not(coverage))]
fn run_tui(args: TuiArgs) -> Result<i32, ScriptLangError> {
    let entry_script = args.entry_script.unwrap_or("main".to_string());
    let state_file = args
        .state_file
        .unwrap_or(".scriptlang/save.json".to_string());
    let scenario = load_source_by_scripts_dir(&args.scripts_dir, &entry_script)?;
    let mut engine = create_engine_from_xml(CreateEngineFromXmlOptions {
        scripts_xml: scenario.scripts_xml.clone(),
        entry_script: Some(entry_script.clone()),
        entry_args: None,
        host_functions: None,
        random_seed: None,
        compiler_version: Some(DEFAULT_COMPILER_VERSION.to_string()),
    })?;

    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return run_tui_line_mode(&state_file, &scenario, &entry_script, &mut engine);
    }

    run_tui_ratatui_mode(&state_file, &scenario, &entry_script, &mut engine)
}

#[cfg(coverage)]
fn run_tui(args: TuiArgs) -> Result<i32, ScriptLangError> {
    let entry_script = args.entry_script.unwrap_or("main".to_string());
    let state_file = args
        .state_file
        .unwrap_or(".scriptlang/save.json".to_string());
    let scenario = load_source_by_scripts_dir(&args.scripts_dir, &entry_script)?;
    let mut engine = create_engine_from_xml(CreateEngineFromXmlOptions {
        scripts_xml: scenario.scripts_xml.clone(),
        entry_script: Some(entry_script.clone()),
        entry_args: None,
        host_functions: None,
        random_seed: None,
        compiler_version: Some(DEFAULT_COMPILER_VERSION.to_string()),
    })?;
    run_tui_line_mode(&state_file, &scenario, &entry_script, &mut engine)
}

fn run_tui_line_mode(
    state_file: &str,
    scenario: &LoadedScenario,
    entry_script: &str,
    engine: &mut sl_runtime::ScriptLangEngine,
) -> Result<i32, ScriptLangError> {
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let mut writer = io::stdout();
    run_tui_line_mode_with_io(
        state_file,
        scenario,
        entry_script,
        engine,
        &mut reader,
        &mut writer,
    )
}

fn run_tui_line_mode_with_io(
    state_file: &str,
    scenario: &LoadedScenario,
    entry_script: &str,
    engine: &mut sl_runtime::ScriptLangEngine,
    reader: &mut dyn BufRead,
    writer: &mut dyn Write,
) -> Result<i32, ScriptLangError> {
    println!("ScriptLang TUI");
    println!("commands: :help :save :load :restart :quit");
    let command_context = TuiCommandContext {
        state_file,
        scenario,
        entry_script,
    };

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
                    let raw = prompt_input_from("> ", reader, writer)?;
                    let mut emit = |line: String| println!("{}", line);
                    let action =
                        handle_line_cmd(raw.as_str(), &command_context, engine, &mut emit)?;
                    match action {
                        TuiCommandAction::Continue => continue,
                        TuiCommandAction::RefreshBoundary => break,
                        TuiCommandAction::Quit => return Ok(0),
                        TuiCommandAction::NotHandled => {}
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
                    let raw = prompt_input_from("> ", reader, writer)?;
                    let mut emit = |line: String| println!("{}", line);
                    let action =
                        handle_line_cmd(raw.as_str(), &command_context, engine, &mut emit)?;
                    match action {
                        TuiCommandAction::Continue => continue,
                        TuiCommandAction::RefreshBoundary => break,
                        TuiCommandAction::Quit => return Ok(0),
                        TuiCommandAction::NotHandled => {}
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

#[cfg(not(coverage))]
fn run_tui_ratatui_mode(
    state_file: &str,
    scenario: &LoadedScenario,
    entry_script: &str,
    engine: &mut sl_runtime::ScriptLangEngine,
) -> Result<i32, ScriptLangError> {
    let mut terminal = TuiTerminal::new()?;
    let mut ui = TuiUiState {
        logs: vec![
            "ScriptLang TUI".to_string(),
            "commands: :help :save :load :restart :quit".to_string(),
            "Use Up/Down to select choice, Enter to submit.".to_string(),
        ],
        input: String::new(),
        status: String::new(),
        boundary: TuiBoundary::End,
    };

    refresh_tui_boundary(engine, &mut ui)?;
    loop {
        terminal
            .terminal_mut()
            .draw(|frame| render_tui(frame, &ui))
            .map_err(|error| ScriptLangError::new("TUI_IO", error.to_string()))?;

        if !event::poll(Duration::from_millis(100))
            .map_err(|error| ScriptLangError::new("TUI_IO", error.to_string()))?
        {
            continue;
        }

        let evt =
            event::read().map_err(|error| ScriptLangError::new("TUI_IO", error.to_string()))?;
        if let Event::Key(key) = evt {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            let should_quit =
                handle_tui_key(key, state_file, scenario, entry_script, engine, &mut ui)?;
            if should_quit {
                break;
            }
        }
    }
    Ok(0)
}

#[cfg(not(coverage))]
fn handle_tui_key(
    key: KeyEvent,
    state_file: &str,
    scenario: &LoadedScenario,
    entry_script: &str,
    engine: &mut sl_runtime::ScriptLangEngine,
    ui: &mut TuiUiState,
) -> Result<bool, ScriptLangError> {
    match key.code {
        KeyCode::Up => {
            if ui.input.is_empty() {
                if let TuiBoundary::Choices {
                    selected, items, ..
                } = &mut ui.boundary
                {
                    if !items.is_empty() {
                        *selected = selected.saturating_sub(1);
                    }
                }
            }
        }
        KeyCode::Down => {
            if ui.input.is_empty() {
                if let TuiBoundary::Choices {
                    selected, items, ..
                } = &mut ui.boundary
                {
                    if !items.is_empty() {
                        let last = items.len().saturating_sub(1);
                        *selected = (*selected + 1).min(last);
                    }
                }
            }
        }
        KeyCode::Backspace => {
            ui.input.pop();
        }
        KeyCode::Esc => {
            ui.input.clear();
            ui.status.clear();
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return Ok(true),
        KeyCode::Char(ch) => {
            if !key.modifiers.contains(KeyModifiers::CONTROL) {
                ui.input.push(ch);
            }
        }
        KeyCode::Enter => {
            let submitted = std::mem::take(&mut ui.input);
            if submitted.starts_with(':') {
                let mut emit = |line: String| tui_push_log(ui, line);
                match handle_tui_command(
                    submitted.as_str(),
                    state_file,
                    scenario,
                    entry_script,
                    engine,
                    &mut emit,
                )? {
                    TuiCommandAction::Continue => {
                        ui.status = "Command executed.".to_string();
                    }
                    TuiCommandAction::RefreshBoundary => {
                        refresh_tui_boundary(engine, ui)?;
                        ui.status = "State refreshed.".to_string();
                    }
                    TuiCommandAction::Quit => return Ok(true),
                    TuiCommandAction::NotHandled => {
                        ui.status = format!("Unknown command: {}", submitted);
                    }
                }
                return Ok(false);
            }

            match &mut ui.boundary {
                TuiBoundary::Choices {
                    items, selected, ..
                } => {
                    let choice = if submitted.trim().is_empty() {
                        items.get(*selected).map(|item| item.0).ok_or_else(|| {
                            ScriptLangError::new("TUI_CHOICE_PARSE", "No choices available")
                        })?
                    } else {
                        submitted.trim().parse::<usize>().map_err(|_| {
                            ScriptLangError::new(
                                "TUI_CHOICE_PARSE",
                                format!("Invalid choice index: {}", submitted),
                            )
                        })?
                    };
                    engine.choose(choice)?;
                    refresh_tui_boundary(engine, ui)?;
                }
                TuiBoundary::Input { .. } => {
                    engine.submit_input(submitted.trim_end_matches(&['\r', '\n'][..]))?;
                    refresh_tui_boundary(engine, ui)?;
                }
                TuiBoundary::End => return Ok(true),
            }
        }
        _ => {}
    }
    Ok(false)
}

#[cfg(not(coverage))]
fn refresh_tui_boundary(
    engine: &mut sl_runtime::ScriptLangEngine,
    ui: &mut TuiUiState,
) -> Result<(), ScriptLangError> {
    let boundary = run_to_boundary(engine)?;
    for text in boundary.texts {
        tui_push_log(ui, text);
    }

    ui.boundary = match boundary.event {
        BoundaryEvent::Choices => TuiBoundary::Choices {
            prompt: boundary.choice_prompt_text,
            items: boundary.choices,
            selected: 0,
        },
        BoundaryEvent::Input => TuiBoundary::Input {
            prompt: boundary.input_prompt_text.unwrap_or_default(),
            default_text: boundary.input_default_text.unwrap_or_default(),
        },
        BoundaryEvent::End => {
            tui_push_log(ui, "[END]".to_string());
            TuiBoundary::End
        }
    };
    Ok(())
}

#[cfg(not(coverage))]
fn tui_push_log(ui: &mut TuiUiState, line: String) {
    ui.logs.push(line);
    const MAX_LOG_LINES: usize = 200;
    if ui.logs.len() > MAX_LOG_LINES {
        let excess = ui.logs.len() - MAX_LOG_LINES;
        ui.logs.drain(0..excess);
    }
}

#[cfg(not(coverage))]
fn render_tui(frame: &mut Frame<'_>, ui: &TuiUiState) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(3),
            Constraint::Length(2),
        ])
        .split(frame.area());

    let header = Paragraph::new("ScriptLang TUI  |  :help :save :load :restart :quit")
        .style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .block(Block::default().borders(Borders::ALL).title("Header"));
    frame.render_widget(header, layout[0]);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(66), Constraint::Percentage(34)])
        .split(layout[1]);

    let logs = Paragraph::new(ui.logs.join("\n"))
        .wrap(Wrap { trim: false })
        .block(Block::default().borders(Borders::ALL).title("Story"));
    frame.render_widget(logs, body[0]);

    match &ui.boundary {
        TuiBoundary::Choices {
            prompt,
            items,
            selected,
        } => {
            let mut lines: Vec<ListItem<'_>> = Vec::new();
            if let Some(text) = prompt {
                lines.push(ListItem::new(Line::from(Span::styled(
                    text,
                    Style::default().fg(Color::Yellow),
                ))));
                lines.push(ListItem::new(""));
            }
            for (idx, (index, text)) in items.iter().enumerate() {
                let style = if idx == *selected {
                    Style::default().fg(Color::Black).bg(Color::Green)
                } else {
                    Style::default()
                };
                lines.push(ListItem::new(Line::from(vec![
                    Span::styled(format!("[{}] ", index), style.add_modifier(Modifier::BOLD)),
                    Span::styled(text, style),
                ])));
            }
            let list =
                List::new(lines).block(Block::default().borders(Borders::ALL).title("Choices"));
            frame.render_widget(list, body[1]);
        }
        TuiBoundary::Input {
            prompt,
            default_text,
        } => {
            let panel = Paragraph::new(vec![
                Line::from(Span::styled(
                    prompt,
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(format!("default: {}", default_text)),
            ])
            .wrap(Wrap { trim: false })
            .block(Block::default().borders(Borders::ALL).title("Input"));
            frame.render_widget(panel, body[1]);
        }
        TuiBoundary::End => {
            let panel = Paragraph::new("Reached [END]. Press Enter or Ctrl+C to exit.")
                .block(Block::default().borders(Borders::ALL).title("Status"));
            frame.render_widget(panel, body[1]);
        }
    }

    let input = Paragraph::new(ui.input.as_str()).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Input / Command"),
    );
    frame.render_widget(input, layout[2]);

    let status = Paragraph::new(ui.status.as_str())
        .style(Style::default().fg(Color::LightBlue))
        .block(Block::default().borders(Borders::ALL).title("Feedback"));
    frame.render_widget(status, layout[3]);
}

fn handle_tui_command(
    raw: &str,
    state_file: &str,
    scenario: &LoadedScenario,
    entry_script: &str,
    engine: &mut sl_runtime::ScriptLangEngine,
    emit: &mut dyn FnMut(String),
) -> Result<TuiCommandAction, ScriptLangError> {
    match raw {
        ":help" => {
            emit("commands: :help :save :load :restart :quit".to_string());
            Ok(TuiCommandAction::Continue)
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
            emit(format!("saved: {}", state_file));
            Ok(TuiCommandAction::Continue)
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
            emit(format!("loaded: {}", state_file));
            Ok(TuiCommandAction::RefreshBoundary)
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
            emit("restarted".to_string());
            Ok(TuiCommandAction::RefreshBoundary)
        }
        ":quit" => {
            emit("bye".to_string());
            Ok(TuiCommandAction::Quit)
        }
        _ => Ok(TuiCommandAction::NotHandled),
    }
}

fn handle_line_cmd(
    raw: &str,
    context: &TuiCommandContext<'_>,
    engine: &mut sl_runtime::ScriptLangEngine,
    emit: &mut dyn FnMut(String),
) -> Result<TuiCommandAction, ScriptLangError> {
    handle_tui_command(
        raw,
        context.state_file,
        context.scenario,
        context.entry_script,
        engine,
        emit,
    )
}

fn prompt_input_from(
    prefix: &str,
    reader: &mut dyn BufRead,
    writer: &mut dyn Write,
) -> Result<String, ScriptLangError> {
    write!(writer, "{}", prefix).map_err(map_tui_io)?;
    writer.flush().map_err(map_tui_io)?;
    let mut input = String::new();
    reader.read_line(&mut input).map_err(map_tui_io)?;
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
            serde_json::to_string(&text).expect("string json")
        );
    }

    if let Some(prompt) = boundary.choice_prompt_text {
        println!(
            "PROMPT_JSON:{}",
            serde_json::to_string(&prompt).expect("string json")
        );
    }

    if let Some(prompt) = boundary.input_prompt_text {
        println!(
            "PROMPT_JSON:{}",
            serde_json::to_string(&prompt).expect("string json")
        );
    }

    for (index, text) in boundary.choices {
        println!(
            "CHOICE:{}|{}",
            index,
            serde_json::to_string(&text).expect("string json")
        );
    }

    if let Some(default_text) = boundary.input_default_text {
        println!(
            "INPUT_DEFAULT_JSON:{}",
            serde_json::to_string(&default_text).expect("string json")
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
        serde_json::to_string(&error.message).expect("string json")
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
            .map_err(map_cli_source_path)?
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
        let path_str = path.to_string_lossy();

        if !(path_str.ends_with(".script.xml")
            || path_str.ends_with(".defs.xml")
            || path_str.ends_with(".json"))
        {
            continue;
        }

        let relative = path
            .strip_prefix(scripts_dir)
            .map_err(map_cli_source_scan)?
            .to_string_lossy()
            .replace('\\', "/");

        let content = fs::read_to_string(path).map_err(map_cli_source_read)?;
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
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    fs::create_dir_all(parent).map_err(map_cli_state_write)?;

    let payload = serde_json::to_string(state).expect("player state should serialize");
    fs::write(path, payload).map_err(map_cli_state_write)
}

fn load_player_state(path: &Path) -> Result<PlayerStateV3, ScriptLangError> {
    if !path.exists() {
        return Err(ScriptLangError::new(
            "CLI_STATE_NOT_FOUND",
            format!("State file does not exist: {}", path.display()),
        ));
    }

    let raw = fs::read_to_string(path).map_err(map_cli_state_read)?;

    let state: PlayerStateV3 = serde_json::from_str(&raw).map_err(map_cli_state_invalid)?;

    if state.schema_version != PLAYER_STATE_SCHEMA {
        return Err(ScriptLangError::new(
            "CLI_STATE_SCHEMA",
            format!("Unsupported player state schema: {}", state.schema_version),
        ));
    }

    Ok(state)
}

fn map_tui_io(error: std::io::Error) -> ScriptLangError {
    ScriptLangError::new("TUI_IO", error.to_string())
}

fn map_cli_source_path(error: std::io::Error) -> ScriptLangError {
    ScriptLangError::new("CLI_SOURCE_PATH", error.to_string())
}

fn map_cli_source_scan(error: std::path::StripPrefixError) -> ScriptLangError {
    ScriptLangError::new("CLI_SOURCE_SCAN", error.to_string())
}

fn map_cli_source_read(error: std::io::Error) -> ScriptLangError {
    ScriptLangError::new("CLI_SOURCE_READ", error.to_string())
}

fn map_cli_state_write(error: std::io::Error) -> ScriptLangError {
    ScriptLangError::new("CLI_STATE_WRITE", error.to_string())
}

fn map_cli_state_read(error: std::io::Error) -> ScriptLangError {
    ScriptLangError::new("CLI_STATE_READ", error.to_string())
}

fn map_cli_state_invalid(error: serde_json::Error) -> ScriptLangError {
    ScriptLangError::new("CLI_STATE_INVALID", error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
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

        let error =
            read_scripts_xml_from_dir(&root).expect_err("empty source set should return error");
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
        let loaded =
            load_source_by_scripts_dir(&scripts_dir, "main").expect("source should be loaded");
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

        let input_start_code = run_start(StartArgs {
            scripts_dir: input_scenario.clone(),
            entry_script: Some("main".to_string()),
            state_out: input_state_1.to_string_lossy().to_string(),
        })
        .expect("input scenario start should pass");
        assert_eq!(input_start_code, 0);

        let input_code = run_input(InputArgs {
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
}
