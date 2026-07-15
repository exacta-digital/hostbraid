# Contributing to HostBraid

Thank you for helping make hosting workflows calmer and more portable.

## Before opening a change

1. Read [AGENTS.md](AGENTS.md) and the relevant design note under `docs/design/`.
2. Keep the change focused on one verifiable behavior.
3. Add or update tests for public CLI and JSON behavior.
4. Run `just check` or the equivalent Cargo commands.

Provider work should begin with a documented API contract and redacted fixtures. A provider adapter
must not expose its wire DTOs to `hostbraid-core`, execute provider-returned shell strings, or leak
bearer-capability download URLs.

## Commit and pull request notes

Explain:

- the user-visible outcome;
- affected command and JSON contracts;
- security or credential implications;
- validation performed;
- known provider-specific limitations.

By contributing, you agree that your contribution may be distributed under the repository's dual
Apache-2.0/MIT license.
