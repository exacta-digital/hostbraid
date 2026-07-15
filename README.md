# HostBraid

**Bring every hosting environment within reach.**

HostBraid is a provider-neutral command-line tool for people who work with sites across managed
hosting platforms. It will discover accounts, sites, and environments; open correctly targeted SSH
sessions; retrieve portable artifacts; and orchestrate explicit files-and-database pulls.

It is built in Rust, designed for humans and automation, and developed as an open-source project by
[It's Ed](https://itsed.se).

> **Project status:** foundational walking skeleton. The CLI contract, provider-neutral core,
> guides, search, diagnostics, and completions work today. No provider credentials or production
> API calls are accepted yet. Kinsta is the first integration target; EdHosting is the second
> adapter used to prove the abstraction.

## Why HostBraid?

WP-CLI is excellent at operating *inside* WordPress. HostBraid operates one layer above it:

```text
hosting account → site → environment → SSH / export / pull → local workspace
```

HostBraid delegates rather than replaces:

- WordPress operations go to WP-CLI.
- Terminal access goes to the system OpenSSH client.
- File transfer prefers existing provider exports, then rsync/SFTP/tar strategies.
- Local development remains the job of tools such as DDEV.

## Try the walking skeleton

Rust 1.85 or newer is required.

```bash
cd HostBraid
cargo run

cargo run -- guide getting-started
cargo run -- doctor
cargo run -- search ssh
cargo run -- --output json --no-input search environment
```

Install the local build as both `hostbraid` and its short alias, `hb`:

```bash
just install
hostbraid --version
hb --version
hostbraid completion zsh > ~/.zfunc/_hostbraid
```

Without [`just`](https://github.com/casey/just), run
`cargo install --path crates/hostbraid-cli --locked --bins`. `just check` runs the complete local
handoff suite.

## Current commands

```text
hostbraid                              Show a friendly starting point
hostbraid guide [topic]                Read a built-in workflow guide
hostbraid guide --list                 List every guide
hostbraid search <query>               Search commands and guides
hostbraid doctor                       Check SSH, transfer, and WP-CLI tools
hostbraid completion <shell>           Generate shell completion code
```

`doctor` is a report: unavailable tools appear as booleans and warnings but do not make the command
itself fail when the report was produced successfully.

Clap provides contextual `--help`, typo suggestions, aliases, environment-backed global options,
and shell completions. Automatic human output uses color and transient progress only on terminals;
an explicit `--color always` overrides color detection. JSON mode never emits a spinner or prompt.

## Agent-friendly by contract

HostBraid is a CLI only. Agents use the same executable and public contract as people:

```bash
hostbraid --output json --no-input <command>
```

- stdout contains exactly one versioned JSON value.
- stderr is reserved for diagnostics and progress.
- `--output json` implies non-interactive behavior.
- Error codes and process exit statuses are stable.
- Ambiguous resource names will fail rather than guessing.
- Exact provider IDs—not domains or display names—are authoritative.
- Secrets, signed download URLs, and raw provider responses are excluded from ordinary output.

Run `hostbraid guide agents` for the embedded operating instructions. The JSON contract is
documented in [docs/design/0002-cli-contract.md](docs/design/0002-cli-contract.md).

## Planned first vertical slice

The first provider-backed milestone is intentionally narrow:

1. Configure a Kinsta profile without placing its token in argv or TOML.
2. List sites and environments in human or JSON form.
3. Inspect exact environment metadata and capabilities.
4. Open SSH through the user's existing OpenSSH configuration and keys.
5. List/download an existing portable export.
6. Prove a safe, browser-free files-and-database pull.
7. Exercise the same contract against EdHosting before freezing the abstraction.

See [docs/roadmap.md](docs/roadmap.md) and
[docs/validation-spikes.md](docs/validation-spikes.md).

## Architecture

```text
Clap CLI
   │
application workflows + policy
   ├── provider capabilities: catalog / SSH access / artifacts
   ├── Kinsta adapter
   ├── EdHosting adapter
   └── transport: OpenSSH / rsync / SFTP / tar
```

Provider wire types stay inside their adapter. The shared core contains opaque resource references,
capabilities, action classifications, and versioned machine-output contracts. Runtime-loaded plugin
systems and persistent databases are deliberately deferred until a demonstrated need exists.

## Product vocabulary

- **Snapshot:** provider-internal restore point; not necessarily downloadable.
- **Export:** portable files/database artifact.
- **Pull:** HostBraid's local acquisition workflow, which may use an export or SSH transfer.

These terms are intentionally not merged into a generic “backup” abstraction.

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) and [AGENTS.md](AGENTS.md). Security-sensitive reports belong
through the private process in [SECURITY.md](SECURITY.md).

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.

HostBraid is independent software and is not affiliated with Kinsta, Automattic, or the WordPress
Foundation. WordPress and WP-CLI are used referentially.
