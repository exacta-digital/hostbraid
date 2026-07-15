use crate::{
    AppError, ArtifactRef, EnvironmentKind, EnvironmentRef, ErrorCode, ProviderId,
    ProviderProfileRef, Result, SiteRef,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Static information about one provider adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderDescriptor {
    pub id: ProviderId,
    pub display_name: String,
    pub documentation_url: Option<String>,
}

/// Minimal site information returned by inventory operations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiteSummary {
    pub reference: SiteRef,
    pub display_name: String,
    pub primary_domain: Option<String>,
}

/// Minimal environment information returned by inventory operations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentSummary {
    pub reference: EnvironmentRef,
    pub display_name: String,
    pub kind: EnvironmentKind,
    pub provider_kind: Option<String>,
    pub primary_domain: Option<String>,
}

/// Support and current availability of one versioned provider capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capability {
    pub name: String,
    pub version: u16,
    pub supported: bool,
    pub available: bool,
    pub reason: Option<String>,
    pub remediation: Option<String>,
}

impl Capability {
    #[must_use]
    pub fn available(name: impl Into<String>, version: u16) -> Self {
        Self {
            name: name.into(),
            version,
            supported: true,
            available: true,
            reason: None,
            remediation: None,
        }
    }

    #[must_use]
    pub fn unsupported(name: impl Into<String>, version: u16) -> Self {
        Self {
            name: name.into(),
            version,
            supported: false,
            available: false,
            reason: Some("unsupported".to_owned()),
            remediation: None,
        }
    }

    #[must_use]
    pub fn unavailable(
        name: impl Into<String>,
        version: u16,
        reason: impl Into<String>,
        remediation: Option<String>,
    ) -> Self {
        Self {
            name: name.into(),
            version,
            supported: true,
            available: false,
            reason: Some(reason.into()),
            remediation,
        }
    }
}

/// Structured SSH coordinates. Provider-returned shell command strings are never authoritative.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SshTarget {
    host: String,
    port: u16,
    user: String,
    working_directory: Option<String>,
}

impl SshTarget {
    pub fn try_new(
        host: impl Into<String>,
        port: u16,
        user: impl Into<String>,
        working_directory: Option<String>,
    ) -> Result<Self> {
        let host = host.into();
        let user = user.into();
        validate_ssh_host(&host)?;
        validate_ssh_user(&user)?;
        if port == 0 {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "SSH port must be greater than zero",
            ));
        }
        if let Some(directory) = &working_directory {
            validate_working_directory(directory)?;
        }
        Ok(Self {
            host,
            port,
            user,
            working_directory,
        })
    }

    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    #[must_use]
    pub const fn port(&self) -> u16 {
        self.port
    }

    #[must_use]
    pub fn user(&self) -> &str {
        &self.user
    }

    #[must_use]
    pub fn working_directory(&self) -> Option<&str> {
        self.working_directory.as_deref()
    }
}

fn validate_ssh_host(host: &str) -> Result<()> {
    let valid = !host.is_empty()
        && host.len() <= 255
        && host.is_ascii()
        && !host.starts_with('-')
        && host.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b':' | b'[' | b']')
        });
    if valid {
        return Ok(());
    }
    Err(AppError::new(
        ErrorCode::InvalidInput,
        "SSH host is empty, unsafe, or longer than 255 bytes",
    ))
}

fn validate_ssh_user(user: &str) -> Result<()> {
    let valid = !user.is_empty()
        && user.len() <= 64
        && !user.starts_with('-')
        && user
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'));
    if valid {
        return Ok(());
    }
    Err(AppError::new(
        ErrorCode::InvalidInput,
        "SSH user is empty, unsafe, or longer than 64 bytes",
    ))
}

fn validate_working_directory(directory: &str) -> Result<()> {
    let valid = directory.starts_with('/')
        && directory.len() <= 4096
        && !directory.chars().any(char::is_control);
    if valid {
        return Ok(());
    }
    Err(AppError::new(
        ErrorCode::InvalidInput,
        "SSH working directory must be an absolute, control-free path of at most 4096 bytes",
    ))
}

/// Portable provider artifact types. Internal restore points are intentionally not artifacts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ArtifactKind {
    ProviderExport,
    DatabaseDump,
    FilesArchive,
}

/// Secret-free metadata for an export or download.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactSummary {
    pub reference: ArtifactRef,
    pub kind: ArtifactKind,
    pub created_at: Option<String>,
    pub expires_at: Option<String>,
    pub size_bytes: Option<u64>,
}

/// Static identity shared by all capability-specific provider traits.
pub trait ProviderIdentity: Send + Sync {
    fn descriptor(&self) -> &ProviderDescriptor;
}

/// Read-only provider inventory.
#[async_trait]
pub trait Catalog: ProviderIdentity {
    async fn list_sites(&self, profile: &ProviderProfileRef) -> Result<Vec<SiteSummary>>;

    async fn list_environments(&self, site: &SiteRef) -> Result<Vec<EnvironmentSummary>>;
}

/// Environment capability discovery.
#[async_trait]
pub trait CapabilitySource: ProviderIdentity {
    async fn capabilities(&self, environment: &EnvironmentRef) -> Result<Vec<Capability>>;
}

/// Structured SSH access discovery.
#[async_trait]
pub trait SshAccess: ProviderIdentity {
    async fn ssh_target(&self, environment: &EnvironmentRef) -> Result<SshTarget>;
}

/// Portable artifact metadata. Download authorization remains inside the provider adapter.
#[async_trait]
pub trait ArtifactCatalog: ProviderIdentity {
    async fn list_artifacts(&self, environment: &EnvironmentRef) -> Result<Vec<ArtifactSummary>>;
}

#[cfg(test)]
mod tests {
    use super::{Capability, SshTarget};

    #[test]
    fn capability_distinguishes_support_from_availability() {
        let disabled = Capability::unavailable(
            "connection.ssh",
            1,
            "ssh_disabled",
            Some("Enable SSH in the provider dashboard".to_owned()),
        );

        assert!(disabled.supported);
        assert!(!disabled.available);
        assert_eq!(disabled.reason.as_deref(), Some("ssh_disabled"));
    }

    #[test]
    fn ssh_target_rejects_option_and_terminal_injection() {
        assert!(SshTarget::try_new("-oProxyCommand=bad", 22, "user", None).is_err());
        assert!(SshTarget::try_new("host.example", 22, "user\nroot", None).is_err());
        assert!(
            SshTarget::try_new("host.example", 22, "user", Some("relative/path".to_owned()))
                .is_err()
        );
    }

    #[test]
    fn ssh_target_accepts_hostname_ipv6_and_absolute_path() {
        let hostname = SshTarget::try_new(
            "host.example",
            61000,
            "site_user",
            Some("/www/site/public".to_owned()),
        )
        .expect("valid hostname target");
        let ipv6 =
            SshTarget::try_new("[2001:db8::1]", 22, "site-user", None).expect("valid IPv6 target");

        assert_eq!(hostname.host(), "host.example");
        assert_eq!(hostname.port(), 61000);
        assert_eq!(hostname.working_directory(), Some("/www/site/public"));
        assert_eq!(ipv6.user(), "site-user");
    }
}
