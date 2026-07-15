use crate::cli::{Cli, CompletionArgs};
use crate::context::Context;
use clap::CommandFactory;
use hostbraid_core::{AppError, ErrorCode, Result};
use std::io;

pub(crate) fn run(arguments: CompletionArgs, context: &Context) -> Result<()> {
    if context.output.is_machine() {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "shell completion output is a script, not JSON",
        )
        .with_hint("Run `hb completion <shell>` without `--output json`."));
    }

    let mut command = Cli::command();
    clap_complete::generate(
        arguments.shell,
        &mut command,
        "hostbraid",
        &mut io::stdout(),
    );
    Ok(())
}
