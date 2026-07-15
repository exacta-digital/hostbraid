use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static CONFIG_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TestConfigHome(PathBuf);

impl TestConfigHome {
    fn new() -> Self {
        let sequence = CONFIG_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!(
            "hostbraid-cli-integration-{}-{sequence}",
            std::process::id()
        )))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestConfigHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run(arguments: &[&str]) -> Output {
    run_binary(env!("CARGO_BIN_EXE_hostbraid"), arguments)
}

fn run_binary(binary: &str, arguments: &[&str]) -> Output {
    Command::new(binary)
        .args(arguments)
        .output()
        .expect("HostBraid binary runs")
}

fn run_with_config(arguments: &[&str], config_home: &TestConfigHome) -> Output {
    Command::new(env!("CARGO_BIN_EXE_hostbraid"))
        .args(arguments)
        .env("HOSTBRAID_CONFIG_HOME", config_home.path())
        .output()
        .expect("HostBraid binary runs")
}

fn write_environment_profile(config_home: &TestConfigHome, is_default: bool) {
    std::fs::create_dir_all(config_home.path()).expect("create test config home");
    let default_profile = if is_default {
        r#"{"provider": "kinsta", "profile": "agency"}"#
    } else {
        "null"
    };
    std::fs::write(
        config_home.path().join("profiles.json"),
        format!(
            r#"{{
  "schema_version": 1,
  "default_profile": {default_profile},
  "profiles": [{{
    "provider": "kinsta",
    "name": "agency",
    "company_id": "company-1",
    "credential_source": {{"type": "environment", "variable": "UNSET_TEST_TOKEN"}}
  }}]
}}"#
        ),
    )
    .expect("write secret-free profile configuration");
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
fn profile_facade_is_visible_in_help_and_search() {
    let commands = ["login", "profiles", "use", "logout"];
    let root_help = run_binary(env!("CARGO_BIN_EXE_hb"), &["--help"]);

    assert!(root_help.status.success());
    let root_help = stdout(&root_help);
    for command in commands {
        assert!(
            root_help
                .lines()
                .any(|line| line.trim_start().starts_with(&format!("{command} "))),
            "root help omitted `{command}`"
        );

        let help = run_binary(env!("CARGO_BIN_EXE_hb"), &[command, "--help"]);
        assert!(help.status.success(), "`hb {command} --help` failed");
        assert!(
            stdout(&help).contains(&format!("Usage: hb {command}")),
            "help for `{command}` did not use its public command name"
        );

        let search = run_binary(
            env!("CARGO_BIN_EXE_hb"),
            &["search", command, "--output=json"],
        );
        assert!(search.status.success(), "search for `{command}` failed");
        let value = json(&search);
        assert!(
            value["data"].as_array().is_some_and(|results| {
                results
                    .iter()
                    .any(|result| result["name"] == format!("hostbraid {command}"))
            }),
            "search omitted `{command}`"
        );
    }

    let long_help = run(&["login", "--help"]);
    assert!(long_help.status.success());
    assert!(stdout(&long_help).contains("Usage: hostbraid login"));
}

#[test]
fn profile_facade_help_describes_its_safe_arguments() {
    let cases = [
        (
            "login",
            &["<PROVIDER>", "<NAME>", "--token-stdin", "--credential-env"][..],
        ),
        ("profiles", &[][..]),
        ("use", &["<PROFILE>"][..]),
        ("logout", &["<PROFILE>", "--yes"][..]),
    ];

    for (command, expected) in cases {
        let output = run_binary(env!("CARGO_BIN_EXE_hb"), &[command, "--help"]);
        assert!(output.status.success(), "`hb {command} --help` failed");
        let help = stdout(&output);
        for fragment in expected {
            assert!(
                help.contains(fragment),
                "help for `{command}` omitted `{fragment}`"
            );
        }
    }

    let login = run_binary(env!("CARGO_BIN_EXE_hb"), &["login", "--help"]);
    assert!(!stdout(&login).contains("--default"));
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
fn explicit_human_output_overrides_a_machine_environment_default() {
    let output = Command::new(env!("CARGO_BIN_EXE_hostbraid"))
        .args(["--output", "human", "definitely-not-a-command"])
        .env("HOSTBRAID_OUTPUT", "json")
        .output()
        .expect("HostBraid binary runs");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("command-line arguments were invalid")
    );
    assert!(!String::from_utf8_lossy(&output.stderr).contains("definitely-not-a-command"));
}

#[test]
fn clustered_short_output_is_detected_for_parse_failures() {
    let output = run(&["-qojson", "definitely-not-a-command"]);

    assert_eq!(output.status.code(), Some(2));
    let value = json(&output);
    assert_eq!(value["command"], "cli.parse");
    assert_eq!(value["error"]["code"], "invalid_arguments");
}

#[test]
fn trailing_remote_arguments_do_not_override_machine_parse_errors() {
    let output = Command::new(env!("CARGO_BIN_EXE_hostbraid"))
        .args(["ssh", "run", "--jobs", "0", "echo", "--output", "human"])
        .env("HOSTBRAID_OUTPUT", "json")
        .output()
        .expect("HostBraid binary runs");

    assert_eq!(output.status.code(), Some(2));
    let value = json(&output);
    assert_eq!(value["command"], "cli.parse");
    assert_eq!(value["error"]["code"], "invalid_arguments");
}

