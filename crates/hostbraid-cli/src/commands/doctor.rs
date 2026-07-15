use crate::context::Context;
use crate::output;
use hostbraid_core::{MachineWarning, Result};
use serde::Serialize;
use std::io::Read;
use std::process::{Child, ExitStatus, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const CAPTURE_LIMIT: usize = 4 * 1024;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum Requirement {
    Required,
    Recommended,
    Optional,
    Fallback,
}

impl Requirement {
    const fn label(self) -> &'static str {
        match self {
            Self::Required => "required",
            Self::Recommended => "recommended",
            Self::Optional => "optional",
            Self::Fallback => "fallback",
        }
    }
}

#[derive(Debug, Serialize)]
struct ToolCheck {
    name: &'static str,
    requirement: Requirement,
    available: bool,
    healthy: bool,
    timed_out: bool,
    exit_code: Option<i32>,
    output_truncated: bool,
    version: Option<String>,
    purpose: &'static str,
    install_hint: &'static str,
}

#[derive(Debug, Serialize)]
struct TransferStrategies {
    rsync_over_ssh: bool,
    sftp: bool,
    tar_over_ssh: bool,
}

impl TransferStrategies {
    const fn any(&self) -> bool {
        self.rsync_over_ssh || self.sftp || self.tar_over_ssh
    }
}

#[derive(Debug, Serialize)]
struct DoctorReport {
    ready_for_ssh: bool,
    transfer_strategy_available: bool,
    wp_cli_available: bool,
    transfer_strategies: TransferStrategies,
    checks: Vec<ToolCheck>,
}

struct ToolProbe {
    name: &'static str,
    arguments: &'static [&'static str],
    success_required: bool,
    capture_version: bool,
    requirement: Requirement,
    purpose: &'static str,
    install_hint: &'static str,
}

const PROBES: [ToolProbe; 5] = [
    ToolProbe {
        name: "ssh",
        arguments: &["-V"],
        success_required: true,
        capture_version: true,
        requirement: Requirement::Required,
        purpose: "Interactive access and remote command transport",
        install_hint: "Install an OpenSSH client.",
    },
    ToolProbe {
        name: "rsync",
        arguments: &["--version"],
        success_required: true,
        capture_version: true,
        requirement: Requirement::Recommended,
        purpose: "Efficient, resumable file pulls",
        install_hint: "Install rsync for the preferred pull strategy.",
    },
    ToolProbe {
        name: "sftp",
        arguments: &["-h"],
        // OpenSSH sftp uses a non-zero status for its help probe on some platforms. Spawning and
        // returning bounded help output is sufficient to establish this fallback's availability.
        success_required: false,
        capture_version: false,
        requirement: Requirement::Fallback,
        purpose: "File transfer when rsync is unavailable",
        install_hint: "Install the OpenSSH client tools.",
    },
    ToolProbe {
        name: "tar",
        arguments: &["--version"],
        success_required: true,
        capture_version: true,
        requirement: Requirement::Fallback,
        purpose: "Streaming file-transfer fallback over SSH",
        install_hint: "Install a tar implementation.",
    },
    ToolProbe {
        name: "wp",
        arguments: &["cli", "version"],
        success_required: true,
        capture_version: true,
        requirement: Requirement::Optional,
        purpose: "WordPress operations delegated to WP-CLI",
        install_hint: "Install WP-CLI when remote WordPress commands are needed.",
    },
];

