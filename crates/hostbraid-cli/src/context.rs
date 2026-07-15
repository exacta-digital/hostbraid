use crate::cli::{Cli, ColorChoice, OutputFormat};
use hostbraid_core::{AppError, ErrorCode, Result};
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use std::io::{self, IsTerminal, Write, stderr, stdin, stdout};
use std::time::Duration;

#[derive(Debug)]
pub(crate) struct Context {
    pub output: OutputFormat,
    pub interactive: bool,
    quiet: bool,
    color: ColorChoice,
}

impl Context {
    pub fn new(cli: &Cli) -> Self {
        Self {
            output: cli.output,
            interactive: !cli.no_input && !cli.output.is_machine() && stdin().is_terminal(),
            quiet: cli.quiet,
            color: cli.color,
        }
    }

    pub fn configure_terminal(&self) {
        let no_color = std::env::var_os("NO_COLOR").is_some();
        let stdout_color = match self.color {
            ColorChoice::Always => true,
            ColorChoice::Never => false,
            ColorChoice::Auto => stdout().is_terminal() && !no_color,
        } && !self.output.is_machine();
        let stderr_color = match self.color {
            ColorChoice::Always => true,
            ColorChoice::Never => false,
            ColorChoice::Auto => stderr().is_terminal() && !no_color,
        } && !self.output.is_machine();

        console::set_colors_enabled(stdout_color);
        console::set_colors_enabled_stderr(stderr_color);
    }

    pub fn spinner(&self, message: impl Into<String>) -> ProgressBar {
        if self.output.is_machine() || self.quiet || !stderr().is_terminal() {
            return ProgressBar::hidden();
        }

        let spinner = ProgressBar::with_draw_target(None, ProgressDrawTarget::stderr());
        spinner.set_style(
            ProgressStyle::with_template("{spinner:.cyan} {msg}")
                .unwrap_or_else(|_| ProgressStyle::default_spinner())
                .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
        );
        spinner.set_message(message.into());
        spinner.enable_steady_tick(Duration::from_millis(80));
        spinner
    }

    pub fn confirm(&self, prompt: &str) -> Result<bool> {
        if !self.interactive {
            return Err(AppError::new(
                ErrorCode::PolicyDenied,
                "HostBraid cannot ask for confirmation because input is non-interactive",
            )
            .with_hint("Rerun in a terminal, or add `--yes` after reviewing the action."));
        }

        let mut stderr = io::stderr().lock();
        write!(stderr, "{prompt} [y/N] ")
            .and_then(|()| stderr.flush())
            .map_err(|error| {
                AppError::io("failed to write confirmation prompt", &error).with_hint(
                    "Check that the terminal is available, then retry with `--yes` only after reviewing the action.",
                )
            })?;
        let mut answer = String::new();
        io::stdin()
            .read_line(&mut answer)
            .map_err(|error| {
                AppError::io("failed to read confirmation", &error).with_hint(
                    "Check that standard input is available, then retry with `--yes` only after reviewing the action.",
                )
            })?;
        Ok(matches!(
            answer.trim().to_ascii_lowercase().as_str(),
            "y" | "yes"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::Context;
    use crate::cli::{ColorChoice, OutputFormat};
    use hostbraid_core::ErrorCode;

    #[test]
    fn non_interactive_confirmation_explains_both_safe_recovery_paths() {
        let context = Context {
            output: OutputFormat::Human,
            interactive: false,
            quiet: true,
            color: ColorChoice::Never,
        };

        let error = context
            .confirm("Remove profile?")
            .expect_err("non-interactive confirmation must be explicit");

        assert_eq!(error.code(), ErrorCode::PolicyDenied);
        assert_eq!(
            error.message(),
            "HostBraid cannot ask for confirmation because input is non-interactive"
        );
        assert_eq!(
            error.hint(),
            Some("Rerun in a terminal, or add `--yes` after reviewing the action.")
        );
        assert!(!error.message().contains("target"));
        assert!(!error.hint().is_some_and(|hint| hint.contains("target")));
    }
}
