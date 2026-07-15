# HostBraid

**Bring every hosting environment within reach.**

HostBraid is a provider-neutral command-line tool for people who work with sites across managed
hosting platforms. It will discover accounts, sites, and environments; open correctly targeted SSH
sessions; retrieve portable artifacts; and orchestrate explicit files-and-database pulls.

It is built in Rust, designed for humans and automation, and developed as an open-source project by
[It's Ed](https://itsed.se).

> **Project status:** first Kinsta vertical slice. The current development build can configure
> secure Kinsta profiles, discover sites and environments, delegate interactive and one-shot work
> to OpenSSH, and inspect Kinsta's read-only plugin/theme inventory. Export and pull workflows are
> still validation work; EdHosting remains the second adapter used to prove the abstraction.

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

## Try HostBraid

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
hb login <provider> <name>              Add an account and make it the default
hb profiles                             List configured provider accounts
hb use <provider:name>                  Select the exact default account
hb logout <provider:name>               Remove an exact account locally
hostbraid profile add|list|show        Manage secret-free provider profiles
hostbraid profile default|remove       Select or remove an exact profile
hostbraid profile credential set       Replace and validate a credential source
hostbraid site list                    List sites for a provider profile
hostbraid environment list|show        List or inspect exact environments
hostbraid ssh open|run                 Open a shell or run one remote command
hostbraid inventory plugins|themes     Inspect read-only WordPress inventory
hostbraid guide [topic]                Read a built-in workflow guide
hostbraid search <query>               Search commands and guides
hostbraid doctor                       Check local workflow dependencies
hostbraid completion <shell>           Generate shell completion code
```

The short account commands are an ergonomic facade over the canonical `profile` commands. The
canonical commands remain available, and JSON output keeps their stable command identities:
`login` emits `profile.add`, `profiles` emits `profile.list`, `use` emits `profile.default`, and
`logout` emits `profile.remove`.

`doctor` is a report: unavailable tools appear as booleans and warnings but do not make the command
itself fail when the report was produced successfully.

Clap provides contextual `--help`, aliases, environment-backed global options, and shell
completions. Parse failures deliberately do not echo unrecognized values, because an accidentally
pasted credential must not reappear in diagnostics. Automatic human output uses color and transient
progress only on terminals; an explicit `--color always` overrides color detection. JSON mode never
emits a spinner or prompt.

## Kinsta quick start

Log in interactively to save a validated token in the operating-system credential store. `login`
creates the profile and always makes it the explicit default:

```bash
hb login kinsta agency
hb profiles
hb site list
hb environment list --site-id SITE_ID
hb environment show --environment-id ENVIRONMENT_ID
```

For CI, name the environment variable that HostBraid should resolve on each use. The token itself is
never passed in argv or written to the profile configuration:

```bash
hb login kinsta ci --credential-env KINSTA_TOKEN
hb site list --profile kinsta:ci
```

Switch the default with an exact reference, or remove a local profile with confirmation:

```bash
hb use kinsta:agency
hb logout kinsta:ci
```

`logout` removes HostBraid's local profile metadata and, when applicable, its HostBraid-managed
keyring entry. It does not revoke the API token at the provider, and it does not unset or delete an
environment-backed token. Revoke provider credentials in the provider's supported control plane
when that is required.

Open one exact environment or run one command on it:

```bash
hostbraid ssh open --environment-id ENVIRONMENT_ID
hostbraid ssh run --environment-id ENVIRONMENT_ID -- uptime
```

Selectors based on a site, kind, label, or `--all` can expand to several environments. HostBraid
previews these broad selections and requires confirmation; pass `--yes` only after reviewing the
scope. Repeated values within one selector category are ORed, while different categories are ANDed:

```bash
hostbraid ssh run --kind production --label customer-a --yes -- wp core version
hostbraid ssh run --site-id SITE_ID --jobs 4 --timeout 2m --yes -- uptime
```

Batch execution collects an ordered result for every target by default. `--jobs` bounds both SSH
coordinate loading and simultaneous OpenSSH children; `--fail-fast` stops queued remote work after
an unsuccessful target. A target whose SSH access is unavailable does not prevent other selected
targets from running unless fail-fast was requested. Ctrl-C cancels queued work and kills and reaps
the captured SSH process groups. HostBraid asks OpenSSH to reuse connections for 60 seconds when a
secure local control-socket directory is available; `--no-pool` disables this.

Kinsta's company-wide WordPress inventory is read-only:

```bash
hostbraid inventory plugins --updates --details
hostbraid inventory themes --vulnerable --search twenty
```

## Agent-friendly by contract

HostBraid is a CLI only. Agents use the same executable and public contract as people:

```bash
hostbraid --output json --no-input <command>
```

- stdout contains exactly one versioned JSON value.
- stderr is reserved for diagnostics and progress.
- `--output json` implies non-interactive behavior.
- Error codes and process exit statuses are stable.
- Ambiguous or missing exact selectors fail rather than guessing.
- Exact provider IDs—not domains or display names—are authoritative.
- Secrets, signed download URLs, and raw provider responses are excluded from ordinary output.
- Captured SSH failures return `ok: false`, `remote_execution_failed`, and structured `data` for
  the per-target results that completed or were skipped.

Run `hostbraid guide agents` for the embedded operating instructions. The JSON contract is
documented in [docs/design/0002-cli-contract.md](docs/design/0002-cli-contract.md).

## First vertical slice

The implemented Kinsta slice is intentionally narrow: secure profiles, site/environment catalog,
environment capability inspection, structured SSH target discovery, OpenSSH shell and one-shot
execution, bounded fan-out, and read-only company plugin/theme inventory. It uses documented Kinsta
API endpoints and never automates the hosting dashboard.

Existing portable exports and a safe browser-free files-and-database pull remain separate future
work. The same useful capabilities must then be exercised against EdHosting before the shared
provider abstraction is treated as proven.

See [docs/roadmap.md](docs/roadmap.md) and
[docs/validation-spikes.md](docs/validation-spikes.md).

## Architecture

```text
Clap CLI
   │
application workflows + policy
   ├── provider capabilities: catalog / SSH access / WordPress inventory / artifacts
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
