Human output is HostBraid's default. It favors concise explanations, clear next actions, and safe
defaults:

- Automatic colors appear only on a terminal and respect `NO_COLOR`; an explicit `--color always`
  overrides automatic detection.
- Progress is transient, goes to stderr, and disappears in pipes or JSON mode.
- Commands explain their arguments through contextual `--help`.
- Parse failures omit unrecognized argument values so accidentally pasted secrets are not echoed.
- `hostbraid search <word>` searches commands and the full guide text.
- `hostbraid completion <shell>` integrates discovery into your shell.

Provider profiles are selected exactly as `provider:name`. Set one explicit default with
`hostbraid profile default kinsta:agency`, or pass `--profile kinsta:agency` to a provider-backed
command. HostBraid does not infer a default merely because only one profile exists.

HostBraid uses the system OpenSSH client rather than asking you to maintain a second set of SSH keys
and host fingerprints. Unknown or changed host keys remain visible decisions; they are never
silently accepted. `ssh open` and a one-target human `ssh run` inherit your terminal streams, so
normal OpenSSH authentication and command behavior remain visible.

```bash
hostbraid ssh open --environment-id ENVIRONMENT_ID
hostbraid ssh run --environment-id ENVIRONMENT_ID -- wp core version
```

Exact environment IDs may be repeated without an extra confirmation. A site ID, environment kind,
site label, or `--all` can expand into a wider target set, so HostBraid prints a deterministic
preview and asks you to confirm it. `--yes` is the explicit non-prompting acknowledgement.

Batch commands run up to eight OpenSSH children by default, continue collecting results after a
failure, and group captured output by environment. Use `--jobs`, `--timeout`, or `--fail-fast` to
change those controls. Captured batches use OpenSSH batch mode and therefore cannot stop for a
password or host-key confirmation; establish trust and non-interactive authentication with an
exact `ssh open` first. Ctrl-C cancels queued targets and stops running captured SSH process groups.
Connections are eligible for 60-second OpenSSH multiplexing when HostBraid can create a secure
owner-only control directory. Pooling is best effort and can be disabled with `--no-pool`; host-key
verification and the user's OpenSSH configuration are still preserved.

Plugin and theme inventory commands are read-only. `--updates`, `--vulnerable`, and `--search`
filter the company inventory, while `--details` expands per-environment installations in human
output.

Use `--quiet` to suppress transient progress. Use `--color never` when recording human output.

`hostbraid doctor` is diagnostic. Missing tools are reported as readiness booleans and warnings; a
success status means the report was produced, not that every optional workflow is available.
