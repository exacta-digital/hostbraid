use crate::cli::{GuideArgs, GuideTopic};
use crate::context::Context;
use crate::output;
use hostbraid_core::Result;
use serde::Serialize;

pub(super) struct GuideDocument {
    pub topic: &'static str,
    pub title: &'static str,
    pub summary: &'static str,
    pub body: &'static str,
}

const GUIDES: [GuideDocument; 5] = [
    GuideDocument {
        topic: "getting-started",
        title: "Getting started",
        summary: "Understand HostBraid's current surface and the path to your first provider.",
        body: include_str!("../../../../docs/guides/getting-started.md"),
    },
    GuideDocument {
        topic: "humans",
        title: "Human-friendly workflows",
        summary: "Learn how HostBraid keeps interactive work clear, calm, and reversible.",
        body: include_str!("../../../../docs/guides/humans.md"),
    },
    GuideDocument {
        topic: "agents",
        title: "Using HostBraid from an agent",
        summary: "Use deterministic JSON, exact targets, and non-interactive behavior.",
        body: include_str!("../../../../docs/guides/agents.md"),
    },
    GuideDocument {
        topic: "concepts",
        title: "Snapshots, exports, and pulls",
        summary: "Keep provider restore points separate from portable local artifacts.",
        body: include_str!("../../../../docs/guides/concepts.md"),
    },
    GuideDocument {
        topic: "security",
        title: "Security model",
        summary: "Understand credentials, host keys, local writes, and remote execution.",
        body: include_str!("../../../../docs/guides/security.md"),
    },
];

#[derive(Debug, Serialize)]
struct GuideSummary {
    topic: &'static str,
    title: &'static str,
    summary: &'static str,
}

#[derive(Debug, Serialize)]
struct GuideOutput {
    topic: &'static str,
    title: &'static str,
    summary: &'static str,
    markdown: &'static str,
}

pub(super) fn documents() -> &'static [GuideDocument] {
    &GUIDES
}

pub(crate) fn run(arguments: GuideArgs, context: &Context) -> Result<()> {
    if arguments.list {
        return list(context);
    }

    let topic = arguments.topic.unwrap_or(GuideTopic::GettingStarted);
    let document = document(topic);
    let data = GuideOutput {
        topic: document.topic,
        title: document.title,
        summary: document.summary,
        markdown: document.body,
    };

    if context.output.is_machine() {
        return output::write_machine_success("guide.show", &data, Vec::new());
    }

    let contents = format!(
        "{}\n{}\n\n{}\n",
        console::style(document.title).cyan().bold(),
        console::style(document.summary).dim(),
        document.body.trim()
    );
    output::write_human(&contents)
}

fn list(context: &Context) -> Result<()> {
    let guides: Vec<GuideSummary> = GUIDES
        .iter()
        .map(|guide| GuideSummary {
            topic: guide.topic,
            title: guide.title,
            summary: guide.summary,
        })
        .collect();

    if context.output.is_machine() {
        return output::write_machine_success("guide.list", &guides, Vec::new());
    }

    let mut contents = format!("{}\n\n", console::style("Built-in guides").cyan().bold());
    for guide in guides {
        contents.push_str(&format!(
            "  {:<18} {}\n",
            console::style(guide.topic).green(),
            guide.summary
        ));
    }
    contents.push_str("\nOpen one with `hostbraid guide <topic>`.\n");
    output::write_human(&contents)
}

const fn document(topic: GuideTopic) -> &'static GuideDocument {
    match topic {
        GuideTopic::GettingStarted => &GUIDES[0],
        GuideTopic::Humans => &GUIDES[1],
        GuideTopic::Agents => &GUIDES[2],
        GuideTopic::Concepts => &GUIDES[3],
        GuideTopic::Security => &GUIDES[4],
    }
}
