use crate::VERSION;
use crate::context::Context;
use hostbraid_core::{
    AppError, MachineFailure, MachinePartialFailure, MachineSuccess, MachineWarning, Result,
};
use serde::Serialize;
use std::io::{self, Write};

pub(crate) fn write_machine_success<T: Serialize>(
    command: &str,
    data: &T,
    warnings: Vec<MachineWarning>,
) -> Result<()> {
    let envelope = MachineSuccess::new(command, data, warnings, VERSION);
    write_json(&envelope)
}

pub(crate) fn write_machine_error(command: &str, error: &AppError) -> Result<()> {
    let envelope = MachineFailure::new(command, error.clone(), VERSION);
    write_json(&envelope)
}

pub(crate) fn write_machine_partial_failure<T: Serialize>(
    command: &str,
    error: &AppError,
    data: &T,
    warnings: Vec<MachineWarning>,
) -> Result<()> {
    let envelope = MachinePartialFailure::new(command, error.clone(), data, warnings, VERSION);
    write_json(&envelope)
}

pub(crate) fn write_error(context: &Context, command: &str, error: &AppError) -> Result<()> {
    if context.output.is_machine() {
        return write_machine_error(command, error);
    }

    write_human_error(error)
}

pub(crate) fn write_human_error(error: &AppError) -> Result<()> {
    let mut stderr = io::stderr().lock();
    writeln!(
        stderr,
        "{} {}",
        console::style("error:").red().bold(),
        error.message()
    )
    .map_err(|io_error| AppError::io("failed to write error", &io_error))?;
    writeln!(stderr, "  code: {}", error.code())
        .map_err(|io_error| AppError::io("failed to write error code", &io_error))?;
    if let Some(hint) = error.hint() {
        writeln!(stderr, "  {} {hint}", console::style("hint:").cyan())
            .map_err(|io_error| AppError::io("failed to write error hint", &io_error))?;
    }
    Ok(())
}

pub(crate) fn write_human(contents: &str) -> Result<()> {
    let mut stdout = io::stdout().lock();
    stdout
        .write_all(contents.as_bytes())
        .and_then(|()| stdout.flush())
        .map_err(|error| AppError::io("failed to write output", &error))
}

pub(crate) fn write_human_stderr(contents: &str) -> Result<()> {
    let mut stderr = io::stderr().lock();
    stderr
        .write_all(contents.as_bytes())
        .and_then(|()| stderr.flush())
        .map_err(|error| AppError::io("failed to write diagnostic output", &error))
}

fn write_json<T: Serialize>(value: &T) -> Result<()> {
    let mut stdout = io::stdout().lock();
    serde_json::to_writer(&mut stdout, value).map_err(|error| {
        AppError::new(
            hostbraid_core::ErrorCode::Internal,
            format!("failed to serialize machine output: {error}"),
        )
    })?;
    writeln!(stdout).map_err(|error| AppError::io("failed to write machine output", &error))
}
