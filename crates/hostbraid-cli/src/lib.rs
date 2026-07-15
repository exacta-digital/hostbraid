mod cli;
mod commands;
mod context;
mod output;

use crate::cli::{Cli, Commands};
use crate::context::Context;
use clap::{CommandFactory, Parser, error::ErrorKind};
use hostbraid_core::{AppError, ErrorCode};
use std::ffi::OsString;
use std::process::ExitCode;

pub use cli::{ColorChoice, OutputFormat};

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Parse process arguments, run one command, and return a stable exit status.
#[must_use]
pub fn main_entry() -> ExitCode {
    let arguments: Vec<OsString> = std::env::args_os().collect();
    let machine_requested = cli::machine_output_requested(&arguments);

    let cli = match Cli::try_parse_from(&arguments) {
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
                    eprintln!("error: {write_error}");
                    return ExitCode::FAILURE;
                }
                return exit_code(code);
            }
            if let Err(print_error) = error.print() {
                eprintln!("error: failed to write help: {print_error}");
                return ExitCode::FAILURE;
            }
            return exit_code(code);
        }
        Err(error) => {
            let code = error.exit_code();
            if machine_requested {
                // Clap's detailed error can echo arbitrary argv values. Machine output uses a
                // curated message so accidental secrets in invalid argv are not serialized.
                let app_error = AppError::new(
                    ErrorCode::InvalidArguments,
                    "command-line arguments were invalid",
                )
                .with_hint("Run `hostbraid help` or `hostbraid search <term>`.");
                if let Err(write_error) = output::write_machine_error("cli.parse", &app_error) {
                    eprintln!("error: {write_error}");
                    return ExitCode::FAILURE;
                }
            } else if let Err(print_error) = error.print() {
                eprintln!("error: failed to write argument error: {print_error}");
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
        None => commands::welcome::run(&context),
        Some(Commands::Guide(arguments)) => commands::guide::run(arguments, &context),
        Some(Commands::Search(arguments)) => commands::search::run(arguments, &context),
        Some(Commands::Doctor) => commands::doctor::run(&context),
        Some(Commands::Completion(arguments)) => commands::completion::run(arguments, &context),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if let Err(write_error) = output::write_error(&context, command_name, &error) {
                eprintln!("error: {write_error}");
                return ExitCode::FAILURE;
            }
            ExitCode::from(error.code().exit_code())
        }
    }
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
