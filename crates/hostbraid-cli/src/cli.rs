use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum};
use clap_complete::Shell;
use hostbraid_core::{AppError, ErrorCode};
use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::time::Duration;

const ABOUT: &str = "Bring every hosting environment within reach";
const LONG_ABOUT: &str = "Bring every hosting environment within reach.\n\nHostBraid is a provider-neutral hosting environment CLI for WordPress professionals. It discovers sites and environments, delegates terminal access to OpenSSH, and will orchestrate explicit export and pull workflows without replacing WP-CLI.";
const AFTER_HELP: &str = "Start here:\n  hb login kinsta <name>\n  hb guide getting-started\n  hb doctor\n  hb search ssh\n\nFor scripts and agents:\n  hb --output json --no-input search environment\n\nHostBraid is an open-source project by It's Ed · https://itsed.se";
const LOGIN_AFTER_HELP: &str = "Login creates a validated profile and selects it for subsequent provider commands. Use `hostbraid profile add` when you want to configure a profile without switching to it.\n\nExamples:\n  hb login kinsta agency\n  printf '%s\\n' \"$KINSTA_TOKEN\" | hb login kinsta agency --token-stdin\n  hb login kinsta ci --credential-env KINSTA_TOKEN";
const LOGOUT_AFTER_HELP: &str = "Logout removes HostBraid's local profile and any credential HostBraid saved in the OS credential store. It does not revoke provider-side API keys or unset credential environment variables.\n\nExample:\n  hb logout kinsta:agency";
const PROFILE_AFTER_HELP: &str = "Common shortcuts:\n  hb login kinsta <name>  Create, validate, and select a profile\n  hb profiles             List profiles\n  hb use provider:name    Switch profiles\n  hb logout provider:name Forget a profile after confirmation";
const PROFILE_ADD_AFTER_HELP: &str = "Examples:\n  hostbraid profile add kinsta agency\n  printf '%s\\n' \"$KINSTA_TOKEN\" | hostbraid profile add kinsta agency --token-stdin\n  hostbraid profile add kinsta ci --credential-env KINSTA_TOKEN";

#[derive(Debug, Parser)]
#[command(
    name = "hostbraid",
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
    /// Log in to a provider and make the new profile active.
    #[command(after_help = LOGIN_AFTER_HELP)]
    Login(LoginArgs),

    /// List the provider profiles available on this machine.
    Profiles,

    /// Switch the active provider profile.
    Use(ProfileRefArgs),

    /// Forget a local provider profile after confirmation.
    #[command(after_help = LOGOUT_AFTER_HELP)]
    Logout(ProfileRemoveArgs),

    /// Configure provider accounts without storing secrets in the profile file.
    #[command(after_help = PROFILE_AFTER_HELP)]
    Profile(ProfileArgs),

    /// Discover sites in a configured provider account.
    Site(SiteArgs),

    /// Discover and inspect hosting environments.
    Environment(EnvironmentArgs),

    /// Open a shell or run a command through the system OpenSSH client.
    Ssh(SshArgs),

    /// Inspect read-only WordPress plugin and theme inventory.
    Inventory(InventoryArgs),

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
            Self::Login(_) => "profile.add",
            Self::Profiles => "profile.list",
            Self::Use(_) => "profile.default",
            Self::Logout(_) => "profile.remove",
            Self::Profile(arguments) => arguments.command.machine_name(),
            Self::Site(arguments) => arguments.command.machine_name(),
            Self::Environment(arguments) => arguments.command.machine_name(),
            Self::Ssh(arguments) => arguments.command.machine_name(),
            Self::Inventory(arguments) => arguments.command.machine_name(),
            Self::Guide(arguments) if arguments.list => "guide.list",
            Self::Guide(_) => "guide.show",
            Self::Search(_) => "search",
            Self::Doctor => "doctor",
            Self::Completion(_) => "completion",
        }
    }
}

