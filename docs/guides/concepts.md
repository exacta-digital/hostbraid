HostBraid keeps provider identity, catalog discovery, inventory, execution, and artifact workflows
separate rather than hiding them behind one generic hosting operation.

**Profile**

A local, exact `provider:name` reference to one validated provider company/account. A profile stores
the provider, company ID, credential source, and optional expiry metadata, but never the credential
value. A configured default is authoritative; HostBraid does not guess one from the number of
profiles.

**Site and environment catalog**

A site owns environments. Provider-issued opaque site and environment IDs are canonical; display
names, domains, kinds, and labels are useful context but are not identity. Catalog commands are
read-only snapshots used to resolve later operations consistently.

**WordPress inventory**

Company-wide plugin/theme metadata and the environments where each component is installed. This is
read-only provider inventory, not WP-CLI passthrough and not an update mechanism.

**SSH execution**

HostBraid resolves structured host, port, and user coordinates, then delegates to OpenSSH. Opening a
shell and passing a remote command are arbitrary-code actions. Repeated selector values are ORed
inside a category and categories are ANDed. Selectors that can implicitly expand beyond exact
environment IDs require a target preview and confirmation.

Providers also use “backup” for several incompatible things. HostBraid uses three explicit artifact
concepts.

**Snapshot**

A provider-internal restore point. It may support restore or clone operations without ever becoming
a portable file. Creating or restoring one is a remote mutation.

**Export**

A portable artifact, usually site files plus a database dump. Export links are often short-lived
bearer capabilities and are never included in ordinary machine output.

**Pull**

A HostBraid workflow that brings files and a database to a new local destination. It may download a
ready export or use SSH with rsync, SFTP, or a tar stream. A live SSH pull is not an atomic snapshot;
its manifest must describe the selected strategy and consistency limits.

Capability discovery reports both whether a provider supports an operation and whether it is
currently available for a particular environment. For example, SSH can be supported but disabled.
HostBraid does not silently perform the remote mutation required to enable it.