#[test]
fn malformed_prefixes_do_not_expose_trailing_remote_output_options() {
    let machine = Command::new(env!("CARGO_BIN_EXE_hostbraid"))
        .args([
            "--definitely-invalid",
            "ssh",
            "run",
            "echo",
            "--output",
            "human",
        ])
        .env("HOSTBRAID_OUTPUT", "json")
        .output()
        .expect("HostBraid binary runs");

    assert_eq!(machine.status.code(), Some(2));
    assert_eq!(json(&machine)["command"], "cli.parse");

    let human = run(&[
        "--definitely-invalid",
        "ssh",
        "run",
        "echo",
        "--output",
        "json",
    ]);
    assert_eq!(human.status.code(), Some(2));
    assert!(human.stdout.is_empty());
    assert!(String::from_utf8_lossy(&human.stderr).contains("command-line arguments were invalid"));

    let nested = Command::new(env!("CARGO_BIN_EXE_hostbraid"))
        .args([
            "ssh",
            "--definitely-invalid",
            "run",
            "echo",
            "--output",
            "human",
        ])
        .env("HOSTBRAID_OUTPUT", "json")
        .output()
        .expect("HostBraid binary runs");
    assert_eq!(nested.status.code(), Some(2));
    assert_eq!(json(&nested)["command"], "cli.parse");
}

#[test]
fn ssh_words_inside_another_command_do_not_hide_global_output() {
    let output = run(&["search", "ssh", "run", "echo", "--output", "json"]);

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(json(&output)["command"], "cli.parse");
}

#[cfg(unix)]
#[test]
fn non_utf8_remote_commands_do_not_expose_trailing_output_options() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let output = Command::new(env!("CARGO_BIN_EXE_hostbraid"))
        .args([
            OsString::from("ssh"),
            OsString::from("run"),
            OsString::from("--jobs"),
            OsString::from("0"),
            OsString::from_vec(vec![0xff]),
            OsString::from("--output"),
            OsString::from("human"),
        ])
        .env("HOSTBRAID_OUTPUT", "json")
        .output()
        .expect("HostBraid binary runs");

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(json(&output)["command"], "cli.parse");
}

#[test]
fn human_parse_failures_do_not_echo_accidental_secret_arguments() {
    let canary = "secret-token-canary-never-render";
    let output = run(&["profile", "add", "kinsta", "agency", canary]);

    assert_eq!(output.status.code(), Some(2));
    assert!(!stdout(&output).contains(canary));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(canary));
}

#[test]
fn login_parse_failures_do_not_echo_accidental_secret_arguments() {
    let canary = "login-secret-token-canary-never-render";
    let output = run_binary(
        env!("CARGO_BIN_EXE_hb"),
        &["login", "kinsta", "agency", canary],
    );

    assert_eq!(output.status.code(), Some(2));
    assert!(!stdout(&output).contains(canary));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(canary));
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

#[test]
fn empty_profile_list_has_a_secret_free_machine_shape() {
    let config_home = TestConfigHome::new();
    let output = run_with_config(&["profile", "list", "--output=json"], &config_home);

    assert!(output.status.success());
    let value = json(&output);
    assert_eq!(value["command"], "profile.list");
    assert_eq!(value["data"]["default_profile"], Value::Null);
    assert_eq!(value["data"]["profiles"], serde_json::json!([]));
}

#[test]
fn login_requires_a_safe_credential_source_before_network_access() {
    let config_home = TestConfigHome::new();
    let output = run_with_config(
        &["login", "kinsta", "agency", "--output=json"],
        &config_home,
    );

    assert_eq!(output.status.code(), Some(2));
    let value = json(&output);
    assert_eq!(value["command"], "profile.add");
    assert_eq!(value["error"]["code"], "invalid_input");
    assert_eq!(
        value["error"]["message"],
        "a credential source is required in non-interactive mode"
    );
}

#[test]
fn profiles_uses_the_canonical_secret_free_machine_contract() {
    let config_home = TestConfigHome::new();
    let output = run_with_config(&["profiles", "--output=json"], &config_home);

    assert!(output.status.success());
    let value = json(&output);
    assert_eq!(value["command"], "profile.list");
    assert_eq!(value["data"]["default_profile"], Value::Null);
    assert_eq!(value["data"]["profiles"], serde_json::json!([]));
}

#[test]
fn use_sets_an_exact_default_without_resolving_its_credential() {
    let config_home = TestConfigHome::new();
    write_environment_profile(&config_home, false);
    let output = run_with_config(&["use", "kinsta:agency", "--output=json"], &config_home);

    assert!(output.status.success());
    let value = json(&output);
    assert_eq!(value["command"], "profile.default");
    assert_eq!(value["data"]["reference"]["provider"], "kinsta");
    assert_eq!(value["data"]["reference"]["profile"], "agency");
    assert_eq!(value["data"]["is_default"], true);
    assert_eq!(
        value["data"]["credential_source"]["variable"],
        "UNSET_TEST_TOKEN"
    );
}

