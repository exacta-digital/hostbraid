# Roadmap

## Foundation — current

- Rust workspace with isolated core and CLI crates.
- Human and JSON output contracts.
- Built-in guides and generated command search.
- Local dependency doctor and shell completions.
- Provider-neutral identifiers, capabilities, SSH coordinates, and artifact metadata.

## First Kinsta vertical slice

- XDG profile configuration with no stored secret values.
- Credential-helper/keyring decision from a portability spike.
- Kinsta authentication, company profile, site list, and environment list/show.
- Capability inspection and structured SSH target discovery.
- Interactive OpenSSH delegation.
- Existing downloadable-export metadata and safe download.
- Browser-free pull spike with a manifest and explicit consistency semantics.

## Second adapter and abstraction proof

- EdHosting service-account authentication.
- The same catalog, environment, capability, SSH, and artifact contract.
- Provider contract fixtures and conformance tests.

## Later, only after evidence

- Curated WP-CLI bridge.
- DDEV pull integration.
- Additional built-in providers.
- Versioned subprocess adapter protocol.
- SQLite only for demonstrated durable-job or indexed-cache needs.

HostBraid will not add MCP. Agent support remains the stable CLI and JSON contract.
