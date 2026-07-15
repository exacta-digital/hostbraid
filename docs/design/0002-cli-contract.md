# 0002: CLI and machine-output contract

Status: accepted, 2026-07-15.

## Human mode

Human mode is the default. Commands may emit styled prose, tables, guides, and progress. Color is
TTY-aware and respects `NO_COLOR`; progress goes to stderr and is disabled when piped.

## Machine mode

`--output json` implies `--no-input` and emits exactly one JSON object on stdout. Logs and progress
remain on stderr. Success has this envelope:

```json
{
  "schema_version": 1,
  "ok": true,
  "command": "search",
  "data": [],
  "warnings": [],
  "meta": { "cli_version": "0.1.0" }
}
```

An ordinary failure omits `data` and includes:

```json
{
  "schema_version": 1,
  "ok": false,
  "command": "cli.parse",
  "error": {
    "code": "invalid_arguments",
    "message": "…",
    "hint": "…"
  },
  "warnings": [],
  "meta": { "cli_version": "0.1.0" }
}
```

Captured SSH execution has one additive exception: when work was attempted and at least one target
did not succeed, the failure envelope retains the ordered per-target report in `data`:

```json
{
  "schema_version": 1,
  "ok": false,
  "command": "ssh.run",
  "error": {
    "code": "remote_execution_failed",
    "message": "one or more remote commands failed",
    "hint": "Inspect each failed target's failure and captured stderr, correct local OpenSSH, provider SSH access, or the remote command, then retry only failed environments with exact --environment-id selectors."
  },
  "data": {
    "results": [
      {
        "environment": {
          "provider": "kinsta",
          "profile": "agency",
          "site_id": "site_opaque",
          "environment_id": "env_opaque"
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
      }
    ],
    "stream_capture_limit_bytes": 1048576
  },
  "warnings": [],
  "meta": { "cli_version": "0.1.0" }
}
```

Pre-execution failures—invalid selectors, missing credentials, provider errors, or declined
policy—continue to omit `data`.

Every top-level CLI failure includes a non-empty, secret-safe `hint`. Error producers provide the
most specific recovery action they know; the CLI presentation boundary supplies a code-aware
fallback for rare errors without one. Parse failures use only the parser's structural error kind
and exact comparisons against allowlisted command and argument identities. Parser context is never
rendered. Diagnostics never reproduce unknown arguments, rejected values, or the raw parser
message, because argv may contain an accidentally pasted credential. Per-target SSH failure objects
keep their existing `code` and `message` shape; the enclosing failure provides the overall
remediation.

Fields may only be removed or have their meaning changed in a new schema version. Additive fields
must not contain secrets. Canonical provider references are structured objects, not display strings.

Provider-backed success data uses these top-level shapes in schema version 1:

| Command | `data` shape |
|---|---|
| `profile.add`, `profile.show`, `profile.default` | Secret-free profile metadata and `is_default` |
| `profile.list` | `default_profile` plus a `profiles` array |
| `profile.remove`, `profile.credential.set` | `profile` plus `credential_cleanup_failed` |
| `site.list` | Array of canonical site summaries |
| `environment.list` | Exact `site` plus its `environments` array |
| `environment.show` | Exact `site`, `environment`, and current `capabilities` |
| `inventory.plugins`, `inventory.themes` | Kind, provider total, matched count, refresh time, and full matching components |
| `ssh.run` | Ordered `results` plus `stream_capture_limit_bytes` |

Profile metadata may name an environment variable but never contains its value. Machine inventory
always includes matching installation detail; `--details` only expands human output.

## Account command facade

The short account commands are a human-friendly facade over the canonical profile operations. They
do not introduce new machine command identities or output shapes:

| Facade | Canonical operation | Behavior |
|---|---|---|
| `login <provider> <name>` | `profile.add` | Creates the profile and always makes it the explicit default |
| `profiles` | `profile.list` | Lists secret-free profile metadata without resolving credentials |
| `use <provider:name>` | `profile.default` | Selects one exact profile as the default |
| `logout <provider:name>` | `profile.remove` | Removes one exact local profile after confirmation |

