use serde_json::Value;
use std::process::{Command, Output};

fn run(arguments: &[&str]) -> Output {
    run_binary(env!("CARGO_BIN_EXE_hostbraid"), arguments)
}

fn run_binary(binary: &str, arguments: &[&str]) -> Output {
    Command::new(binary)
        .args(arguments)
        .output()
        .expect("HostBraid binary runs")
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout is UTF-8")
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("stdout is one JSON value")
}

#[test]
fn no_arguments_lists_every_command_category() {
    let output = run(&[]);

    assert!(output.status.success());
    let stdout = stdout(&output);
    assert!(stdout.contains("HostBraid"));
    assert!(stdout.contains("hostbraid guide getting-started"));

    let mut root = hostbraid::command();
    root.build();
    for command in root
        .get_subcommands()
        .filter(|command| command.get_name() != "help" && !command.is_hide_set())
    {
        assert!(
            stdout.contains(&format!("hostbraid {}", command.get_name())),
            "default output omitted the `{}` command",
            command.get_name()
        );
    }
}

#[test]
fn hb_is_an_alias_for_the_hostbraid_binary() {
    let output = run_binary(env!("CARGO_BIN_EXE_hb"), &["--output=json", "--version"]);

    assert!(output.status.success());
    let value = json(&output);
    assert_eq!(value["command"], "cli.version");
    assert_eq!(value["data"]["kind"], "version");
}

#[test]
fn root_help_is_descriptive_and_actionable() {
    let output = run(&["--help"]);

    assert!(output.status.success());
    let stdout = stdout(&output);
    assert!(stdout.contains("Bring every hosting environment within reach"));
    assert!(stdout.contains("search ssh"));
    assert!(stdout.contains("--output <OUTPUT>"));
}

#[test]
fn help_and_version_honor_machine_mode() {
    let help = run(&["--output=json", "--help"]);
    let version = run(&["--version", "--output=json"]);

    assert!(help.status.success());
    assert!(version.status.success());
    let help = json(&help);
    let version = json(&version);
    assert_eq!(help["command"], "cli.help");
    assert_eq!(help["data"]["kind"], "help");
    assert_eq!(version["command"], "cli.version");
    assert_eq!(version["data"]["kind"], "version");
}

#[test]
fn search_has_a_versioned_machine_contract() {
    let output = run(&["search", "ssh", "--output", "json", "--no-input"]);

    assert!(output.status.success());
    let value = json(&output);
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["ok"], true);
    assert_eq!(value["command"], "search");
    assert!(
        value["data"]
            .as_array()
            .is_some_and(|items| !items.is_empty())
    );
}

#[test]
fn parse_failures_are_json_when_machine_mode_was_requested() {
    let output = run(&["--output", "json", "definitely-not-a-command"]);

    assert_eq!(output.status.code(), Some(2));
    let value = json(&output);
    assert_eq!(value["ok"], false);
    assert_eq!(value["command"], "cli.parse");
    assert_eq!(value["error"]["code"], "invalid_arguments");
    assert_eq!(
        value["error"]["message"],
        "command-line arguments were invalid"
    );
    assert!(!stdout(&output).contains("definitely-not-a-command"));
}

#[test]
fn runtime_failures_keep_their_command_identity() {
    let output = run(&["completion", "bash", "--output=json"]);

    assert_eq!(output.status.code(), Some(2));
    let value = json(&output);
    assert_eq!(value["ok"], false);
    assert_eq!(value["command"], "completion");
    assert_eq!(value["error"]["code"], "invalid_input");
}

#[test]
fn agent_guide_is_available_inside_the_binary() {
    let output = run(&["guide", "agents", "--output=json"]);

    assert!(output.status.success());
    let value = json(&output);
    assert_eq!(value["data"]["topic"], "agents");
    assert!(
        value["data"]["markdown"]
            .as_str()
            .is_some_and(|body| body.contains("Never parse human tables"))
    );
}

#[test]
fn doctor_remains_machine_readable_when_optional_tools_are_missing() {
    let output = run(&["doctor", "--output=json"]);

    assert!(output.status.success());
    let value = json(&output);
    assert_eq!(value["command"], "doctor");
    assert!(value["data"]["checks"].is_array());
}

#[test]
fn completion_writes_a_shell_script() {
    let output = run(&["completion", "bash"]);

    assert!(output.status.success());
    assert!(stdout(&output).contains("_hostbraid"));
}