#[derive(Debug, Args)]
pub(crate) struct ProfileArgs {
    #[command(subcommand)]
    pub command: ProfileCommand,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ProfileCommand {
    /// Add and validate a provider profile.
    #[command(after_help = PROFILE_ADD_AFTER_HELP)]
    Add(ProfileAddArgs),

    /// List configured profiles without resolving their credentials.
    List,

    /// Show secret-free metadata for one profile.
    Show(ProfileRefArgs),

    /// Choose the authoritative default profile.
    Default(ProfileRefArgs),

    /// Remove a profile and its HostBraid-managed keyring credential.
    Remove(ProfileRemoveArgs),

    /// Change how a profile resolves its API credential.
    Credential(ProfileCredentialArgs),
}

impl ProfileCommand {
    pub const fn machine_name(&self) -> &'static str {
        match self {
            Self::Add(_) => "profile.add",
            Self::List => "profile.list",
            Self::Show(_) => "profile.show",
            Self::Default(_) => "profile.default",
            Self::Remove(_) => "profile.remove",
            Self::Credential(arguments) => arguments.command.machine_name(),
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum ProviderChoice {
    /// Kinsta Managed WordPress Hosting.
    Kinsta,
}

impl ProviderChoice {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Kinsta => "kinsta",
        }
    }
}

#[derive(Debug, Args)]
pub(crate) struct LoginArgs {
    /// Compiled-in provider adapter.
    #[arg(value_enum)]
    pub provider: ProviderChoice,

    /// Local profile name used in provider:name selectors.
    pub name: String,

    /// Read one API token from stdin and store it in the OS credential store.
    #[arg(long, conflicts_with = "credential_env")]
    pub token_stdin: bool,

    /// Resolve the token from this named environment variable on every use.
    #[arg(long, value_name = "NAME", conflicts_with = "token_stdin")]
    pub credential_env: Option<String>,
}

impl From<LoginArgs> for ProfileAddArgs {
    fn from(arguments: LoginArgs) -> Self {
        Self {
            provider: arguments.provider,
            name: arguments.name,
            token_stdin: arguments.token_stdin,
            credential_env: arguments.credential_env,
            default: true,
        }
    }
}

#[derive(Debug, Args)]
pub(crate) struct ProfileAddArgs {
    /// Compiled-in provider adapter.
    #[arg(value_enum)]
    pub provider: ProviderChoice,

    /// Local profile name used in provider:name selectors.
    pub name: String,

    /// Read one API token from stdin and store it in the OS credential store.
    #[arg(long, conflicts_with = "credential_env")]
    pub token_stdin: bool,

    /// Resolve the token from this named environment variable on every use.
    #[arg(long, value_name = "NAME", conflicts_with = "token_stdin")]
    pub credential_env: Option<String>,

    /// Make the new profile the explicit default.
    #[arg(long)]
    pub default: bool,
}

#[derive(Debug, Args)]
pub(crate) struct ProfileRefArgs {
    /// Exact profile selector in provider:name form.
    pub profile: String,
}

#[derive(Debug, Args)]
pub(crate) struct ProfileRemoveArgs {
    /// Exact profile selector in provider:name form.
    pub profile: String,

    /// Confirm removal without prompting.
    #[arg(long)]
    pub yes: bool,
}

#[derive(Debug, Args)]
pub(crate) struct ProfileCredentialArgs {
    #[command(subcommand)]
    pub command: ProfileCredentialCommand,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ProfileCredentialCommand {
    /// Replace the credential source after validating it against the provider.
    Set(ProfileCredentialSetArgs),
}

impl ProfileCredentialCommand {
    pub const fn machine_name(&self) -> &'static str {
        match self {
            Self::Set(_) => "profile.credential.set",
        }
    }
}

#[derive(Debug, Args)]
pub(crate) struct ProfileCredentialSetArgs {
    /// Exact profile selector in provider:name form.
    pub profile: String,

    /// Read one API token from stdin and store it in the OS credential store.
    #[arg(long, conflicts_with = "credential_env")]
    pub token_stdin: bool,