pub(crate) fn run(context: &Context) -> Result<()> {
    let spinner = context.spinner("Checking local workflow tools");
    let checks: Vec<ToolCheck> = PROBES.iter().map(check_tool).collect();
    spinner.finish_and_clear();

    let ready_for_ssh = healthy(&checks, "ssh");
    let transfer_strategies = TransferStrategies {
        rsync_over_ssh: ready_for_ssh && healthy(&checks, "rsync"),
        sftp: healthy(&checks, "sftp"),
        tar_over_ssh: ready_for_ssh && healthy(&checks, "tar"),
    };
    let report = DoctorReport {
        ready_for_ssh,
        transfer_strategy_available: transfer_strategies.any(),
        wp_cli_available: healthy(&checks, "wp"),
        transfer_strategies,
        checks,
    };

    let mut warnings = Vec::new();
    if !report.ready_for_ssh {
        warnings.push(MachineWarning::new(
            "ssh_unavailable",
            "A healthy OpenSSH client was not found; SSH workflows will not work",
        ));
    }
    if !report.transfer_strategy_available {
        warnings.push(MachineWarning::new(
            "transfer_strategy_unavailable",
            "No complete local transfer strategy was found",
        ));
    }
    for check in &report.checks {
        if check.timed_out {
            warnings.push(MachineWarning::new(
                "tool_probe_timeout",
                format!("The `{}` diagnostic probe timed out", check.name),
            ));
        }
    }

    // `doctor` is a diagnostic report: missing optional or required tools are represented by
    // booleans and warnings, while failure to produce the report itself remains an error.
    if context.output.is_machine() {
        return output::write_machine_success("doctor", &report, warnings);
    }
    render_human(&report)
}

fn check_tool(probe: &ToolProbe) -> ToolCheck {
    let spawn = std::process::Command::new(probe.name)
        .args(probe.arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let Ok(mut child) = spawn else {
        return missing_check(probe);
    };

    let stdout_reader = child
        .stdout
        .take()
        .map(|reader| thread::spawn(move || read_bounded(reader)));
    let stderr_reader = child
        .stderr
        .take()
        .map(|reader| thread::spawn(move || read_bounded(reader)));
    let outcome = wait_bounded(&mut child, PROBE_TIMEOUT);
    let stdout = join_reader(stdout_reader);
    let stderr = join_reader(stderr_reader);
    let output_truncated = stdout.truncated || stderr.truncated;
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&stdout.bytes),
        String::from_utf8_lossy(&stderr.bytes)
    );
    let version = if probe.capture_version {
        combined
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(|line| line.chars().take(160).collect())
    } else {
        None
    };
    let (healthy, timed_out, exit_code) = match outcome {
        ProbeOutcome::Exited(status) => (
            !probe.success_required || status.success(),
            false,
            status.code(),
        ),
        ProbeOutcome::TimedOut => (false, true, None),
        ProbeOutcome::WaitFailed => (false, false, None),
    };

    ToolCheck {
        name: probe.name,
        requirement: probe.requirement,
        available: true,
        healthy,
        timed_out,
        exit_code,
        output_truncated,
        version,
        purpose: probe.purpose,
        install_hint: probe.install_hint,
    }
}

fn missing_check(probe: &ToolProbe) -> ToolCheck {
    ToolCheck {
        name: probe.name,
        requirement: probe.requirement,
        available: false,
        healthy: false,
        timed_out: false,
        exit_code: None,
        output_truncated: false,
        version: None,
        purpose: probe.purpose,
        install_hint: probe.install_hint,
    }
}

enum ProbeOutcome {
    Exited(ExitStatus),
    TimedOut,
    WaitFailed,
}

fn wait_bounded(child: &mut Child, timeout: Duration) -> ProbeOutcome {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return ProbeOutcome::Exited(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return ProbeOutcome::TimedOut;
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return ProbeOutcome::WaitFailed;
            }
        }
    }
}

#[derive(Default)]
struct CapturedOutput {
    bytes: Vec<u8>,
    truncated: bool,
}

fn read_bounded(mut reader: impl Read) -> CapturedOutput {
    let mut output = CapturedOutput {
        bytes: Vec::with_capacity(CAPTURE_LIMIT),
        truncated: false,
    };
    let mut buffer = [0_u8; 1024];
    loop {
        let bytes_read = match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(bytes_read) => bytes_read,
        };
        let remaining = CAPTURE_LIMIT.saturating_sub(output.bytes.len());
        let kept = remaining.min(bytes_read);
        output.bytes.extend_from_slice(&buffer[..kept]);
        output.truncated |= kept < bytes_read;
    }
    output
}

