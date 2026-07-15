# Pre-provider validation spikes

These experiments happen before HostBraid promises a production workflow.

## Kinsta catalog and SSH

Using a real restricted account, validate pagination, role behavior, disabled SSH, IP allowlists,
structured host/port/user fields, key-only authentication, custom SSH config, and changed host keys.

Pass condition: an environment can be selected by exact IDs and opened through OpenSSH without
copying dashboard values or retrieving an SSH password.

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

Pass condition: HostBraid adds target discovery without changing command semantics.

## Two-provider contract

Exercise catalog, SSH, and artifacts against Kinsta and EdHosting fixtures before freezing shared
types. Pass only when core behavior contains no provider-name branching or pervasive unsupported
placeholders.

## Agent contract

Have an agent discover commands through help/search and inspect fixtures using only JSON and process
statuses. Test ambiguity, huge labels, malicious text, timeouts, and missing credentials.

Pass condition: no human prose parsing, prompting, guessing, or secret exposure is required.
