# Provider validation spikes

The Kinsta adapter and its first CLI workflows are implemented against documented APIs and fixture
tests. These opt-in real-account experiments must still pass before HostBraid promises a production
workflow. Use a restricted test company, never production credentials or customer data in fixtures
or logs.

The ignored read-only harness covers validation, catalog, capability discovery, and company
inventory without running SSH commands or mutations:

```bash
read -rsp 'Restricted Kinsta test token: ' HOSTBRAID_KINSTA_TEST_TOKEN
export HOSTBRAID_KINSTA_TEST_TOKEN
just test-kinsta-live
unset HOSTBRAID_KINSTA_TEST_TOKEN
```

Supply the token only through that named environment variable. Normal `just check` never runs the
ignored live test.

## Kinsta catalog and SSH

Implemented: API-key validation, company-bound profiles, site/environment catalog mapping, exact
resource resolution, capability inspection, structured SSH coordinates, interactive OpenSSH
delegation, and one-shot command execution. Provider response DTOs remain private and public errors
exclude raw responses.

Using a real restricted account, validate role behavior, large catalogs, disabled SSH, IP allowlists,
structured host/port/user fields, key-only authentication, custom SSH config, jump hosts, and
changed host keys. Exercise both an OS-keyring profile and a named-environment profile.

Pass condition: an environment can be selected by exact IDs and opened through OpenSSH without
copying dashboard values or retrieving an SSH password.

## SSH fan-out and pooling

Implemented: repeated exact environment IDs, broad site/kind/label/all selectors, deterministic
preview and confirmation, default concurrency of eight, optional timeout and fail-fast, ordered
per-target results, bounded capture, non-UTF-8 base64 output, and best-effort OpenSSH multiplexing
with a 60-second persistence window.

On disposable test environments, run the same harmless command across small and large target sets.
Measure connection reuse, concurrency bounds, timeout cleanup, cancellation, fail-fast skips, mixed
remote exit codes, unavailable hosts, output truncation, and the fair 64 MiB total capture budget.
Confirm that Ctrl-C reaps running process groups, `--no-pool` creates independent connections, and
pooling never changes unknown/changed-host-key behavior.

Pass condition: every selected target has one ordered result, no more than `--jobs` children run at
once, failures retain safe structured results, and no local child or control socket outlives its
documented behavior.

## Kinsta WordPress inventory

Implemented: read-only company plugin and theme inventory, catalog joins by exact environment ID,
pagination, and local search/update/vulnerability filters. No update action is exposed.

With a restricted real account, compare company totals and representative installations with the
Kinsta dashboard. Validate empty inventories, inaccessible environments, duplicate component slugs,
components installed at different versions, vulnerability flags, update availability, pagination,
and environments removed between the catalog and inventory requests.

Pass condition: every installation is tied to the correct canonical environment or fails safely;
filters do not silently drop unmatched pages; and output contains no raw provider payloads or
credentials.

## Existing downloadable export

Validate listing, expiry, redirects, filenames, sizes, interruption, resume, and redaction. Confirm
that no documented public endpoint can request creation before encoding that capability.

Pass condition: an already-created export can be downloaded without exposing its signed URL.

## Browser-free pull

Compare a provider export with SSH database export plus rsync/SFTP/tar. Measure consistency,
production load, large-file behavior, interrupted transfers, remote disk requirements, cleanup, and
destination safety.

Pass condition: a representative site can be acquired safely and repeatably with an honest manifest.
If this fails without dashboard automation, do not market HostBraid as a pull workflow.

## Remote WP-CLI

Compare WP-CLI's native remote aliases with `ssh … wp`. Validate quoting, JSON, exit statuses,
Bedrock paths, multisite, and arbitrary commands.

The current `ssh run` deliberately exposes generic remote-command passthrough rather than a curated
WP-CLI command tree. Pass condition: HostBraid adds target discovery without changing command
semantics or misclassifying passthrough as a read operation.

## Two-provider contract

Exercise catalog, SSH, and artifacts against Kinsta and EdHosting fixtures before freezing shared
types. Pass only when core behavior contains no provider-name branching or pervasive unsupported
placeholders.

## Agent contract

Have an agent discover commands through help/search and inspect fixtures using only JSON and process
statuses. Test ambiguity, huge labels, malicious text, timeouts, missing credentials, broad selectors
without `--yes`, mixed-result batches, binary output, and capture truncation.

Pass condition: no human prose parsing, prompting, guessing, or secret exposure is required, and a
partial `ssh.run` failure returns `remote_execution_failed` plus ordered result `data`.