    /// Resolve the token from this named environment variable on every use.
    #[arg(long, value_name = "NAME", conflicts_with = "token_stdin")]
    pub credential_env: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct SiteArgs {
    #[command(subcommand)]
    pub command: SiteCommand,
}

#[derive(Debug, Subcommand)]
pub(crate) enum SiteCommand {
    /// List every site visible to a provider profile.
    List(ProfileSelectionArgs),
}

impl SiteCommand {
    pub const fn machine_name(&self) -> &'static str {
        match self {
            Self::List(_) => "site.list",
        }
    }
}

#[derive(Debug, Args)]
pub(crate) struct EnvironmentArgs {
    #[command(subcommand)]
    pub command: EnvironmentCommand,
}

#[derive(Debug, Subcommand)]
pub(crate) enum EnvironmentCommand {
    /// List environments belonging to one exact site ID.
    List(EnvironmentListArgs),

    /// Show one exact environment and its current capabilities.
    Show(EnvironmentShowArgs),
}

impl EnvironmentCommand {
    pub const fn machine_name(&self) -> &'static str {
        match self {
            Self::List(_) => "environment.list",
            Self::Show(_) => "environment.show",
        }
    }
}

#[derive(Debug, Args)]
pub(crate) struct ProfileSelectionArgs {
    /// Provider profile in provider:name form; omit only when a default is configured.
    #[arg(long, value_name = "PROVIDER:NAME")]
    pub profile: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct EnvironmentListArgs {
    #[command(flatten)]
    pub selection: ProfileSelectionArgs,

    /// Exact opaque site ID returned by `site list`.
    #[arg(long)]
    pub site_id: String,
}

#[derive(Debug, Args)]
pub(crate) struct EnvironmentShowArgs {
    #[command(flatten)]
    pub selection: ProfileSelectionArgs,

    /// Exact opaque environment ID returned by `environment list`.
    #[arg(long)]
    pub environment_id: String,
}

#[derive(Debug, Args)]
pub(crate) struct SshArgs {
    #[command(subcommand)]
    pub command: SshCommand,
}

#[derive(Debug, Subcommand)]
pub(crate) enum SshCommand {
    /// Open one interactive shell with normal OpenSSH trust and authentication prompts.
    Open(SshOpenArgs),

    /// Run one remote command on one or more selected environments.
    #[command(
        trailing_var_arg = true,
        after_help = "Examples:\n  hostbraid ssh run --environment-id ENV_ID -- uptime\n  hostbraid ssh run --kind production --label customer-a --yes -- wp core version"
    )]
    Run(SshRunArgs),
}

impl SshCommand {
    pub const fn machine_name(&self) -> &'static str {
        match self {
            Self::Open(_) => "ssh.open",
            Self::Run(_) => "ssh.run",
        }
    }
}

#[derive(Debug, Args)]
pub(crate) struct SshOpenArgs {
    #[command(flatten)]
    pub selection: ProfileSelectionArgs,

    /// Exact opaque environment ID to open.
    #[arg(long)]
    pub environment_id: String,

    /// Disable HostBraid's short-lived OpenSSH connection reuse.
    #[arg(long)]
    pub no_pool: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum EnvironmentKindArg {
    Production,
    Staging,
    Development,
    Other,
}

#[derive(Debug, Args)]
pub(crate) struct SshRunArgs {
    #[command(flatten)]
    pub selection: ProfileSelectionArgs,

    /// Exact environment ID; repeat to target several explicit environments.
    #[arg(
        long = "environment-id",
        value_name = "ENVIRONMENT_ID",
        action = clap::ArgAction::Append
    )]
    pub environment_ids: Vec<String>,

    /// Exact site ID; repeat to select every matching site's environments.
    #[arg(
        long = "site-id",
        value_name = "SITE_ID",
        action = clap::ArgAction::Append
    )]
    pub site_ids: Vec<String>,

    /// Normalized environment kind; repeat to allow several kinds.
    #[arg(long, value_enum, value_name = "KIND", action = clap::ArgAction::Append)]
    pub kind: Vec<EnvironmentKindArg>,

    /// Exact, case-sensitive site label; repeat to allow several labels.
    #[arg(long, value_name = "LABEL", action = clap::ArgAction::Append)]
    pub label: Vec<String>,

    /// Deliberately select every environment in the profile.
    #[arg(
        long,
        conflicts_with_all = ["environment_ids", "site_ids", "kind", "label"]
    )]
    pub all: bool,

    /// Maximum concurrent SSH preparations and OpenSSH children.
    #[arg(long, default_value_t = 8, value_parser = clap::value_parser!(u16).range(1..=64))]
    pub jobs: u16,

    /// Maximum connection-and-command duration per target, such as 30s or 5m.
    #[arg(long, value_parser = parse_duration)]
    pub timeout: Option<Duration>,

    /// Stop scheduling queued targets after the first unsuccessful result.
    #[arg(long)]
    pub fail_fast: bool,

    /// Confirm a broad selector without prompting.
    #[arg(long)]
    pub yes: bool,

    /// Disable HostBraid's short-lived OpenSSH connection reuse.
    #[arg(long)]
    pub no_pool: bool,

    /// Command and arguments interpreted by the remote SSH shell.
    #[arg(required = true, num_args = 1.., allow_hyphen_values = true)]
    pub remote_command: Vec<OsString>,
}

