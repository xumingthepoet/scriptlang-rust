use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "sl-cli")]
#[command(about = "ScriptLang CLI for scripted runs and interactive debugging")]
#[command(
    long_about = "ScriptLang CLI for scripted runs and interactive debugging.\n\nUse `agent` for stateful automation workflows (`start/choose/input/replay`), `compile` for artifact build output, and `tui` for manual interactive playtesting."
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Mode,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Mode {
    Agent(AgentArgs),
    #[command(about = "Compile scripts and output artifact JSON")]
    #[command(
        long_about = "Compile scripts and output artifact JSON.\n\nUse --dry-run to compile only in memory without writing output (useful for debugging compilation errors)."
    )]
    Compile(CompileArgs),
    Tui(TuiArgs),
}

#[derive(Debug, Args)]
pub(crate) struct AgentArgs {
    #[command(subcommand)]
    pub(crate) command: AgentCommand,
}

#[derive(Debug, Subcommand)]
pub(crate) enum AgentCommand {
    #[command(about = "Start a new session from scripts and stop at first boundary")]
    Start(StartArgs),
    #[command(about = "Resume from state and submit a choice index")]
    Choose(ChooseArgs),
    #[command(about = "Resume from state and submit input text")]
    Input(InputArgs),
    #[command(about = "Run from a fresh start with queued --step actions")]
    #[command(
        long_about = "Run from a fresh start with queued --step actions.\n\nEach `--step` is consumed when a matching boundary appears:\n- choose:<index>\n- input:<text>\n\nWhen steps are exhausted, replay continues until the next boundary (CHOICES/INPUT/END), then exits successfully with a summary."
    )]
    Replay(ReplayArgs),
}

#[derive(Debug, Args)]
pub(crate) struct StartArgs {
    #[arg(long = "scripts-dir")]
    #[arg(help = "Directory containing *.xml")]
    pub(crate) scripts_dir: String,
    #[arg(long = "entry-script")]
    #[arg(help = "Entry script name (default: main.main)")]
    pub(crate) entry_script: Option<String>,
    #[arg(long = "state-out")]
    #[arg(help = "Path to write player state json")]
    pub(crate) state_out: String,
    #[arg(long = "rand")]
    #[arg(help = "Comma-separated random sequence, e.g. 12,3,1")]
    pub(crate) rand: Option<String>,
    #[arg(long = "show-debug")]
    #[arg(help = "Show <debug> output events")]
    pub(crate) show_debug: bool,
}

#[derive(Debug, Args)]
pub(crate) struct ChooseArgs {
    #[arg(long = "state-in")]
    #[arg(help = "Path to input player state json")]
    pub(crate) state_in: String,
    #[arg(long = "choice")]
    #[arg(help = "Visible choice index to submit")]
    pub(crate) choice: usize,
    #[arg(long = "state-out")]
    #[arg(help = "Path to output player state json")]
    pub(crate) state_out: String,
    #[arg(long = "rand")]
    #[arg(help = "Comma-separated random sequence override, e.g. 12,3,1")]
    pub(crate) rand: Option<String>,
    #[arg(long = "show-debug")]
    #[arg(help = "Show <debug> output events")]
    pub(crate) show_debug: bool,
}

#[derive(Debug, Args)]
pub(crate) struct InputArgs {
    #[arg(long = "state-in")]
    #[arg(help = "Path to input player state json")]
    pub(crate) state_in: String,
    #[arg(long = "text")]
    #[arg(help = "Input text to submit")]
    pub(crate) text: String,
    #[arg(long = "state-out")]
    #[arg(help = "Path to output player state json")]
    pub(crate) state_out: String,
    #[arg(long = "rand")]
    #[arg(help = "Comma-separated random sequence override, e.g. 12,3,1")]
    pub(crate) rand: Option<String>,
    #[arg(long = "show-debug")]
    #[arg(help = "Show <debug> output events")]
    pub(crate) show_debug: bool,
}

#[derive(Debug, Args)]
#[command(
    after_help = "Examples:\n  sl-cli agent replay --scripts-dir crates/sl-test-example/examples/16-input-name --step input:Rin\n  sl-cli agent replay --scripts-dir crates/sl-test-example/examples/07-battle-duel --step choose:0 --step choose:1 --step input:Rin"
)]
pub(crate) struct ReplayArgs {
    #[arg(long = "scripts-dir")]
    #[arg(help = "Directory containing *.xml")]
    pub(crate) scripts_dir: String,
    #[arg(long = "entry-script")]
    #[arg(help = "Entry script name (default: main.main)")]
    pub(crate) entry_script: Option<String>,
    #[arg(long = "step")]
    #[arg(help = "Replay action: choose:<index> or input:<text>. Repeat to build a queue")]
    pub(crate) step: Vec<String>,
    #[arg(long = "rand")]
    #[arg(help = "Comma-separated random sequence, e.g. 12,3,1")]
    pub(crate) rand: Option<String>,
    #[arg(long = "show-debug")]
    #[arg(help = "Show <debug> output events")]
    pub(crate) show_debug: bool,
}

#[derive(Debug, Args)]
#[command(about = "Interactive TUI mode (auto-fallback to line mode in non-TTY/test env)")]
pub(crate) struct TuiArgs {
    #[arg(long = "scripts-dir")]
    #[arg(help = "Directory containing *.xml")]
    pub(crate) scripts_dir: String,
    #[arg(long = "entry-script")]
    #[arg(help = "Entry script name (default: main.main)")]
    pub(crate) entry_script: Option<String>,
    #[arg(long = "state-file")]
    #[arg(help = "Path to save/load state (default: .scriptlang/save.json)")]
    pub(crate) state_file: Option<String>,
    #[arg(long = "rand")]
    #[arg(help = "Comma-separated random sequence, e.g. 12,3,1")]
    pub(crate) rand: Option<String>,
    #[arg(long = "show-debug")]
    #[arg(help = "Show <debug> output events")]
    pub(crate) show_debug: bool,
}

#[derive(Debug, Args)]
pub(crate) struct CompileArgs {
    #[arg(long = "scripts-dir")]
    #[arg(help = "Directory containing *.xml")]
    pub(crate) scripts_dir: String,
    #[arg(long = "entry-script")]
    #[arg(help = "Entry script name (default: main.main)")]
    pub(crate) entry_script: Option<String>,
    #[arg(long = "output", short = 'o')]
    #[arg(help = "Output path for artifact JSON (required if not --dry-run)")]
    pub(crate) output: Option<String>,
    #[arg(long = "dry-run")]
    #[arg(help = "Compile only in memory, do not write output")]
    pub(crate) dry_run: bool,
    #[arg(long = "rand")]
    #[arg(help = "Comma-separated random sequence, e.g. 12,3,1")]
    pub(crate) rand: Option<String>,
}