fn join_reader(reader: Option<JoinHandle<CapturedOutput>>) -> CapturedOutput {
    reader
        .and_then(|reader| reader.join().ok())
        .unwrap_or_default()
}

fn healthy(checks: &[ToolCheck], name: &str) -> bool {
    checks
        .iter()
        .any(|check| check.name == name && check.healthy)
}

fn render_human(report: &DoctorReport) -> Result<()> {
    let mut contents = format!("{}\n\n", console::style("HostBraid doctor").cyan().bold());
    for check in &report.checks {
        let mark = if check.healthy {
            console::style("✓").green().bold()
        } else if check.available {
            console::style("!").yellow().bold()
        } else {
            console::style("–").yellow().bold()
        };
        let detail = check_detail(check);
        contents.push_str(&format!(
            "  {mark} {:<8} {:<11} {detail}\n      {}\n",
            console::style(check.name).bold(),
            console::style(check.requirement.label()).dim(),
            console::style(check.purpose).dim(),
        ));
    }

    contents.push_str(&format!(
        "\n  SSH workflows       {}\n  Transfer strategy   {}\n  WP-CLI bridge       {}\n",
        readiness(report.ready_for_ssh),
        readiness(report.transfer_strategy_available),
        if report.wp_cli_available {
            console::style("available").green()
        } else {
            console::style("optional").dim()
        },
    ));
    output::write_human(&contents)
}

fn check_detail(check: &ToolCheck) -> String {
    if !check.available {
        return check.install_hint.to_owned();
    }
    if check.timed_out {
        return "diagnostic probe timed out".to_owned();
    }
    if !check.healthy {
        return check.exit_code.map_or_else(
            || "diagnostic probe failed".to_owned(),
            |code| format!("diagnostic probe exited with status {code}"),
        );
    }
    let mut detail = check.version.as_deref().unwrap_or("available").to_owned();
    if check.output_truncated {
        detail.push_str(" (output truncated)");
    }
    detail
}

fn readiness(ready: bool) -> console::StyledObject<&'static str> {
    if ready {
        console::style("ready").green()
    } else {
        console::style("not ready").yellow()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CAPTURE_LIMIT, CapturedOutput, Requirement, ToolCheck, TransferStrategies, healthy,
        read_bounded,
    };
    use std::io::Cursor;

    fn check(name: &'static str, healthy: bool) -> ToolCheck {
        ToolCheck {
            name,
            requirement: Requirement::Fallback,
            available: healthy,
            healthy,
            timed_out: false,
            exit_code: Some(0),
            output_truncated: false,
            version: None,
            purpose: "test",
            install_hint: "test",
        }
    }

    #[test]
    fn transfer_readiness_requires_a_complete_strategy() {
        let tar_only = vec![check("ssh", false), check("tar", true)];
        let ssh_and_tar = vec![check("ssh", true), check("tar", true)];
        let sftp = vec![check("sftp", true)];

        let strategies = |checks: &[ToolCheck]| TransferStrategies {
            rsync_over_ssh: healthy(checks, "ssh") && healthy(checks, "rsync"),
            sftp: healthy(checks, "sftp"),
            tar_over_ssh: healthy(checks, "ssh") && healthy(checks, "tar"),
        };
        assert!(!strategies(&tar_only).any());
        assert!(strategies(&ssh_and_tar).any());
        assert!(strategies(&sftp).any());
    }

    #[test]
    fn diagnostic_capture_drains_but_bounds_memory() {
        let input = vec![b'x'; CAPTURE_LIMIT * 3];
        let CapturedOutput { bytes, truncated } = read_bounded(Cursor::new(input));

        assert_eq!(bytes.len(), CAPTURE_LIMIT);
        assert!(truncated);
    }
}
