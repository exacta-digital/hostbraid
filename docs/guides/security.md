HostBraid separates credentials, provider control-plane access, SSH data-plane access, and local
artifact writes.

- API tokens belong in the operating-system credential store or an external credential helper.
- CI may supply an explicitly named environment credential or stdin; tokens never belong in argv.
- SSH uses existing OpenSSH keys, agents, configuration, jump hosts, and `known_hosts` behavior.
- Provider-returned SSH command strings are not executed. Host, port, user, and path are consumed as
  typed values and passed as separate process arguments.
- Signed export URLs are secret-bearing and stay inside the downloader.
- New artifact destinations use restrictive permissions and staging files.
- Archive extraction must prevent traversal, symlink escape, overwrite, decompression bombs, and
  unbounded disk use before it is enabled.

Machine mode never prompts. If an operation needs human confirmation or additional authority, it
returns a typed error instead of guessing.
