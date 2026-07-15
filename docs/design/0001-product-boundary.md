# 0001: Product boundary

Status: accepted, 2026-07-15.

## Decision

HostBraid is a provider-neutral hosting environment CLI, not a WP-CLI replacement and not an MCP
server. It owns discovery, normalized references, provider capabilities, artifact metadata, pull
planning, policy, and stable machine output.

It delegates WordPress operations to WP-CLI, terminals to OpenSSH, transfers to established local
tools, and local development to DDEV or equivalent workflows.

## Consequences

- Kinsta and EdHosting adapters are compiled in first; a runtime plugin protocol is deferred.
- The same vertical slice must work across two providers before the abstraction is considered proven.
- Provider wire types remain private to adapters.
- Inventory, SSH, snapshots, exports, and pulls remain distinct application capabilities.
- No dashboard scraping, Git/Composer wrappers, daemon, TUI, MCP, or SQLite is part of the first
  release.
