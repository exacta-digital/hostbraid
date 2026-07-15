use crate::VERSION;
use crate::cli::Cli;
use crate::context::Context;
use crate::output;
use clap::CommandFactory;
use hostbraid_core::Result;
use serde::Serialize;

#[derive(Debug, Serialize)]
struct Welcome<'a> {
    name: &'a str,
    version: &'a str,
    description: &'a str,
    interactive: bool,
    next_steps: [&'a str; 3],
}

pub(crate) fn run(context: &Context) -> Result<()> {
    let welcome = Welcome {
        name: "HostBraid",
        version: VERSION,
        description: "Provider-neutral hosting workflows for people and agents",
        interactive: context.interactive,
        next_steps: [
            "hostbraid guide getting-started",
            "hostbraid doctor",
            "hostbraid search <term>",
        ],
    };

    if context.output.is_machine() {
        return output::write_machine_success("welcome", &welcome, Vec::new());
    }

    let commands = command_categories();
    let command_width = commands
        .iter()
        .map(|(command, _)| command.len())
        .max()
        .unwrap_or_default();
    let mut contents = format!(
        "{} {}\n{}\n\n{}\n",
        console::style("HostBraid").cyan().bold(),
        console::style(format!("v{VERSION}")).dim(),
        "Bring every hosting environment within reach.",
        console::style("Commands").cyan().bold(),
    );
    for (command, summary) in commands {
        contents.push_str(&format!(
            "  {}  {summary}\n",
            console::style(format!("{command:<command_width$}")).green()
        ));
    }
    contents.push_str(&format!(
        "\nStart with {}.\n\n{}\n",
        console::style("hostbraid guide getting-started").green(),
        console::style("Open source by It's Ed · https://itsed.se").dim(),
    ));
    output::write_human(&contents)
}

fn command_categories() -> Vec<(String, String)> {
    let mut root = Cli::command();
    root.build();
    root.get_subcommands()
        .filter(|command| command.get_name() != "help" && !command.is_hide_set())
        .map(|command| {
            let name = format!("hostbraid {}", command.get_name());
            let summary = command
                .get_about()
                .map(ToString::to_string)
                .unwrap_or_default();
            (name, summary)
        })
        .collect()
}
