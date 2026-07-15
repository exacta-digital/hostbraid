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

Failure omits `data` and includes:

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

Fields may only be removed or have their meaning changed in a new schema version. Additive fields
must not contain secrets. Canonical provider references are structured objects, not display strings.

## Exit statuses

| Status | Meaning |
|---:|---|
| 0 | Success, including an empty search result |
| 1 | General, not-found, unsupported, I/O, or internal failure |
| 2 | Invalid arguments, invalid input, or ambiguous target |
| 3 | Required local dependency missing |
| 4 | Authentication failed |
| 5 | Provider or capability unavailable |
| 6 | Policy denied the operation |