#[derive(Debug, Args)]
pub(crate) struct InventoryArgs {
    #[command(subcommand)]
    pub command: InventoryCommand,
}

#[derive(Debug, Subcommand)]
pub(crate) enum InventoryCommand {
    /// List company-wide WordPress plugin inventory.
    Plugins(InventoryListArgs),

    /// List company-wide WordPress theme inventory.
    Themes(InventoryListArgs),
}

impl InventoryCommand {
    pub const fn machine_name(&self) -> &'static str {
        match self {
            Self::Plugins(_) => "inventory.plugins",
            Self::Themes(_) => "inventory.themes",
        }
    }
}

#[derive(Debug, Args)]
pub(crate) struct InventoryListArgs {
    #[command(flatten)]
    pub selection: ProfileSelectionArgs,

    /// Case-insensitive text matched against component slug and title.
    #[arg(long)]
    pub search: Option<String>,

    /// Include only components with an update available.
    #[arg(long)]
    pub updates: bool,

    /// Include only components with a vulnerable installed version.
    #[arg(long)]
    pub vulnerable: bool,

    /// Include every matching environment installation in human output.
    #[arg(long)]
    pub details: bool,
}

fn parse_duration(value: &str) -> std::result::Result<Duration, String> {
    let duration = humantime::parse_duration(value)
        .map_err(|_| "duration must be a positive value such as `30s`, `5m`, or `1h`".to_owned())?;
    if duration.is_zero() {
        return Err("duration must be greater than zero".to_owned());
    }
    Ok(duration)
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

pub(crate) fn try_parse_from(arguments: &[OsString]) -> Result<Cli, clap::Error> {
    let binary_name = invoked_binary_name(arguments);
    let matches = Cli::command()
        .bin_name(binary_name)
        .try_get_matches_from(arguments)?;
    Cli::from_arg_matches(&matches)
}

pub(crate) fn parse_app_error(arguments: &[OsString], error: &clap::Error) -> AppError {
    use clap::error::ErrorKind;

    let binary = invoked_binary_name(arguments);
    let command = safe_command_path(arguments);
    let is_login = command.ends_with(" login");
    let accepts_credentials = is_login
        || command.ends_with(" profile add")
        || command.ends_with(" profile credential set");

    let (message, hint) = match error.kind() {
        ErrorKind::MissingRequiredArgument
        | ErrorKind::TooFewValues
        | ErrorKind::WrongNumberOfValues
            if is_login
                && error_targets_any(error, &["<NAME>"])
                && !error_targets_any(error, &["<PROVIDER>"]) => (
                    "login requires a local profile name".to_owned(),
                    format!(
                        "Run `{command} kinsta <name>` (for example, `{command} kinsta agency`). The name is local metadata and never contains the API token."
                    ),
                ),
        ErrorKind::MissingRequiredArgument
        | ErrorKind::TooFewValues
        | ErrorKind::WrongNumberOfValues
            if is_login && error_targets_any(error, &["<PROVIDER>", "<NAME>"]) => (
                "login requires a provider and local profile name".to_owned(),
                format!(
                    "Run `{command} kinsta <name>` to enter a token securely (for example, `{command} kinsta agency`), or `{command} --help` for non-interactive credential options."
                ),
            ),
        ErrorKind::MissingRequiredArgument
        | ErrorKind::TooFewValues
        | ErrorKind::WrongNumberOfValues
            if command.ends_with(" use") && error_targets_any(error, &["<PROFILE>"]) => (
                "use requires an exact provider:name profile reference".to_owned(),
                format!(
                    "Run `{} profiles` to list exact references, then `{command} provider:name`.",
                    binary
                ),
            ),
        ErrorKind::MissingRequiredArgument
        | ErrorKind::TooFewValues
        | ErrorKind::WrongNumberOfValues
            if command.ends_with(" logout") && error_targets_any(error, &["<PROFILE>"]) => (
                "logout requires an exact provider:name profile reference".to_owned(),
                format!(
                    "Run `{} profiles` to review exact references, then `{command} provider:name`; HostBraid will ask for confirmation.",
                    binary
                ),
            ),
        ErrorKind::MissingRequiredArgument
        | ErrorKind::TooFewValues
        | ErrorKind::WrongNumberOfValues
            if command.ends_with(" ssh run")
                && error_targets_any(error, &["<REMOTE_COMMAND>..."]) => (
                "ssh run requires a remote command after `--`".to_owned(),
                format!(
                    "For example: `{command} --environment-id <ID> -- uptime`. Run `{command} --help` for target selectors."
                ),
            ),
        ErrorKind::MissingRequiredArgument
        | ErrorKind::TooFewValues
        | ErrorKind::WrongNumberOfValues
            if command.ends_with(" environment list")
                && error_targets_any(error, &["--site-id <SITE_ID>"]) => (
                "environment list requires an exact site ID".to_owned(),
                format!(
                    "Run `{binary} site list`, copy an exact site ID, then pass `--site-id <ID>`."
                ),
            ),
        ErrorKind::MissingRequiredArgument
        | ErrorKind::TooFewValues
        | ErrorKind::WrongNumberOfValues
            if (command.ends_with(" environment show") || command.ends_with(" ssh open"))
                && error_targets_any(error, &["--environment-id <ENVIRONMENT_ID>"]) => (
                format!("`{command}` requires an exact environment ID"),
                format!(
                    "Run `{binary} environment list --site-id <SITE_ID>`, copy an exact environment ID, then pass `--environment-id <ID>`."
                ),
            ),
        ErrorKind::MissingRequiredArgument
        | ErrorKind::TooFewValues
        | ErrorKind::WrongNumberOfValues
            if command.ends_with(" search") && error_targets_any(error, &["<QUERY>"]) => (
                "search requires a query".to_owned(),
                format!("For example: `{command} ssh`, or run `{binary} guide --list`."),
            ),
        ErrorKind::MissingRequiredArgument
        | ErrorKind::TooFewValues
        | ErrorKind::WrongNumberOfValues
            if command.ends_with(" completion") && error_targets_any(error, &["<SHELL>"]) => (
                "completion requires a shell name".to_owned(),
                format!(
                    "Run `{command} --help` to list supported shells, then redirect the generated script into your shell's completion setup."
                ),
            ),
        ErrorKind::MissingRequiredArgument
        | ErrorKind::TooFewValues
        | ErrorKind::WrongNumberOfValues => (
            format!("`{command}` is missing required input"),
            format!(
                "Run `{command} --help` to see the required arguments, accepted formats, and examples."
            ),
        ),
        ErrorKind::MissingSubcommand | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
            if command.ends_with(" profile") => (
                "profile requires a subcommand".to_owned(),
                format!(
                    "Use `{} login`, `{} profiles`, `{} use`, or `{} logout` for common account tasks; run `{command} --help` for advanced profile management.",
                    binary, binary, binary, binary,
                ),
            ),
        ErrorKind::MissingSubcommand | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => (
            format!("`{command}` requires a subcommand"),
            format!("Run `{command} --help` to list the available subcommands and examples."),
        ),
        ErrorKind::UnknownArgument | ErrorKind::TooManyValues if accepts_credentials => (
            format!("`{command}` received an unexpected argument or option"),
            format!(
                "API tokens are never accepted as ordinary arguments. Run `{command} --help`; use the hidden prompt, `--token-stdin`, or `--credential-env <NAME>`."
            ),
        ),
        ErrorKind::UnknownArgument | ErrorKind::TooManyValues => (
            format!("`{command}` received an unrecognized argument or option"),
            format!("Run `{command} --help` to review the accepted arguments and examples."),
        ),
        ErrorKind::InvalidSubcommand if command == binary => (
            "HostBraid did not recognize that command".to_owned(),
            format!(
                "Run `{} --help` to list commands, or `{} search <term>` to find one.",
                binary, binary
            ),
        ),
        ErrorKind::InvalidSubcommand => (
            format!("`{command}` does not recognize that subcommand"),
            format!(
                "Run `{command} --help` to list subcommands, or `{} search <term>` to find a command.",
                binary
            ),
        ),
        ErrorKind::InvalidValue | ErrorKind::ValueValidation
            if is_login && error_targets_any(error, &["<PROVIDER>"]) => (
                "login received a provider or option value that is not accepted".to_owned(),
                format!(
                    "This build supports `kinsta`. Run `{command} kinsta <name>`, or `{command} --help` to review accepted values."
                ),
            ),
        ErrorKind::InvalidValue | ErrorKind::ValueValidation
            if command.ends_with(" ssh run")
                && error_targets_any(error, &["--jobs <JOBS>"]) => (
                    "--jobs must be an integer between 1 and 64".to_owned(),
                    format!("Choose a value from 1 through 64, then retry `{command}`."),
                ),
        ErrorKind::InvalidValue | ErrorKind::ValueValidation
            if command.ends_with(" ssh run")
                && error_targets_any(error, &["--timeout <TIMEOUT>"]) => (
                    "--timeout must be a positive duration".to_owned(),
                    "Use a value such as `30s`, `5m`, or `1h`; zero is not accepted.".to_owned(),
                ),
        ErrorKind::InvalidValue | ErrorKind::ValueValidation => (
            format!("`{command}` received a value that is not accepted"),
            format!("Run `{command} --help` to review accepted values and formats."),
        ),
        ErrorKind::ArgumentConflict if accepts_credentials => (
            format!("`{command}` received conflicting credential options"),
            format!(
                "Choose one credential source: the hidden prompt, `--token-stdin`, or `--credential-env <NAME>`. Run `{command} --help` for examples."
            ),
        ),
        ErrorKind::ArgumentConflict => (
            format!("`{command}` received options that cannot be used together"),
            format!("Run `{command} --help` to review which options conflict."),
        ),
        ErrorKind::NoEquals => (
            format!("`{command}` requires `=` for one option value"),
            format!("Run `{command} --help` to see the required option syntax."),
        ),
        ErrorKind::InvalidUtf8 => (
            "command-line arguments contain text that is not valid UTF-8".to_owned(),
            format!(
                "Retype the command using valid text, or run `{command} --help` to review accepted arguments."
            ),
        ),
        ErrorKind::Io | ErrorKind::Format => (
            "HostBraid could not read the command-line arguments".to_owned(),
            "Retry in a normal terminal. If the problem persists, update HostBraid and report the error code."
                .to_owned(),
        ),
        _ => (
            format!("HostBraid could not parse `{command}`"),
            format!("Run `{command} --help` and retry using the documented syntax."),
        ),
    };

    AppError::new(ErrorCode::InvalidArguments, message).with_hint(hint)
}

fn invoked_binary_name(arguments: &[OsString]) -> &'static str {
    arguments
        .first()
        .and_then(|argument| Path::new(argument).file_stem())
        .filter(|name| *name == OsStr::new("hb"))
        .map_or("hostbraid", |_| "hb")
}

