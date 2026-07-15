use clap::{Args, Parser, Subcommand, ValueEnum};
use clap_complete::Shell;
use std::ffi::OsString;

const ABOUT: &str = "Bring every hosting environment within reach";
const LONG_ABOUT: &str = "Bring every hosting environment within reach.\n\nHostBraid is a provider-neutral hosting environment CLI for WordPress professionals. It discovers sites and environments, delegates terminal access to OpenSSH, and will orchestrate explicit export and pull workflows without replacing WP-CLI.";
const AFTER_HELP: &str = "Start here:\n  hostbraid guide getting-started\n  hostbraid doctor\n  hostbraid search ssh\n\nFor scripts and agents:\n  hostbraid --output json --no-input search environment\n\nHostBraid is an open-source project by It's Ed · https://itsed.se";

#[derive(Debug, Parser)]
#[command(
    name = "hostbraid",
    bin_name = "hostbraid",
    version,
    about = ABOUT,
    long_about = LONG_ABOUT,
    after_help = AFTER_HELP,
    propagate_version = true,
    disable_help_subcommand = false
)]
pub(crate) struct Cli {
    /// Select human-friendly terminal output or the stable JSON envelope.
    #[arg(
        short = 'o',
        long,
        value_enum,
        default_value_t = OutputFormat::Human,
        env = "HOSTBRAID_OUTPUT",
        global = true
    )]
    pub output: OutputFormat,

    /// Never prompt for input. Implied by `--output json`.
    #[arg(long, global = true)]
    pub no_input: bool,

    /// Control terminal colors. `NO_COLOR` is also respected.
    #[arg(
        long,
        value_enum,
        default_value_t = ColorChoice::Auto,
        env = "HOSTBRAID_COLOR",
        global = true
    )]
    pub color: ColorChoice,

    /// Hide transient progress UI.
    #[arg(short, long, global = true)]
    pub quiet: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Commands {
    /// Read practical HostBraid workflow guides.
    #[command(alias = "docs")]
    Guide(GuideArgs),

    /// Search every command and built-in guide.
    #[command(alias = "find")]
    Search(SearchArgs),

    /// Check local tools used for SSH, transfer, and WordPress workflows.
    #[command(
        long_about = "Check local tools used for SSH, transfer, and WordPress workflows.\n\nDoctor is a report: missing or unhealthy tools are returned as booleans and warnings, while the command exits successfully when it can produce that report."
    )]
    Doctor,

    /// Generate shell completion code on stdout.
    Completion(CompletionArgs),
}

impl Commands {
    pub const fn machine_name(&self) -> &'static str {
        match self {
            Self::Guide(arguments) if arguments.list => "guide.list",
            Self::Guide(_) => "guide.show",
            Self::Search(_) => "search",
            Self::Doctor => "doctor",
            Self::Completion(_) => "completion",
        }
    }
}

#[derive(Debug, Args)]
pub(crate) struct GuideArgs {
    /// Guide topic. Omit it to open the getting-started guide.
    #[arg(value_enum, conflicts_with = "list")]
    pub topic: Option<GuideTopic>,

    /// List available guide topics.
    #[arg(long)]
    pub list: bool,
}

#[derive(Debug, Args)]
pub(crate) struct SearchArgs {
    /// Words to find in command names, descriptions, and guides.
    pub query: String,

    /// Maximum number of results.
    #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u16).range(1..=100))]
    pub limit: u16,
}

#[derive(Debug, Args)]
pub(crate) struct CompletionArgs {
    /// Shell whose completion script should be generated.
    #[arg(value_enum)]
    pub shell: Shell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum GuideTopic {
    GettingStarted,
    Humans,
    Agents,
    Concepts,
    Security,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    Human,
    Json,
}

impl OutputFormat {
    #[must_use]
    pub const fn is_machine(self) -> bool {
        matches!(self, Self::Json)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ColorChoice {
    Auto,
    Always,
    Never,
}

pub(crate) fn machine_output_requested(arguments: &[OsString]) -> bool {
    arguments.windows(2).any(|window| {
        matches!(window[0].to_str(), Some("--output" | "-o")) && window[1].to_str() == Some("json")
    }) || arguments.iter().any(|argument| {
        matches!(
            argument.to_str(),
            Some("--output=json" | "-o=json" | "-ojson")
        )
    }) || std::env::var("HOSTBRAID_OUTPUT").is_ok_and(|value| value == "json")
}
