Providers use “backup” for several incompatible things. HostBraid uses three explicit concepts.

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
