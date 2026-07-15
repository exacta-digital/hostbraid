use crate::{AppError, MACHINE_SCHEMA_VERSION};
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

#[cfg(test)]
mod tests {
    use super::{MachineFailure, MachineSuccess, MachineWarning};
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
}
