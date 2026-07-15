# AGENTS.md

This file is the repository contract for coding agents. End-user automation instructions are built
into `hostbraid guide agents`.

## Product boundary

- HostBraid is a CLI only. Do not add MCP servers, HTTP agent endpoints, a daemon, or a TUI.
- HostBraid manages the hosting layer; it does not reimplement WP-CLI, Git, Composer, DDEV, SSH, or
  rsync.
- Provider neutrality is proved by working vertical slices, not by a large trait full of optional
  methods. Keep capability traits focused.
- Keep `Snapshot`, `Export`, and `Pull` distinct.
- Do not automate private hosting-dashboard endpoints or browser UI.

## Safety

- Never put API tokens, SSH passwords, signed artifact URLs, database contents, or raw provider
  responses in logs, examples, fixtures, errors, or normal output.
- Never accept secrets through ordinary command arguments. Prefer the OS credential store, a
  credential helper, environment references for CI, or stdin.
- Construct child processes with argument arrays. Do not interpolate provider or user values into
  shell strings.
- Preserve OpenSSH host-key verification. Never add `StrictHostKeyChecking=no`.
- Exact opaque IDs are authoritative. Non-interactive commands must reject ambiguous selectors.
- Treat remote SSH and arbitrary WP-CLI passthrough as arbitrary-code actions, not read operations.

## CLI contract

- Human output is the default. It may use color, guides, tables, and progress on a TTY.
- `--output json` emits exactly one JSON value to stdout and implies `--no-input`.
- Logs and progress belong on stderr. Machine mode has no color, spinner, or prompt.
- Keep `schema_version`, error codes, exit statuses, field meanings, and canonical references stable.
- Changing a public JSON shape requires updating documentation and integration tests in the same
  change.
- Search is generated from the Clap command tree plus embedded guides so it cannot silently drift.
- Every command and argument needs useful Clap help; include examples when behavior is non-obvious.

## Rust boundaries

- `hostbraid-core` has no Clap, HTTP, terminal, keyring, or provider-specific dependencies.
- Provider adapters translate wire DTOs at their boundary and return core types.
- Libraries use typed `AppError` values. Keep errors secret-safe and actionable.
- Avoid a runtime plugin protocol until built-in Kinsta and EdHosting adapters prove the contract.
- Avoid SQLite until durable jobs, resumable state, or indexed cache requirements are demonstrated.

## Local handoff

Run:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
```

Or run `just check`.