The full `profile` command tree remains public. Machine clients must interpret the canonical
`command` field rather than infer an operation from the spelling used to invoke it.

Logout is a local HostBraid operation. It removes profile metadata and attempts to remove a
HostBraid-managed keyring credential, reporting the existing `credential_cleanup_failed` warning
when cleanup fails. It does not revoke an API token at the provider and does not unset or delete an
environment-backed token. Non-interactive logout requires the exact profile reference and
`--yes`.

## Profile and selector rules

- Provider profiles are exact `provider:name` references. Provider-backed commands use an explicit
  `--profile` or the configured default; no heuristic default is allowed.
- `login` explicitly makes the newly created profile the default; this is not heuristic selection.
- Exact opaque provider IDs are authoritative. Display names, domains, kinds, and labels are not
  substitutes for IDs.
- Repeated values within an SSH selector category are ORed. Different categories are ANDed.
- Exact environment-ID selections do not require a second confirmation. Site, kind, label, and
  `--all` selections are broad: human mode previews and prompts, while machine mode requires
  explicit `--yes` or fails with `policy_denied`.
- `ssh open` is interactive and unsupported in machine mode.

## SSH execution output

Interactive human-mode execution against one resolved target inherits stdin, stdout, and stderr.
Non-interactive, multi-target, and all machine-mode execution capture and group output by canonical
environment reference.

Captured execution uses OpenSSH batch mode, so it does not open password or host-key confirmation
prompts. Establish host trust and non-interactive authentication first, for example with `ssh open`.

Captured stdout and stderr each have `encoding`, `data`, `truncated`, and `captured_bytes` fields.
Valid UTF-8 uses `encoding: "text"`; other bytes use `encoding: "base64"`. Each stream is capped at
1 MiB for ordinary batches, subject to a fair 64 MiB total raw-capture budget across all targets.
Capture limits bound retained output, not the execution itself. Each result includes `failure` only
when a curated per-target failure is available; successful results omit that field.

Batch scheduling defaults to eight simultaneous OpenSSH children and collecting every result.
`--jobs` also bounds concurrent SSH-coordinate loading; an unavailable target becomes an ordered
per-target failure without suppressing healthy targets. An optional per-target `--timeout` and
`--fail-fast` change scheduling policy. Results remain in deterministic target order. Fail-fast
stops scheduling queued targets after the first failure and represents them as skipped; it does not
erase completed results.

Ctrl-C or a termination signal marks queued targets cancelled and kills and reaps every running
captured SSH process group. Interactive one-target human execution stays in HostBraid's foreground
process group so terminal prompts and the terminal's native signal delivery remain intact.

A timeout terminates HostBraid's local OpenSSH child and closes that connection. It cannot prove
that every remote descendant has stopped; commands that require a hard server-side deadline must
enforce one on the remote host as well.

OpenSSH multiplexing is a best-effort local optimization using `ControlMaster=auto` and
`ControlPersist=60s` in a secure owner-only control directory. `--no-pool` disables it. Pooling does
not change OpenSSH host-key verification, authentication, or user configuration.

## Exit statuses

| Status | Meaning |
|---:|---|
| 0 | Success, including an empty search result |
| 1 | General, not-found, unsupported, remote-execution, I/O, or internal failure |
| 2 | Invalid arguments, invalid input, or ambiguous target |
| 3 | Required local dependency missing |
| 4 | Authentication failed |
| 5 | Provider or capability unavailable |
| 6 | Policy denied the operation |

These statuses describe errors produced by HostBraid and captured execution. Interactive `ssh open`
and interactive one-target human `ssh run` pass through the OpenSSH process status, which may use
other values such as `255`. Signal-cancelled captured batches use conventional status `130` for
interrupt or `143` for termination while retaining their structured partial results.
