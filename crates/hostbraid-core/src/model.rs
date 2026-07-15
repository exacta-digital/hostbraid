use crate::{AppError, ErrorCode, Result};
use serde::{Deserialize, Deserializer, Serialize, de};
use std::fmt;

fn validate_component(value: &str, label: &str, max_len: usize) -> Result<()> {
    if value.is_empty() {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            format!("{label} cannot be empty"),
        ));
    }
    if value.len() > max_len {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            format!("{label} is longer than {max_len} bytes"),
        ));
    }
    if value.trim() != value || value.chars().any(char::is_control) {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            format!("{label} contains surrounding whitespace or control characters"),
        ));
    }
    Ok(())
}

/// Stable slug identifying a compiled provider adapter.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct ProviderId(String);

impl ProviderId {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        validate_component(&value, "provider id", 64)?;
        if !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "provider id must contain only lowercase ASCII letters, digits, and hyphens",
            ));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ProviderId {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(de::Error::custom)
    }
}

/// User-chosen local name for one provider account or company.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct ProfileName(String);

impl ProfileName {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        validate_component(&value, "profile name", 64)?;
        if !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "profile name must contain only ASCII letters, digits, hyphens, and underscores",
            ));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProfileName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ProfileName {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(de::Error::custom)
    }
}

/// Opaque identifier returned by a provider.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct OpaqueId(String);

impl OpaqueId {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        validate_component(&value, "provider identifier", 512)?;
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for OpaqueId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for OpaqueId {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(de::Error::custom)
    }
}

/// One configured account for a provider.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProviderProfileRef {
    pub provider: ProviderId,
    pub profile: ProfileName,
}

impl ProviderProfileRef {
    pub fn try_new(provider: impl Into<String>, profile: impl Into<String>) -> Result<Self> {
        Ok(Self {
            provider: ProviderId::new(provider)?,
            profile: ProfileName::new(profile)?,
        })
    }
}

/// Canonical reference to a provider site.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SiteRef {
    pub provider: ProviderId,
    pub profile: ProfileName,
    pub site_id: OpaqueId,
}

impl SiteRef {
    pub fn try_new(
        provider: impl Into<String>,
        profile: impl Into<String>,
        site_id: impl Into<String>,
    ) -> Result<Self> {
        Ok(Self {
            provider: ProviderId::new(provider)?,
            profile: ProfileName::new(profile)?,
            site_id: OpaqueId::new(site_id)?,
        })
    }

    #[must_use]
    pub fn profile_ref(&self) -> ProviderProfileRef {
        ProviderProfileRef {
            provider: self.provider.clone(),
            profile: self.profile.clone(),
        }
    }
}

/// Canonical reference to a provider environment.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EnvironmentRef {
    pub provider: ProviderId,
    pub profile: ProfileName,
    pub site_id: OpaqueId,
    pub environment_id: OpaqueId,
}

impl EnvironmentRef {
    pub fn try_new(
        provider: impl Into<String>,
        profile: impl Into<String>,
        site_id: impl Into<String>,
        environment_id: impl Into<String>,
    ) -> Result<Self> {
        Ok(Self {
            provider: ProviderId::new(provider)?,
            profile: ProfileName::new(profile)?,
            site_id: OpaqueId::new(site_id)?,
            environment_id: OpaqueId::new(environment_id)?,
        })
    }

    #[must_use]
    pub fn site_ref(&self) -> SiteRef {
        SiteRef {
            provider: self.provider.clone(),
            profile: self.profile.clone(),
            site_id: self.site_id.clone(),
        }
    }
}

/// Canonical reference to an export or other downloadable artifact.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ArtifactRef {
    pub environment: EnvironmentRef,
    pub artifact_id: OpaqueId,
}

/// Provider-neutral environment classification while preserving provider terminology separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum EnvironmentKind {
    Production,
    Staging,
    Development,
    Other,
}

/// Security classification used by policy and user confirmation layers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ActionClass {
    ReadOnly,
    LocalWrite,
    RemoteWrite,
    Destructive,
    ArbitraryCode,
}

#[cfg(test)]
mod tests {
    use super::{EnvironmentRef, OpaqueId, ProfileName, ProviderId};

    #[test]
    fn provider_ids_are_normalized_slugs() {
        assert!(ProviderId::new("kinsta").is_ok());
        assert!(ProviderId::new("Kinsta").is_err());
        assert!(ProviderId::new("kinsta api").is_err());
    }

    #[test]
    fn profile_names_are_human_friendly_but_bounded() {
        assert!(ProfileName::new("agency-se").is_ok());
        assert!(ProfileName::new("agency/se").is_err());
    }

    #[test]
    fn environment_reference_serialization_is_explicit() {
        let reference = EnvironmentRef::try_new("kinsta", "agency", "site_1", "env_2")
            .expect("valid reference");
        let value = serde_json::to_value(reference).expect("reference serializes");

        assert_eq!(value["provider"], "kinsta");
        assert_eq!(value["profile"], "agency");
        assert_eq!(value["site_id"], "site_1");
        assert_eq!(value["environment_id"], "env_2");
    }

    #[test]
    fn deserialization_cannot_bypass_identifier_validation() {
        assert!(serde_json::from_str::<ProviderId>(r#""Kinsta""#).is_err());
        assert!(serde_json::from_str::<ProfileName>(r#"" agency ""#).is_err());
        assert!(serde_json::from_str::<OpaqueId>(r#""line\nbreak""#).is_err());
    }
}