#[test]
fn logout_removes_an_exact_profile_without_resolving_its_credential() {
    let config_home = TestConfigHome::new();
    write_environment_profile(&config_home, true);
    let output = run_with_config(
        &["logout", "kinsta:agency", "--yes", "--output=json"],
        &config_home,
    );

    assert!(output.status.success());
    let value = json(&output);
    assert_eq!(value["command"], "profile.remove");
    assert_eq!(value["data"]["profile"]["reference"]["provider"], "kinsta");
    assert_eq!(value["data"]["profile"]["reference"]["profile"], "agency");
    assert_eq!(value["data"]["credential_cleanup_failed"], false);

    let profiles = run_with_config(&["profiles", "--output=json"], &config_home);
    assert!(profiles.status.success());
    let profiles = json(&profiles);
    assert_eq!(profiles["data"]["default_profile"], Value::Null);
    assert_eq!(profiles["data"]["profiles"], serde_json::json!([]));
}

#[test]
fn profile_show_uses_a_structured_reference_and_never_resolves_the_token() {
    let config_home = TestConfigHome::new();
    std::fs::create_dir_all(config_home.path()).expect("create test config home");
    std::fs::write(
        config_home.path().join("profiles.json"),
        r#"{
  "schema_version": 1,
  "default_profile": {"provider": "kinsta", "profile": "agency"},
  "profiles": [{
    "provider": "kinsta",
    "name": "agency",
    "company_id": "company-1",
    "credential_source": {"type": "environment", "variable": "UNSET_TEST_TOKEN"}
  }]
}"#,
    )
    .expect("write secret-free profile configuration");
    let output = run_with_config(
        &["profile", "show", "kinsta:agency", "--output=json"],
        &config_home,
    );

    assert!(output.status.success());
    let value = json(&output);
    assert_eq!(value["data"]["reference"]["provider"], "kinsta");
    assert_eq!(value["data"]["reference"]["profile"], "agency");
    assert_eq!(value["data"]["company_id"], "company-1");
    assert_eq!(value["data"]["is_default"], true);
    assert_eq!(
        value["data"]["credential_source"]["variable"],
        "UNSET_TEST_TOKEN"
    );
}

#[test]
fn provider_commands_do_not_infer_a_sole_or_missing_default() {
    let config_home = TestConfigHome::new();
    let output = run_with_config(&["site", "list", "--output=json"], &config_home);

    assert_eq!(output.status.code(), Some(2));
    let value = json(&output);
    assert_eq!(value["command"], "site.list");
    assert_eq!(value["error"]["code"], "invalid_input");
}

#[test]
fn interactive_ssh_is_rejected_in_machine_mode_before_profile_resolution() {
    let config_home = TestConfigHome::new();
    let output = run_with_config(
        &["ssh", "open", "--environment-id", "env-1", "--output=json"],
        &config_home,
    );

    assert_eq!(output.status.code(), Some(1));
    let value = json(&output);
    assert_eq!(value["command"], "ssh.open");
    assert_eq!(value["error"]["code"], "unsupported");
}

#[test]
fn broad_machine_ssh_requires_yes_before_credentials_or_network() {
    let config_home = TestConfigHome::new();
    let output = run_with_config(
        &["--output=json", "ssh", "run", "--all", "--", "uptime"],
        &config_home,
    );

    assert_eq!(output.status.code(), Some(6));
    let value = json(&output);
    assert_eq!(value["command"], "ssh.run");
    assert_eq!(value["error"]["code"], "policy_denied");
    assert!(value.get("data").is_none());
}

#[test]
fn broad_piped_human_ssh_requires_yes_before_profile_resolution() {
    let config_home = TestConfigHome::new();
    let output = run_with_config(&["ssh", "run", "--all", "--", "uptime"], &config_home);

    assert_eq!(output.status.code(), Some(6));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("broad SSH selectors require explicit non-interactive confirmation"));
    assert!(!stderr.contains("no default provider profile"));
}

#[test]
fn missing_ssh_selector_is_rejected_before_profile_resolution() {
    let config_home = TestConfigHome::new();
    let output = run_with_config(
        &["--output=json", "ssh", "run", "--", "uptime"],
        &config_home,
    );

    assert_eq!(output.status.code(), Some(2));
    let value = json(&output);
    assert_eq!(value["command"], "ssh.run");
    assert_eq!(
        value["error"]["message"],
        "at least one SSH target selector is required"
    );
}

#[test]
fn blank_inventory_search_is_rejected_before_profile_resolution() {
    let config_home = TestConfigHome::new();
    let output = run_with_config(
        &["--output=json", "inventory", "plugins", "--search", "   "],
        &config_home,
    );

    assert_eq!(output.status.code(), Some(2));
    let value = json(&output);
    assert_eq!(value["command"], "inventory.plugins");
    assert_eq!(
        value["error"]["message"],
        "inventory search text cannot be empty"
    );
}