fn safe_command_path(arguments: &[OsString]) -> String {
    let mut root = Cli::command();
    root.build();
    let mut command = &root;
    let mut path = vec![invoked_binary_name(arguments).to_owned()];
    let mut index = 1;

    while let Some((candidate, next_index)) = next_subcommand_candidate(arguments, index) {
        let Some(subcommand) = command.get_subcommands().find(|subcommand| {
            subcommand.get_name() == candidate
                || subcommand.get_all_aliases().any(|alias| alias == candidate)
        }) else {
            break;
        };
        if subcommand.get_name() == "help" {
            break;
        }
        path.push(subcommand.get_name().to_owned());
        command = subcommand;
        index = next_index;
    }

    path.join(" ")
}

fn next_subcommand_candidate(arguments: &[OsString], mut index: usize) -> Option<(&str, usize)> {
    while index < arguments.len() {
        let argument = arguments[index].to_str()?;
        if argument == "--" {
            return None;
        }
        if matches!(argument, "--output" | "--color") {
            index = index.saturating_add(2);
            continue;
        }
        if argument.starts_with("--output=") || argument.starts_with("--color=") {
            index += 1;
            continue;
        }
        if matches!(argument, "--no-input" | "--quiet" | "--help" | "--version") {
            index += 1;
            continue;
        }
        if argument.starts_with('-') && !argument.starts_with("--") {
            let scan = scan_short_options(
                argument,
                arguments.get(index + 1).and_then(|value| value.to_str()),
            );
            if !scan.known {
                return None;
            }
            index += usize::from(scan.consumes_next) + 1;
            continue;
        }
        if argument.starts_with('-') {
            return None;
        }
        return Some((argument, index + 1));
    }
    None
}

