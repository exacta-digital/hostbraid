# Roadmap

## Foundation — complete

- Rust workspace with isolated core and CLI crates.
- Human and JSON output contracts.
- Built-in guides and generated command search.
- Local dependency doctor and shell completions.
- Provider-neutral identifiers, capabilities, SSH coordinates, and artifact metadata.

## First Kinsta vertical slice — implemented, live validation ongoing

- Platform-aware profile configuration with no stored secret values.
- OS credential-store integration, named environment sources, and bounded stdin credential input.
- Exact `provider:name` profile selection and an explicit configured default.
- Kinsta authentication, company profile, site list, and environment list/show.
- Capability inspection and structured SSH target discovery.
- Interactive OpenSSH delegation.
- One-shot OpenSSH execution for exact and broad selectors, including preview/confirmation policy.
- Bounded-parallel fan-out with per-target timeout, fail-fast, ordered results, and bounded capture.
- Best-effort 60-second OpenSSH connection reuse without weakening host-key verification.
- Read-only company plugin/theme inventory with update/vulnerability/search filters and detail
  expansion.
- Versioned partial-failure JSON that retains captured SSH execution results.

The implementation is covered by provider fixtures and local process tests. Opt-in checks against a
real restricted Kinsta account remain part of release validation; see
[validation-spikes.md](validation-spikes.md).

## Next Kinsta workflow validation

- Existing downloadable-export metadata and safe download.
- Browser-free pull spike with a manifest and explicit consistency semantics.
- Real-account validation for large catalogs, SSH availability, batching, rate limits, and company
  inventory joins.

## Second adapter and abstraction proof

- EdHosting service-account authentication.
- The same catalog, environment, capability, SSH, and artifact contract.
- Provider contract fixtures and conformance tests.

## Later, only after evidence

- Curated WP-CLI bridge.
- DDEV pull integration.
- Provider inventory update actions, only with a separate mutation contract and explicit policy.
- Additional built-in providers.
- Versioned subprocess adapter protocol.
- SQLite only for demonstrated durable-job or indexed-cache needs.

HostBraid will not add MCP. Agent support remains the stable CLI and JSON contract.
