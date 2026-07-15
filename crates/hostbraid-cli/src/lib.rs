mod cli;
mod commands;
mod context;
mod output;
mod profiles;
mod ssh;
mod text;

use crate::cli::{Cli, Commands};
use crate::context::Context;
use clap::{CommandFactory, error::ErrorKind};
use hostbraid_core::{AppError, ErrorCode};
use std::ffi::OsString;
use std::process::ExitCode;

pub use cli::{ColorChoice, OutputFormat};

const VERSION: &str = env!("CARGO_PKG_VERSION");

pub(crate) enum CommandOutcome {
    Success,
    Exit(u8),
}

/// Parse process arguments, run one command, and return a stable exit status.
#[must_use]
pub fn main_entry() -> ExitCode {
    let arguments: Vec<OsString> = std::env::args_os().collect();
    let machine_requested = cli::machine_output_requested(&arguments);

    let cli = match cli::try_parse_from(&arguments) {
        Ok(cli) => cli,
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            let code = error.exit_code();
            if machine_requested {
                let (command, kind) = match error.kind() {
                    ErrorKind::DisplayVersion => ("cli.version", "version"),
                    _ => ("cli.help", "help"),
                };
                let text = console::strip_ansi_codes(&error.to_string())
                    .trim()
                    .to_owned();
                let data = serde_json::json!({
                    "kind": kind,
                    "text": text,
                });
                if let Err(write_error) = output::write_machine_success(command, &data, Vec::new())
                {
                    report_emergency_error(&write_error);
                    return ExitCode::FAILURE;
                }
                return exit_code(code);
            }
            if let Err(print_error) = error.print() {
                report_emergency_error(
                    &AppError::io("failed to write help", &print_error).with_hint(
                        "Check that the output stream is open and writable, then retry.",
                    ),
                );
                return ExitCode::FAILURE;
            }
            return exit_code(code);
        }
        Err(error) => {
            let code = error.exit_code();
            // The mapper uses Clap's structural error kind plus exact comparisons against
            // allowlisted command and argument identities. Never render parser context or Clap's
            // raw diagnostic: either can contain arbitrary argv values, including credentials
            // pasted in the wrong position.
            let app_error = cli::parse_app_error(&arguments, &error);
            if machine_requested {
                if let Err(write_error) = output::write_machine_error("cli.parse", &app_error) {
                    report_emergency_error(&write_error);
                    return ExitCode::FAILURE;
                }
            } else if let Err(write_error) = output::write_human_error(&app_error) {
                report_emergency_error(&write_error);
                return ExitCode::FAILURE;
            }
            return exit_code(code);
        }
    };

    let context = Context::new(&cli);
    context.configure_terminal();
    let command_name = cli
        .command
        .as_ref()
        .map_or("welcome", Commands::machine_name);

    let result = match cli.command {
        None => commands::welcome::run(&context).map(|()| CommandOutcome::Success),
        Some(
            command @ (Commands::Login(_)
            | Commands::Profiles
            | Commands::Use(_)
            | Commands::Logout(_)
            | Commands::Profile(_)
            | Commands::Site(_)
            | Commands::Environment(_)
            | Commands::Ssh(_)
            | Commands::Inventory(_)),
        ) => run_provider_command(command, &context),
        Some(Commands::Guide(arguments)) => {
            commands::guide::run(arguments, &context).map(|()| CommandOutcome::Success)
        }
        Some(Commands::Search(arguments)) => {
            commands::search::run(arguments, &context).map(|()| CommandOutcome::Success)
        }
        Some(Commands::Doctor) => commands::doctor::run(&context).map(|()| CommandOutcome::Success),
        Some(Commands::Completion(arguments)) => {
            commands::completion::run(arguments, &context).map(|()| CommandOutcome::Success)
        }
    };

    match result {
        Ok(CommandOutcome::Success) => ExitCode::SUCCESS,
        Ok(CommandOutcome::Exit(code)) => ExitCode::from(code),
        Err(error) => {
            if let Err(write_error) = output::write_error(&context, command_name, &error) {
                report_emergency_error(&write_error);
                return ExitCode::FAILURE;
            }
            ExitCode::from(error.code().exit_code())
        }
    }
}

fn report_emergency_error(error: &AppError) {
    if output::write_human_error(error).is_err() {
        // There is no remaining reliable output channel. Make one final best-effort write with
        // the complete curated diagnostic rather than silently dropping its code and hint.
        eprintln!("error: {}", error.message());
        eprintln!("  code: {}", error.code());
        if let Some(hint) = error.hint().filter(|hint| !hint.trim().is_empty()) {
            eprintln!("  hint: {hint}");
        }
    }
}

fn run_provider_command(
    command: Commands,
    context: &Context,
) -> hostbraid_core::Result<CommandOutcome> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| {
            AppError::new(
                ErrorCode::Internal,
                "could not initialize the provider command runtime",
            )
        })?;
    runtime.block_on(commands::provider::run(command, context))
}

fn exit_code(code: i32) -> ExitCode {
    u8::try_from(code).map_or(ExitCode::FAILURE, ExitCode::from)
}

/// Return the complete Clap command tree for completion generators and tests.
#[must_use]
pub fn command() -> clap::Command {
    Cli::command()
}

#[cfg(test)]
mod tests {
    use super::command;

    #[test]
    fn clap_definition_is_internally_consistent() {
        command().debug_assert();
    }
}