fn error_targets_any(error: &clap::Error, allowlist: &[&str]) -> bool {
    use clap::error::{ContextKind, ContextValue};

    match error.get(ContextKind::InvalidArg) {
        Some(ContextValue::String(argument)) => allowlist.contains(&argument.as_str()),
        Some(ContextValue::Strings(arguments)) => arguments
            .iter()
            .any(|argument| allowlist.contains(&argument.as_str())),
        _ => false,
    }
}

pub(crate) fn machine_output_requested(arguments: &[OsString]) -> bool {
    if let Ok(cli) = try_parse_from(arguments) {
        return cli.output.is_machine();
    }

    let mut explicit = None;
    let mut index = 1;
    let scan_limit = ssh_run_remote_boundary(arguments).unwrap_or(arguments.len());
    while index < scan_limit {
        let Some(argument) = arguments[index].to_str() else {
            index += 1;
            continue;
        };
        if argument == "--" {
            break;
        }
        if matches!(argument, "--output" | "-o") {
            if let Some(value) = arguments.get(index + 1).and_then(|value| value.to_str()) {
                explicit = requested_output(value).or(explicit);
            }
            index += 2;
            continue;
        }
        if argument.starts_with('-') && !argument.starts_with("--") {
            let scan = scan_short_options(
                argument,
                arguments.get(index + 1).and_then(|value| value.to_str()),
            );
            if let Some(output) = scan.output {
                explicit = Some(output);
            }
            index += usize::from(scan.consumes_next) + 1;
            continue;
        }
        if let Some(value) = argument.strip_prefix("--output=") {
            explicit = requested_output(value).or(explicit);
        }
        index += 1;
    }
    explicit.unwrap_or_else(|| std::env::var("HOSTBRAID_OUTPUT").is_ok_and(|value| value == "json"))
}

