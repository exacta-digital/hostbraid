use crate::VERSION;
use crate::context::Context;
use crate::output;
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

    let contents = format!(
        "{} {}\n{}\n\n  {}  Read the two-minute introduction\n  {}  Check SSH and transfer tools\n  {}  Find any command or guide\n\n{}\n",
        console::style("HostBraid").cyan().bold(),
        console::style(format!("v{VERSION}")).dim(),
        "Bring every hosting environment within reach.",
        console::style("hostbraid guide getting-started").green(),
        console::style("hostbraid doctor").green(),
        console::style("hostbraid search <term>").green(),
        console::style("Open source by It's Ed · https://itsed.se").dim(),
    );
    output::write_human(&contents)
}
