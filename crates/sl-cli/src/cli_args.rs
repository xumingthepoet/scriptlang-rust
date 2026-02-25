use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "scriptlang-player")]
#[command(about = "ScriptLang Rust agent CLI")]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Mode,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Mode {
    Agent(AgentArgs),
    Tui(TuiArgs),
}

#[derive(Debug, Args)]
pub(crate) struct AgentArgs {
    #[command(subcommand)]
    pub(crate) command: AgentCommand,
}

#[derive(Debug, Subcommand)]
pub(crate) enum AgentCommand {
    Start(StartArgs),
    Choose(ChooseArgs),
    Input(InputArgs),
}

#[derive(Debug, Args)]
pub(crate) struct StartArgs {
    #[arg(long = "scripts-dir")]
    pub(crate) scripts_dir: String,
    #[arg(long = "entry-script")]
    pub(crate) entry_script: Option<String>,
    #[arg(long = "state-out")]
    pub(crate) state_out: String,
}

#[derive(Debug, Args)]
pub(crate) struct ChooseArgs {
    #[arg(long = "state-in")]
    pub(crate) state_in: String,
    #[arg(long = "choice")]
    pub(crate) choice: usize,
    #[arg(long = "state-out")]
    pub(crate) state_out: String,
}

#[derive(Debug, Args)]
pub(crate) struct InputArgs {
    #[arg(long = "state-in")]
    pub(crate) state_in: String,
    #[arg(long = "text")]
    pub(crate) text: String,
    #[arg(long = "state-out")]
    pub(crate) state_out: String,
}

#[derive(Debug, Args)]
pub(crate) struct TuiArgs {
    #[arg(long = "scripts-dir")]
    pub(crate) scripts_dir: String,
    #[arg(long = "entry-script")]
    pub(crate) entry_script: Option<String>,
    #[arg(long = "state-file")]
    pub(crate) state_file: Option<String>,
}
