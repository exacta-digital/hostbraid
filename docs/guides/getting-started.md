HostBraid works at the hosting layer: accounts, sites, environments, access coordinates, portable
artifacts, and local pulls. It delegates WordPress commands to WP-CLI and connections to OpenSSH.

The current release is the walking skeleton. Start by checking the local tools HostBraid will use:

```bash
hostbraid doctor
```

Explore without leaving the terminal:

```bash
hostbraid guide --list
hostbraid search ssh
hostbraid search export
hostbraid completion --help
```

For automation, switch to the versioned machine contract:

```bash
hostbraid --output json --no-input doctor
hostbraid --output json --no-input search environment
```

Provider profiles and live inventory are the next vertical slice. This build intentionally accepts
no API credentials yet; it establishes the CLI and safety contracts before connecting to customer
environments.
