use crate::{
    AppError, Capability, EnvironmentRef, EnvironmentSummary, MACHINE_SCHEMA_VERSION, SiteSummary,
    WordPressComponent, WordPressComponentKind,
};
use serde::Serialize;

/// Non-fatal condition included with a successful command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MachineWarning {
    pub code: String,
    pub message: String,
}

impl MachineWarning {
    #[must_use]
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

/// Stable `data` payload for `environment list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MachineEnvironmentListData {
    pub site: SiteSummary,
    pub environments: Vec<EnvironmentSummary>,
}

/// Stable `data` payload for `environment show`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MachineEnvironmentShowData {
    pub site: SiteSummary,
    pub environment: EnvironmentSummary,
    pub capabilities: Vec<Capability>,
}

/// Stable filtered `data` payload for plugin and theme inventory commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MachineInventoryData {
    pub kind: WordPressComponentKind,
    pub provider_total: u64,
    pub matched_count: usize,
    pub refreshed_at: Option<String>,
    pub components: Vec<WordPressComponent>,
}

/// Stable state for one target in captured SSH execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MachineSshExecutionState {
    Succeeded,
    Failed,
    TimedOut,
    Cancelled,
    Skipped,
}

/// Stable reason for a per-target captured SSH failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MachineSshFailureCode {
    TargetUnavailable,
    InvalidTarget,
    SpawnFailed,
    WaitFailed,
    CaptureFailed,
    RemoteExit,
    TimedOut,
    Cancelled,
    FailFast,
}

/// Curated, secret-safe failure detail for one SSH target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MachineSshFailure {
    pub code: MachineSshFailureCode,
    pub message: String,
}

/// Encoding used for bounded SSH process output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MachineSshCaptureEncoding {
    Text,
    Base64,
}

/// JSON-safe bounded stdout or stderr from one SSH target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MachineSshCapturedStream {
    pub encoding: MachineSshCaptureEncoding,
    pub data: String,
    pub truncated: bool,
    pub captured_bytes: usize,
}

/// Stable machine result for one target in an SSH fan-out operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MachineSshTargetResult {
    pub environment: EnvironmentRef,
    pub state: MachineSshExecutionState,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub stdout: MachineSshCapturedStream,
    pub stderr: MachineSshCapturedStream,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<MachineSshFailure>,
}

/// Stable `data` payload shared by successful and partially failed captured `ssh run` commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MachineSshRunData {
    pub results: Vec<MachineSshTargetResult>,
    pub stream_capture_limit_bytes: usize,
}

/// Metadata included in every machine envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MachineMeta {
    pub cli_version: String,
}

impl MachineMeta {
    #[must_use]
    pub fn new(cli_version: impl Into<String>) -> Self {
        Self {
            cli_version: cli_version.into(),
        }
    }
}

/// Versioned success envelope emitted by `--output json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MachineSuccess<T> {
    pub schema_version: u32,
    pub ok: bool,
    pub command: String,
    pub data: T,
    pub warnings: Vec<MachineWarning>,
    pub meta: MachineMeta,
}

impl<T> MachineSuccess<T> {
    #[must_use]
    pub fn new(
        command: impl Into<String>,
        data: T,
        warnings: Vec<MachineWarning>,
        cli_version: impl Into<String>,
    ) -> Self {
        Self {
            schema_version: MACHINE_SCHEMA_VERSION,
            ok: true,
            command: command.into(),
            data,
            warnings,
            meta: MachineMeta::new(cli_version),
        }
    }
}

/// Versioned failure envelope emitted by machine mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MachineFailure {
    pub schema_version: u32,
    pub ok: bool,
    pub command: String,
    pub error: AppError,
    pub warnings: Vec<MachineWarning>,
    pub meta: MachineMeta,
}

impl MachineFailure {
    #[must_use]
    pub fn new(
        command: impl Into<String>,
        error: AppError,
        cli_version: impl Into<String>,
    ) -> Self {
        Self {
            schema_version: MACHINE_SCHEMA_VERSION,
            ok: false,
            command: command.into(),
            error,
            warnings: Vec::new(),
            meta: MachineMeta::new(cli_version),
        }
    }
}

/// Versioned failure envelope that retains structured results from completed work.
///
/// This is intended for operations such as captured SSH execution where the command failed as a
/// whole but per-target outcomes are still useful. Pre-execution failures continue to use
/// [`MachineFailure`], which deliberately has no `data` field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MachinePartialFailure<T> {
    pub schema_version: u32,
    pub ok: bool,
    pub command: String,
    pub error: AppError,
    pub data: T,
    pub warnings: Vec<MachineWarning>,
    pub meta: MachineMeta,
}

impl<T> MachinePartialFailure<T> {
    #[must_use]
    pub fn new(
        command: impl Into<String>,
        error: AppError,
        data: T,
        warnings: Vec<MachineWarning>,
        cli_version: impl Into<String>,
    ) -> Self {
        Self {
            schema_version: MACHINE_SCHEMA_VERSION,
            ok: false,
            command: command.into(),
            error,
            data,
            warnings,
            meta: MachineMeta::new(cli_version),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MachineFailure, MachinePartialFailure, MachineSuccess, MachineWarning};
    use crate::{AppError, ErrorCode};
    use serde_json::json;

    #[test]
    fn success_envelope_shape_is_stable() {
        let envelope = MachineSuccess::new(
            "site.list",
            vec![json!({"id": "site_1"})],
            vec![MachineWarning::new("cached", "Using cached inventory")],
            "0.1.0",
        );
        let value = serde_json::to_value(envelope).expect("success envelope serializes");

        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["ok"], true);
        assert_eq!(value["command"], "site.list");
        assert_eq!(value["meta"]["cli_version"], "0.1.0");
    }

    #[test]
    fn failure_envelope_has_no_data_field() {
        let envelope = MachineFailure::new(
            "cli.parse",
            AppError::new(ErrorCode::InvalidArguments, "unknown command"),
            "0.1.0",
        );
        let value = serde_json::to_value(envelope).expect("failure envelope serializes");

        assert_eq!(value["ok"], false);
        assert_eq!(value["error"]["code"], "invalid_arguments");
        assert!(value.get("data").is_none());
    }

    #[test]
    fn partial_failure_adds_data_without_changing_failure_shape() {
        let envelope = MachinePartialFailure::new(
            "ssh.run",
            AppError::new(
                ErrorCode::RemoteExecutionFailed,
                "one or more remote commands failed",
            ),
            vec![json!({"environment_id": "env_1", "exit_code": 7})],
            Vec::new(),
            "0.1.0",
        );
        let value = serde_json::to_value(envelope).expect("partial failure serializes");

        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["ok"], false);
        assert_eq!(value["error"]["code"], "remote_execution_failed");
        assert_eq!(value["data"][0]["exit_code"], 7);

        let ordinary = MachineFailure::new(
            "ssh.run",
            AppError::new(ErrorCode::InvalidInput, "no targets"),
            "0.1.0",
        );
        let ordinary_value = serde_json::to_value(ordinary).expect("ordinary failure serializes");
        assert!(ordinary_value.get("data").is_none());
    }
}
