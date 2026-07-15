Human output is HostBraid's default. It favors concise explanations, clear next actions, and safe
defaults:

- Automatic colors appear only on a terminal and respect `NO_COLOR`; an explicit `--color always`
  overrides automatic detection.
- Progress is transient, goes to stderr, and disappears in pipes or JSON mode.
- Commands explain their arguments through contextual `--help`.
- Typos receive Clap suggestions.
- `hostbraid search <word>` searches commands and the full guide text.
- `hostbraid completion <shell>` integrates discovery into your shell.

HostBraid will use the system OpenSSH client rather than asking you to maintain a second set of SSH
keys and host fingerprints. Unknown or changed host keys remain visible decisions; they are never
silently accepted.

Use `--quiet` to suppress transient progress. Use `--color never` when recording human output.

`hostbraid doctor` is diagnostic. Missing tools are reported as readiness booleans and warnings; a
success status means the report was produced, not that every optional workflow is available.
