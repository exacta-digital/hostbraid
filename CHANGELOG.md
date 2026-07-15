# Changelog

All notable changes to HostBraid will be documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
intends to follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Initial Rust workspace and Clap walking skeleton.
- Built-in guides, command search, environment doctor, and shell completions.
- Versioned JSON success and error envelopes.
- Provider-neutral domain and capability contracts.
- Secure Kinsta profiles using the OS credential store or named environment references, with
  hidden-prompt and bounded-stdin token input.
- Ergonomic `login`, `profiles`, `use`, and `logout` account commands backed by the canonical
  profile operations and stable `profile.*` machine identities.
- Kinsta site/environment discovery, capability inspection, and read-only plugin/theme inventory.
- Interactive and one-shot OpenSSH workflows with selector confirmation, bounded fan-out, graceful
  batch signal cancellation, optional timeout/fail-fast controls, and best-effort 60-second
  connection reuse.
- Structured partial-failure JSON for captured SSH results.
