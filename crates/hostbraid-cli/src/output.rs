use crate::VERSION;
use crate::context::Context;
use hostbraid_core::{
    AppError, ErrorCode, MachineFailure, MachinePartialFailure, MachineSuccess, MachineWarning,
    Result,
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
    let envelope = MachineFailure::new(command, remediated_error(command, error), VERSION);
    write_json(&envelope)
}

pub(crate) fn write_machine_partial_failure<T: Serialize>(
    command: &str,
    error: &AppError,
    data: &T,
    warnings: Vec<MachineWarning>,
) -> Result<()> {
    let envelope = MachinePartialFailure::new(
        command,
        remediated_error(command, error),
        data,
        warnings,
        VERSION,
    );
    write_json(&envelope)
}

pub(crate) fn write_error(context: &Context, command: &str, error: &AppError) -> Result<()> {
    if context.output.is_machine() {
        return write_machine_error(command, error);
    }

    write_human_error_for(command, error)
}

pub(crate) fn write_human_error(error: &AppError) -> Result<()> {
    write_human_error_for("cli.parse", error)
}

fn write_human_error_for(command: &str, error: &AppError) -> Result<()> {
    let error = remediated_error(command, error);
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
    let contents = json_line(value)?;
    let mut stdout = io::stdout().lock();
    stdout.write_all(&contents).map_err(|error| {
        AppError::io("failed to write machine output", &error)
            .with_hint("Check that stdout is open and writable, then retry the command.")
    })
}

fn json_line<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let mut contents = serde_json::to_vec(value).map_err(|_| {
        AppError::new(
            hostbraid_core::ErrorCode::Internal,
            "failed to serialize machine output",
        )
        .with_hint(
            "Retry once. If it repeats, update HostBraid and report the command and error code.",
        )
    })?;
    contents.push(b'\n');
    Ok(contents)
}

fn remediated_error(command: &str, error: &AppError) -> AppError {
    if error.hint().is_some_and(|hint| !hint.trim().is_empty()) {
        return error.clone();
    }

    let help = command_help(command);
    let hint = match error.code() {
        ErrorCode::InvalidArguments | ErrorCode::InvalidInput => {
            format!("Review the indicated input, then run `{help}` to check the accepted syntax before retrying.")
        }
        ErrorCode::NotFound => format!(
            "Refresh or list the relevant resources, then retry with an exact provider:name reference or opaque ID. Run `{help}` for the discovery command."
        ),
        ErrorCode::AmbiguousTarget => format!(
            "List the relevant resources and retry with one exact canonical reference or opaque ID. Run `{help}` for selector details."
        ),
        ErrorCode::Unsupported => {
            "Use a supported command mode or update HostBraid. If the feature should be available, report the operation and error code."
                .to_owned()
        }
        ErrorCode::Unavailable => {
            "Check that the required local or provider capability is enabled and accessible, then retry."
                .to_owned()
        }
        ErrorCode::AuthenticationFailed => {
            "Run `hb profile credential set provider:name` with an active credential, then retry. Never pass credentials as command arguments."
                .to_owned()
        }
        ErrorCode::ProviderUnavailable => {
            "Check network access and provider status, then retry. If it persists, update HostBraid and report the operation and error code."
                .to_owned()
        }
        ErrorCode::DependencyMissing => {
            "Run `hb doctor`, install or repair the missing local tool, then retry.".to_owned()
        }
        ErrorCode::PolicyDenied => {
            "Review the selected action, account permissions, and any required confirmation before retrying."
                .to_owned()
        }
        ErrorCode::RemoteExecutionFailed => {
            "Inspect each target's failure and captured stderr, correct the command or SSH access, then retry failed environments by exact ID."
                .to_owned()
        }
        ErrorCode::Io => {
            "Check terminal access, file ownership, permissions, and free disk space, then retry. Run `hb doctor` if a local tool was involved."
                .to_owned()
        }
        ErrorCode::Internal => {
            "Retry once. If it repeats, update HostBraid and report the operation, error code, and `hb --version` output without including secrets."
                .to_owned()
        }
        _ => {
            "Review the error and retry. If it persists, update HostBraid and report the operation and error code without including secrets."
                .to_owned()
        }
    };
    error.clone().with_hint(hint)
}

fn command_help(command: &str) -> String {
    let path = match command {
        "welcome" | "cli.parse" => String::new(),
        "guide.show" | "guide.list" => "guide".to_owned(),
        other => other.replace('.', " "),
    };
    if path.is_empty() {
        "hb --help".to_owned()
    } else {
        format!("hb {path} --help")
    }
}

#[cfg(test)]
mod tests {
    use super::{json_line, remediated_error};
    use hostbraid_core::{AppError, ErrorCode};
    use serde::ser::{Error as _, Serialize, Serializer};

    #[test]
    fn remediation_fallback_covers_every_error_code() {
        let codes = [
            ErrorCode::InvalidArguments,
            ErrorCode::InvalidInput,
            ErrorCode::NotFound,
            ErrorCode::AmbiguousTarget,
            ErrorCode::Unsupported,
            ErrorCode::Unavailable,
            ErrorCode::AuthenticationFailed,
            ErrorCode::ProviderUnavailable,
            ErrorCode::DependencyMissing,
            ErrorCode::PolicyDenied,
            ErrorCode::RemoteExecutionFailed,
            ErrorCode::Io,
            ErrorCode::Internal,
        ];

        for code in codes {
            let error = remediated_error("environment.show", &AppError::new(code, "failure"));
            assert!(error.hint().is_some_and(|hint| !hint.trim().is_empty()));
        }
    }

    #[test]
    fn remediation_preserves_a_specific_hint() {
        let error = AppError::new(ErrorCode::NotFound, "missing")
            .with_hint("Run the exact recovery command.");

        assert_eq!(
            remediated_error("profile.show", &error).hint(),
            Some("Run the exact recovery command.")
        );
    }

    #[test]
    fn remediation_replaces_empty_and_whitespace_hints() {
        for hint in ["", " \t\n"] {
            let error = AppError::new(ErrorCode::NotFound, "missing").with_hint(hint);
            let remediated = remediated_error("profile.show", &error);

            assert!(
                remediated
                    .hint()
                    .is_some_and(|value| !value.trim().is_empty())
            );
        }
    }

    #[test]
    fn serialization_failure_happens_before_any_json_line_is_produced() {
        struct FailingSerialize;

        impl Serialize for FailingSerialize {
            fn serialize<S>(&self, _serializer: S) -> std::result::Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                Err(S::Error::custom("private serializer detail"))
            }
        }

        let error = json_line(&FailingSerialize).expect_err("serialization must fail");
        assert_eq!(error.code(), ErrorCode::Internal);
        assert_eq!(error.message(), "failed to serialize machine output");
        assert!(error.hint().is_some_and(|hint| !hint.trim().is_empty()));
        assert!(!format!("{error:?}").contains("private serializer detail"));
    }
}
