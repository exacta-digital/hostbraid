use hostbraid_core::{
    AppError, Capability, EnvironmentKind, EnvironmentRef, EnvironmentSummary, ErrorCode,
    MachineEnvironmentListData, MachineEnvironmentShowData, MachineInventoryData,
    MachinePartialFailure, MachineSshCaptureEncoding, MachineSshCapturedStream,
    MachineSshExecutionState, MachineSshFailure, MachineSshFailureCode, MachineSshRunData,
    MachineSshTargetResult, MachineSuccess, OpaqueId, SiteLabel, SiteRef, SiteSummary,
    WordPressComponent, WordPressComponentInstallation, WordPressComponentKind,
};
use serde_json::{Value, json};

fn site() -> SiteSummary {
    SiteSummary {
        reference: SiteRef::try_new("kinsta", "agency", "site_1").expect("site reference"),
        display_name: "Example".to_owned(),
        primary_domain: Some("example.test".to_owned()),
        labels: vec![SiteLabel {
            id: OpaqueId::new("label_1").expect("label ID"),
            name: "customer-a".to_owned(),
        }],
    }
}

fn environment() -> EnvironmentSummary {
    EnvironmentSummary {
        reference: EnvironmentRef::try_new("kinsta", "agency", "site_1", "env_1")
            .expect("environment reference"),
        display_name: "Production".to_owned(),
        kind: EnvironmentKind::Production,
        provider_kind: Some("live".to_owned()),
        primary_domain: Some("example.test".to_owned()),
    }
}

fn success_envelope(command: &str, data: Value) -> Value {
    json!({
        "schema_version": 1,
        "ok": true,
        "command": command,
        "data": data,
        "warnings": [],
        "meta": { "cli_version": "0.1.0" }
    })
}

#[test]
fn provider_success_payloads_keep_their_documented_schema_v1_shapes() {
    let site_json = json!({
        "reference": {
            "provider": "kinsta",
            "profile": "agency",
            "site_id": "site_1"
        },
        "display_name": "Example",
        "primary_domain": "example.test",
        "labels": [{ "id": "label_1", "name": "customer-a" }]
    });
    let environment_json = json!({
        "reference": {
            "provider": "kinsta",
            "profile": "agency",
            "site_id": "site_1",
            "environment_id": "env_1"
        },
        "display_name": "Production",
        "kind": "production",
        "provider_kind": "live",
        "primary_domain": "example.test"
    });

    let sites = MachineSuccess::new("site.list", vec![site()], Vec::new(), "0.1.0");
    assert_eq!(
        serde_json::to_value(sites).expect("site list serializes"),
        success_envelope("site.list", json!([site_json.clone()]))
    );

    let environments = MachineSuccess::new(
        "environment.list",
        MachineEnvironmentListData {
            site: site(),
            environments: vec![environment()],
        },
        Vec::new(),
        "0.1.0",
    );
    assert_eq!(
        serde_json::to_value(environments).expect("environment list serializes"),
        success_envelope(
            "environment.list",
            json!({ "site": site_json.clone(), "environments": [environment_json.clone()] })
        )
    );

    let shown = MachineSuccess::new(
        "environment.show",
        MachineEnvironmentShowData {
            site: site(),
            environment: environment(),
            capabilities: vec![Capability::available("connection.ssh", 1)],
        },
        Vec::new(),
        "0.1.0",
    );
    assert_eq!(
        serde_json::to_value(shown).expect("environment show serializes"),
        success_envelope(
            "environment.show",
            json!({
                "site": site_json,
                "environment": environment_json.clone(),
                "capabilities": [{
                    "name": "connection.ssh",
                    "version": 1,
                    "supported": true,
                    "available": true,
                    "reason": null,
                    "remediation": null
                }]
            })
        )
    );

    let inventory = MachineSuccess::new(
        "inventory.plugins",
        MachineInventoryData {
            kind: WordPressComponentKind::Plugin,
            provider_total: 7,
            matched_count: 1,
            refreshed_at: Some("2026-07-15T10:00:00Z".to_owned()),
            components: vec![WordPressComponent {
                slug: "sample-plugin".to_owned(),
                title: "Sample Plugin".to_owned(),
                description: Some("A sample".to_owned()),
                latest_version: Some("2.0.0".to_owned()),
                latest_version_vulnerable: false,
                environment_count: 1,
                update_count: 1,
                installations: vec![WordPressComponentInstallation {
                    environment: environment().reference,
                    status: "active".to_owned(),
                    installed_version: "1.0.0".to_owned(),
                    installed_version_vulnerable: true,
                    update_state: Some("available".to_owned()),
                    available_version: Some("2.0.0".to_owned()),
                    available_version_vulnerable: false,
                    update_status: Some("pending".to_owned()),
                    auto_update_type: Some("disabled".to_owned()),
                }],
            }],
        },
        Vec::new(),
        "0.1.0",
    );
    assert_eq!(
        serde_json::to_value(inventory).expect("inventory serializes"),
        success_envelope(
            "inventory.plugins",
            json!({
                "kind": "plugin",
                "provider_total": 7,
                "matched_count": 1,
                "refreshed_at": "2026-07-15T10:00:00Z",
                "components": [{
                    "slug": "sample-plugin",
                    "title": "Sample Plugin",
                    "description": "A sample",
                    "latest_version": "2.0.0",
                    "latest_version_vulnerable": false,
                    "environment_count": 1,
                    "update_count": 1,
                    "installations": [{
                        "environment": environment_json["reference"].clone(),
                        "status": "active",
                        "installed_version": "1.0.0",
                        "installed_version_vulnerable": true,
                        "update_state": "available",
                        "available_version": "2.0.0",
                        "available_version_vulnerable": false,
                        "update_status": "pending",
                        "auto_update_type": "disabled"
                    }]
                }]
            })
        )
    );
}

