HostBraid works at the hosting layer: provider profiles, sites, environments, access coordinates,
portable artifacts, and local pulls. It delegates WordPress commands to WP-CLI and connections to
the system OpenSSH client.

Start by checking the local tools HostBraid will use:

```bash
hostbraid doctor
```

Explore without leaving the terminal:

```bash
hostbraid guide --list
hostbraid search ssh
hostbraid search export
hostbraid completion --help
```

Configure a Kinsta profile. With a terminal attached, HostBraid reads the token without echoing it,
validates it with Kinsta, and saves it in the operating-system credential store. The profile file
contains only secret-free metadata.

```bash
hostbraid profile add kinsta agency --default
hostbraid profile list
hostbraid profile show kinsta:agency
```

For non-interactive use, either pipe a token through stdin or configure a named environment source.
Never put the token itself in a command argument.

```bash
printf '%s\n' "$KINSTA_TOKEN" | hostbraid profile add kinsta agency-stdin --token-stdin
hostbraid profile add kinsta ci --credential-env KINSTA_TOKEN
```

An explicit `--profile provider:name` selects one account. If it is omitted, HostBraid uses only the
profile selected with `profile default`; it does not guess from the profiles on disk.

```bash
hostbraid site list
hostbraid site list --profile kinsta:ci
hostbraid environment list --site-id SITE_ID
hostbraid environment show --environment-id ENVIRONMENT_ID
```

Copy opaque IDs from catalog output. They are authoritative even when two resources have similar
display names or domains.

Open a shell or run a command on one exact environment:

```bash
hostbraid ssh open --environment-id ENVIRONMENT_ID
hostbraid ssh run --environment-id ENVIRONMENT_ID -- uptime
```

`ssh run` also supports repeated `--environment-id`, `--site-id`, `--kind`, and `--label`
selectors, plus deliberate `--all`. Site, kind, label, and all-environment selections are broad, so
HostBraid previews their targets and asks for confirmation. Add `--yes` only after reviewing that
scope. Repeated values in one category are ORed; categories are ANDed.

```bash
hostbraid ssh run --kind production --label customer-a --yes -- wp core version
hostbraid ssh run --site-id SITE_ID --jobs 4 --timeout 90s --yes -- uptime
```

Inspect Kinsta's read-only company inventory with optional filters:

```bash
hostbraid inventory plugins --updates --details
hostbraid inventory themes --vulnerable --search twenty
```

For automation, switch to the versioned machine contract:

```bash
hostbraid --output json --no-input doctor
hostbraid --output json --no-input site list --profile kinsta:ci
```

Machine mode cannot answer a broad-selection prompt. Pass `--yes` explicitly or use only exact
environment IDs. `ssh open` is interactive and therefore unavailable in JSON mode; use `ssh run`
for automation.
