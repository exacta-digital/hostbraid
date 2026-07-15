use super::guide;
use crate::cli::{Cli, SearchArgs};
use crate::context::Context;
use crate::output;
use clap::CommandFactory;
use hostbraid_core::{AppError, ErrorCode, MachineWarning, Result};
use serde::Serialize;
use std::cmp::Ordering;

#[derive(Debug)]
struct SearchDocument {
    kind: SearchKind,
    name: String,
    summary: String,
    details: String,
    usage: String,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum SearchKind {
    Command,
    Guide,
}

impl SearchKind {
    const fn label(self) -> &'static str {
        match self {
            Self::Command => "command",
            Self::Guide => "guide",
        }
    }
}

#[derive(Debug, Serialize)]
struct SearchResult {
    kind: SearchKind,
    name: String,
    summary: String,
    usage: String,
    #[serde(skip)]
    score: u16,
}

pub(crate) fn run(arguments: SearchArgs, context: &Context) -> Result<()> {
    let query = arguments.query.trim();
    if query.is_empty() {
        return Err(
            AppError::new(ErrorCode::InvalidInput, "search query cannot be empty")
                .with_hint("Try `hostbraid search ssh` or `hostbraid guide --list`."),
        );
    }

    let mut results: Vec<SearchResult> = documents()
        .into_iter()
        .filter_map(|document| {
            let score = score(query, &document);
            (score > 0).then_some(SearchResult {
                kind: document.kind,
                name: document.name,
                summary: document.summary,
                usage: document.usage,
                score,
            })
        })
        .collect();
    results.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.name.cmp(&right.name))
    });
    results.truncate(usize::from(arguments.limit));

    let warnings = if results.is_empty() {
        vec![MachineWarning::new(
            "no_matches",
            format!("No commands or guides matched `{query}`"),
        )]
    } else {
        Vec::new()
    };

    if context.output.is_machine() {
        return output::write_machine_success("search", &results, warnings);
    }

    if results.is_empty() {
        return output::write_human(&format!(
            "No commands or guides matched {}.\nTry a broader word or run `hostbraid guide --list`.\n",
            console::style(query).yellow()
        ));
    }

    let mut contents = format!(
        "{} {}\n\n",
        console::style("Results for").dim(),
        console::style(query).cyan().bold()
    );
    for result in results {
        contents.push_str(&format!(
            "  {}  {}\n      {}\n      {}\n\n",
            console::style(format!("{:<7}", result.kind.label())).dim(),
            console::style(&result.name).green().bold(),
            result.summary,
            console::style(&result.usage).dim(),
        ));
    }
    output::write_human(&contents)
}

fn documents() -> Vec<SearchDocument> {
    let mut root = Cli::command();
    root.build();
    let mut documents = Vec::new();
    collect_commands(&root, String::new(), &mut documents);
    documents.extend(guide::documents().iter().map(|document| SearchDocument {
        kind: SearchKind::Guide,
        name: document.topic.to_owned(),
        summary: document.summary.to_owned(),
        details: document.body.to_owned(),
        usage: format!("hostbraid guide {}", document.topic),
    }));
    documents
}

fn collect_commands(command: &clap::Command, parent: String, documents: &mut Vec<SearchDocument>) {
    let path = if parent.is_empty() {
        command.get_name().to_owned()
    } else {
        format!("{parent} {}", command.get_name())
    };

    let summary = command
        .get_about()
        .map(ToString::to_string)
        .unwrap_or_default();
    let details = command
        .get_long_about()
        .map(ToString::to_string)
        .unwrap_or_else(|| summary.clone());
    documents.push(SearchDocument {
        kind: SearchKind::Command,
        name: path.clone(),
        summary,
        details,
        usage: format!("{path} --help"),
    });

    for child in command
        .get_subcommands()
        .filter(|child| child.get_name() != "help")
    {
        collect_commands(child, path.clone(), documents);
    }
}

fn score(query: &str, document: &SearchDocument) -> u16 {
    let query = query.to_lowercase();
    let name = document.name.to_lowercase();
    let summary = document.summary.to_lowercase();
    let details = document.details.to_lowercase();

    if name == query {
        return 1000;
    }
    if name.starts_with(&query) {
        return 900;
    }
    if name.contains(&query) {
        return 800;
    }
    if summary.contains(&query) {
        return 650;
    }
    if details.contains(&query) {
        return 500;
    }

    let similarity = strsim::jaro_winkler(&query, &name);
    match similarity.partial_cmp(&0.78) {
        Some(Ordering::Greater | Ordering::Equal) => (similarity * 400.0) as u16,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::{SearchDocument, SearchKind, score};

    fn document() -> SearchDocument {
        SearchDocument {
            kind: SearchKind::Command,
            name: "hostbraid doctor".to_owned(),
            summary: "Check local tools".to_owned(),
            details: "SSH and transfer diagnostics".to_owned(),
            usage: "hostbraid doctor --help".to_owned(),
        }
    }

    #[test]
    fn exact_name_beats_description_match() {
        assert_eq!(score("hostbraid doctor", &document()), 1000);
        assert_eq!(score("transfer", &document()), 500);
    }

    #[test]
    fn unrelated_query_does_not_match() {
        assert_eq!(score("pineapple", &document()), 0);
    }

    #[test]
    fn generated_help_aliases_do_not_duplicate_commands() {
        let names: Vec<String> = super::documents()
            .into_iter()
            .map(|document| document.name)
            .collect();

        assert!(names.iter().all(|name| !name.contains(" help ")));
    }
}