#[derive(Debug, Clone, Copy)]
struct ShortOptionScan {
    output: Option<bool>,
    consumes_next: bool,
    known: bool,
}

fn scan_short_options(argument: &str, next: Option<&str>) -> ShortOptionScan {
    let Some(cluster) = argument.strip_prefix('-').filter(|value| !value.is_empty()) else {
        return ShortOptionScan {
            output: None,
            consumes_next: false,
            known: false,
        };
    };
    for (offset, option) in cluster.char_indices() {
        match option {
            'q' | 'h' | 'V' => {}
            'o' => {
                let value = &cluster[offset + option.len_utf8()..];
                let consumes_next = value.is_empty();
                let value = if consumes_next {
                    next
                } else {
                    Some(value.strip_prefix('=').unwrap_or(value))
                };
                return ShortOptionScan {
                    output: value.and_then(requested_output),
                    consumes_next,
                    known: true,
                };
            }
            _ => {
                return ShortOptionScan {
                    output: None,
                    consumes_next: false,
                    known: false,
                };
            }
        }
    }
    ShortOptionScan {
        output: None,
        consumes_next: false,
        known: true,
    }
}

fn ssh_run_remote_boundary(arguments: &[OsString]) -> Option<usize> {
    let mut index = 1;
    while index < arguments.len() {
        let argument = arguments[index].to_str()?;
        if argument == "ssh" {
            return ssh_run_remote_boundary_from(arguments, index);
        }
        if argument == "--" {
            return None;
        }
        if matches!(argument, "--output" | "--color") {
            index += 2;
            continue;
        }
        if argument.starts_with("--output=") || argument.starts_with("--color=") {
            index += 1;
            continue;
        }
        if matches!(argument, "--no-input" | "--quiet" | "--help" | "--version") {
            index += 1;
            continue;
        }
        if argument.starts_with('-') && !argument.starts_with("--") {
            let scan = scan_short_options(
                argument,
                arguments.get(index + 1).and_then(|value| value.to_str()),
            );
            index += usize::from(scan.known && scan.consumes_next) + 1;
            continue;
        }
        if argument.starts_with('-') {
            // The overall parse is already invalid, but an unknown option does not consume an
            // ordinary positional token. Continue far enough to recognize a real root `ssh run`
            // path while never searching through another command's positional arguments.
            index += 1;
            continue;
        }
        return None;
    }
    None
}

