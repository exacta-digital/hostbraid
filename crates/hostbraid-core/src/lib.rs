//! Provider-neutral contracts shared by HostBraid frontends and adapters.
//!
//! This crate deliberately contains no Clap, terminal, HTTP, or provider-specific types. Provider
//! adapters translate their wire formats into these contracts; the CLI decides how to present them.

mod error;
mod machine;
mod model;
mod provider;

pub use error::{AppError, ErrorCode, Result};
pub use machine::{MachineFailure, MachineMeta, MachineSuccess, MachineWarning};
pub use model::{
    ActionClass, ArtifactRef, EnvironmentKind, EnvironmentRef, OpaqueId, ProfileName, ProviderId,
    ProviderProfileRef, SiteRef,
};
pub use provider::{
    ArtifactCatalog, ArtifactKind, ArtifactSummary, Capability, CapabilitySource, Catalog,
    EnvironmentSummary, ProviderDescriptor, ProviderIdentity, SiteSummary, SshAccess, SshTarget,
};

/// Version of HostBraid's public JSON envelope.
pub const MACHINE_SCHEMA_VERSION: u32 = 1;
