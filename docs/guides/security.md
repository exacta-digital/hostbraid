HostBraid separates credentials, provider control-plane access, SSH data-plane access, and local
artifact writes.

- API tokens belong in the operating-system credential store. Profile configuration contains only
  secret-free provider/company metadata and the credential source.
- CI may supply an explicitly named environment credential or stdin; tokens never belong in argv.
  Environment-backed profiles store the variable name, not its value.
- New and replacement credentials are validated before their profile metadata is committed. A
  replacement credential must resolve to the same provider company; otherwise create a new profile.
- SSH uses existing OpenSSH keys, agents, configuration, jump hosts, and `known_hosts` behavior.
- Provider-returned SSH command strings are not executed. Host, port, and user are consumed as typed
  values and passed as separate process arguments; unsupported working-directory changes fail
  explicitly.
- SSH pooling uses OpenSSH `ControlMaster=auto` with `ControlPersist=60s` and an owner-only local
  control-socket directory. If that directory cannot be created safely, HostBraid warns and runs
  without pooling. It never weakens host-key checking.
- Remote command values are appended to an OpenSSH argument array, not interpolated into a local
  shell. The remote SSH server still interprets commands according to its normal remote-shell
  semantics; treat every `ssh run` as arbitrary code execution.
- Batch stdout and stderr capture is bounded. Each stream is capped at 1 MiB for ordinary batches,
  with a fair 64 MiB total raw-capture budget across all targets; non-UTF-8 data is base64-encoded
  and truncation is explicit.
- Ctrl-C and termination signals cancel queued batch work and kill and reap each running captured
  SSH process group before HostBraid exits. Interactive one-target SSH remains in the foreground so
  OpenSSH prompts and terminal signal handling continue to work normally.
- `--timeout` closes the local SSH process and connection but cannot guarantee that detached remote
  descendants stopped. Enforce critical deadlines on the remote host too.
- Signed export URLs are secret-bearing and stay inside the downloader.
- New artifact destinations use restrictive permissions and staging files.
- Archive extraction must prevent traversal, symlink escape, overwrite, decompression bombs, and
  unbounded disk use before it is enabled.

Machine mode never prompts. If an operation needs human confirmation or additional authority, it
returns a typed error instead of guessing. Broad SSH selectors require `--yes`; using only exact
environment IDs avoids the broad-selection confirmation but does not make the remote command a
read-only action.
