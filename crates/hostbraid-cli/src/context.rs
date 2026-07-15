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
                "the operation requires explicit non-interactive confirmation",
            )
            .with_hint("Review the selected targets, then rerun with `--yes`."));
        }

        let mut stderr = io::stderr().lock();
        write!(stderr, "{prompt} [y/N] ")
            .and_then(|()| stderr.flush())
            .map_err(|error| AppError::io("failed to write confirmation prompt", &error))?;
        let mut answer = String::new();
        io::stdin()
            .read_line(&mut answer)
            .map_err(|error| AppError::io("failed to read confirmation", &error))?;
        Ok(matches!(
            answer.trim().to_ascii_lowercase().as_str(),
            "y" | "yes"
        ))
    }
}
