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

Log in to Kinsta. With a terminal attached, HostBraid reads the token without echoing it, validates
it with Kinsta, and saves it in the operating-system credential store. The profile file contains
only secret-free metadata. `login` creates the profile and always selects it as the explicit
default.

```bash
hb login kinsta agency
hb profiles
```

For non-interactive use, either pipe a token through stdin or configure a named environment source.
Never put the token itself in a command argument.

```bash
printf '%s\n' "$KINSTA_TOKEN" | hb login kinsta agency-stdin --token-stdin
hb login kinsta ci --credential-env KINSTA_TOKEN
```

Use an exact `provider:name` reference to change the default. An explicit `--profile provider:name`
selects one account for a single command. If it is omitted, HostBraid uses only the configured
default; it does not guess from the profiles on disk.

```bash
hb use kinsta:agency
hb site list
hb site list --profile kinsta:ci
hb environment list --site-id SITE_ID
hb environment show --environment-id ENVIRONMENT_ID
```

Remove an exact local profile with `hb logout provider:name`. HostBraid asks for confirmation; use
`--yes` only when the exact reference has already been reviewed. Logout removes the local profile
metadata and any HostBraid-managed keyring credential, but it does not revoke the API token at the
provider or remove an environment-backed token.

```bash
hb logout kinsta:ci
```

The canonical `hostbraid profile add|list|show|default|remove` and
`hostbraid profile credential set` commands remain available. The short facade keeps the canonical
machine identities: `login` is `profile.add`, `profiles` is `profile.list`, `use` is
`profile.default`, and `logout` is `profile.remove` in JSON envelopes.

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