fn captured(
    encoding: MachineSshCaptureEncoding,
    data: &str,
    captured_bytes: usize,
) -> MachineSshCapturedStream {
    MachineSshCapturedStream {
        encoding,
        data: data.to_owned(),
        truncated: false,
        captured_bytes,
    }
}

#[test]
fn ssh_success_uses_the_public_schema_v1_run_payload() {
    let data = MachineSshRunData {
        results: vec![MachineSshTargetResult {
            environment: environment().reference,
            state: MachineSshExecutionState::Succeeded,
            exit_code: Some(0),
            duration_ms: 42,
            stdout: captured(MachineSshCaptureEncoding::Text, "ok\n", 3),
            stderr: captured(MachineSshCaptureEncoding::Base64, "AP8=", 2),
            failure: None,
        }],
        stream_capture_limit_bytes: 1_048_576,
    };
    let envelope = MachineSuccess::new("ssh.run", data, Vec::new(), "0.1.0");

    assert_eq!(
        serde_json::to_value(envelope).expect("SSH success serializes"),
        success_envelope(
            "ssh.run",
            json!({
                "results": [{
                    "environment": {
                        "provider": "kinsta",
                        "profile": "agency",
                        "site_id": "site_1",
                        "environment_id": "env_1"
                    },
                    "state": "succeeded",
                    "exit_code": 0,
                    "duration_ms": 42,
                    "stdout": {
                        "encoding": "text",
                        "data": "ok\n",
                        "truncated": false,
                        "captured_bytes": 3
                    },
                    "stderr": {
                        "encoding": "base64",
                        "data": "AP8=",
                        "truncated": false,
                        "captured_bytes": 2
                    }
                }],
                "stream_capture_limit_bytes": 1_048_576
            })
        )
    );
}

#[test]
fn ssh_partial_failure_retains_the_same_public_schema_v1_run_payload() {
    let data = MachineSshRunData {
        results: vec![MachineSshTargetResult {
            environment: environment().reference,
            state: MachineSshExecutionState::Failed,
            exit_code: Some(7),
            duration_ms: 120,
            stdout: captured(MachineSshCaptureEncoding::Text, "", 0),
            stderr: captured(MachineSshCaptureEncoding::Text, "command failed\n", 15),
            failure: Some(MachineSshFailure {
                code: MachineSshFailureCode::RemoteExit,
                message: "the remote command exited unsuccessfully".to_owned(),
            }),
        }],
        stream_capture_limit_bytes: 1_048_576,
    };
    let envelope = MachinePartialFailure::new(
        "ssh.run",
        AppError::new(
            ErrorCode::RemoteExecutionFailed,
            "one or more remote commands failed",
        ),
        data,
        Vec::new(),
        "0.1.0",
    );

    assert_eq!(
        serde_json::to_value(envelope).expect("SSH partial failure serializes"),
        json!({
            "schema_version": 1,
            "ok": false,
            "command": "ssh.run",
            "error": {
                "code": "remote_execution_failed",
                "message": "one or more remote commands failed"
            },
            "data": {
                "results": [{
                    "environment": {
                        "provider": "kinsta",
                        "profile": "agency",
                        "site_id": "site_1",
                        "environment_id": "env_1"
                    },
                    "state": "failed",
                    "exit_code": 7,
                    "duration_ms": 120,
                    "stdout": {
                        "encoding": "text",
                        "data": "",
                        "truncated": false,
                        "captured_bytes": 0
                    },
                    "stderr": {
                        "encoding": "text",
                        "data": "command failed\n",
                        "truncated": false,
                        "captured_bytes": 15
                    },
                    "failure": {
                        "code": "remote_exit",
                        "message": "the remote command exited unsuccessfully"
                    }
                }],
                "stream_capture_limit_bytes": 1_048_576
            },
            "warnings": [],
            "meta": { "cli_version": "0.1.0" }
        })
    );
}