fn ssh_run_remote_boundary_from(arguments: &[OsString], start: usize) -> Option<usize> {
    let mut stage = SshRunScanStage::AwaitRun;
    let mut index = start + 1;
    while index < arguments.len() {
        let Some(argument) = arguments[index].to_str() else {
            return (stage == SshRunScanStage::Run).then_some(index);
        };
        if argument == "--" {
            return (stage == SshRunScanStage::Run).then_some(index);
        }
        if matches!(argument, "--output" | "--color") {
            index += 2;
            continue;
        }
        if argument.starts_with("--output=") || argument.starts_with("--color=") {
            index += 1;
            continue;
        }
        if matches!(argument, "--no-input" | "--quiet" | "--help" | "--version") {
            index += 1;
            continue;
        }
        if argument.starts_with('-') && !argument.starts_with("--") {
            let scan = scan_short_options(
                argument,
                arguments.get(index + 1).and_then(|value| value.to_str()),
            );
            if scan.known {
                index += usize::from(scan.consumes_next) + 1;
                continue;
            }
            if stage == SshRunScanStage::Run {
                return Some(index);
            }
            index += 1;
            continue;
        }

        if stage == SshRunScanStage::Run {
            if matches!(
                argument,
                "--profile"
                    | "--environment-id"
                    | "--site-id"
                    | "--kind"
                    | "--label"
                    | "--jobs"
                    | "--timeout"
            ) {
                index += 2;
                continue;
            }
            if argument.starts_with("--profile=")
                || argument.starts_with("--environment-id=")
                || argument.starts_with("--site-id=")
                || argument.starts_with("--kind=")
                || argument.starts_with("--label=")
                || argument.starts_with("--jobs=")
                || argument.starts_with("--timeout=")
                || matches!(argument, "--all" | "--fail-fast" | "--yes" | "--no-pool")
            {
                index += 1;
                continue;
            }
            return Some(index);
        }

        if argument.starts_with('-') {
            index += 1;
            continue;
        }
        if argument != "run" {
            return None;
        }
        stage = SshRunScanStage::Run;
        index += 1;
    }
    None
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SshRunScanStage {
    AwaitRun,
    Run,
}

fn requested_output(value: &str) -> Option<bool> {
    match value {
        "json" => Some(true),
        "human" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{LoginArgs, ProfileAddArgs, ProviderChoice};

    #[test]
    fn login_explicitly_selects_the_new_profile() {
        let arguments = LoginArgs {
            provider: ProviderChoice::Kinsta,
            name: "agency".to_owned(),
            token_stdin: false,
            credential_env: Some("KINSTA_TOKEN".to_owned()),
        };

        let profile: ProfileAddArgs = arguments.into();

        assert!(profile.default);
        assert_eq!(profile.name, "agency");
        assert_eq!(profile.credential_env.as_deref(), Some("KINSTA_TOKEN"));
    }
}
