use hostbraid_core::{
    AppError, ErrorCode, OpaqueId, ProfileName, ProviderId, ProviderProfileRef, Result,
};
use serde::{Deserialize, Serialize};

pub(crate) const CONFIG_SCHEMA_VERSION: u32 = 1;
const CONFIG_REPAIR_HINT: &str =
    "Back up profiles.json, then repair it or move it aside; recreate profiles with `hb login`.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum CredentialSource {
    Keyring,
    Environment { variable: String },
}

impl CredentialSource {
    pub(crate) fn environment(variable: impl Into<String>) -> Result<Self> {
        let variable = variable.into();
        validate_environment_variable(&variable)?;
        Ok(Self::Environment { variable })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProfileRecord {
    pub provider: ProviderId,
    pub name: ProfileName,
    pub company_id: OpaqueId,
    pub credential_source: CredentialSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_expires_at: Option<String>,
}

impl ProfileRecord {
    pub(crate) fn reference(&self) -> ProviderProfileRef {
        ProviderProfileRef {
            provider: self.provider.clone(),
            profile: self.name.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProfileConfig {
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_profile: Option<ProviderProfileRef>,
    #[serde(default)]
    pub profiles: Vec<ProfileRecord>,
}

impl Default for ProfileConfig {
    fn default() -> Self {
        Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            default_profile: None,
            profiles: Vec::new(),
        }
    }
}

impl ProfileConfig {
    pub(crate) fn find(&self, reference: &ProviderProfileRef) -> Option<&ProfileRecord> {
        self.profiles
            .iter()
            .find(|profile| same_reference(&profile.reference(), reference))
    }

    pub(crate) fn normalize_and_validate(&mut self) -> Result<()> {
        if self.schema_version != CONFIG_SCHEMA_VERSION {
            return Err(AppError::new(
                ErrorCode::Unsupported,
                "profile configuration uses an unsupported schema version",
            )
            .with_hint(
                "Install a HostBraid version that supports this profile file before running profile or provider commands.",
            ));
        }

        self.profiles.sort_by(|left, right| {
            left.provider
                .as_str()
                .cmp(right.provider.as_str())
                .then_with(|| left.name.as_str().cmp(right.name.as_str()))
        });

        for profile in &self.profiles {
            if let CredentialSource::Environment { variable } = &profile.credential_source {
                validate_environment_variable(variable).map_err(|_| {
                    AppError::new(
                        ErrorCode::InvalidInput,
                        "profile configuration contains an invalid credential environment variable",
                    )
                    .with_hint(CONFIG_REPAIR_HINT)
                })?;
            }
            if profile.credential_expires_at.as_ref().is_some_and(|value| {
                value.len() > 128 || value.trim() != value || value.chars().any(char::is_control)
            }) {
                return Err(AppError::new(
                    ErrorCode::InvalidInput,
                    "profile configuration contains invalid credential expiry metadata",
                )
                .with_hint(CONFIG_REPAIR_HINT));
            }
        }

        if self
            .profiles
            .windows(2)
            .any(|profiles| profiles[0].reference() == profiles[1].reference())
        {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "profile configuration contains a duplicate profile reference",
            )
            .with_hint(CONFIG_REPAIR_HINT));
        }

        if self
            .default_profile
            .as_ref()
            .is_some_and(|reference| self.find(reference).is_none())
        {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "the configured default profile does not exist",
            )
            .with_hint(CONFIG_REPAIR_HINT));
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ValidatedCredential {
    pub company_id: OpaqueId,
    pub expires_at: Option<String>,
}

impl ValidatedCredential {
    pub(crate) fn new(company_id: impl Into<String>, expires_at: Option<String>) -> Result<Self> {
        if expires_at.as_ref().is_some_and(|value| {
            value.len() > 128 || value.trim() != value || value.chars().any(char::is_control)
        }) {
            return Err(invalid_provider_credential_metadata());
        }
        Ok(Self {
            company_id: OpaqueId::new(company_id)
                .map_err(|_| invalid_provider_credential_metadata())?,
            expires_at,
        })
    }
}

pub(crate) fn parse_profile_ref(value: &str) -> Result<ProviderProfileRef> {
    let Some((provider, profile)) = value.split_once(':') else {
        return Err(invalid_profile_reference());
    };
    if profile.contains(':') {
        return Err(invalid_profile_reference());
    }
    ProviderProfileRef::try_new(provider, profile).map_err(|_| invalid_profile_reference())
}

pub(crate) fn format_profile_ref(reference: &ProviderProfileRef) -> String {
    format!("{}:{}", reference.provider, reference.profile)
}

pub(crate) fn validate_environment_variable(value: &str) -> Result<()> {
    let valid = !value.is_empty()
        && value.len() <= 128
        && !value.as_bytes()[0].is_ascii_digit()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
    if !valid {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "credential environment variable name is invalid",
        )
        .with_hint(
            "Use 1–128 ASCII letters, digits, or underscores, and do not start with a digit.",
        ));
    }
    Ok(())
}

fn invalid_profile_reference() -> AppError {
    AppError::new(
        ErrorCode::InvalidInput,
        "profile reference must use the exact provider:name form",
    )
    .with_hint("For example, `kinsta:agency`. Run `hb profiles` to list valid references.")
}

fn invalid_provider_credential_metadata() -> AppError {
    AppError::new(
        ErrorCode::InvalidInput,
        "the provider returned credential metadata HostBraid could not validate",
    )
    .with_hint(
        "Update HostBraid and retry; if it repeats, report the provider compatibility issue.",
    )
}

fn same_reference(left: &ProviderProfileRef, right: &ProviderProfileRef) -> bool {
    left == right
}

#[cfg(test)]
mod tests {
    use super::{
        CONFIG_SCHEMA_VERSION, CredentialSource, ProfileConfig, ProfileRecord, ValidatedCredential,
        format_profile_ref, parse_profile_ref, validate_environment_variable,
    };
    use hostbraid_core::{ErrorCode, OpaqueId, ProfileName, ProviderId};

    fn profile(name: &str) -> ProfileRecord {
        ProfileRecord {
            provider: ProviderId::new("kinsta").expect("valid provider"),
            name: ProfileName::new(name).expect("valid profile"),
            company_id: OpaqueId::new(format!("company-{name}")).expect("valid company"),
            credential_source: CredentialSource::Keyring,
            credential_expires_at: None,
        }
    }

    #[test]
    fn profile_references_are_exact_and_canonical() {
        let reference = parse_profile_ref("kinsta:agency-se").expect("valid reference");
        assert_eq!(format_profile_ref(&reference), "kinsta:agency-se");

        for invalid in [
            "kinsta",
            "kinsta:",
            ":agency",
            "Kinsta:agency",
            "kinsta:agency:extra",
            "kinsta: agency",
        ] {
            let error = parse_profile_ref(invalid).expect_err("reference is invalid");
            assert_eq!(error.code(), ErrorCode::InvalidInput);
            assert!(
                error
                    .hint()
                    .is_some_and(|hint| hint.contains("hb profiles"))
            );
            assert!(!error.message().contains(invalid));
        }
    }

    #[test]
    fn configuration_is_sorted_and_rejects_duplicates() {
        let mut configuration = ProfileConfig {
            schema_version: CONFIG_SCHEMA_VERSION,
            default_profile: None,
            profiles: vec![profile("zeta"), profile("alpha")],
        };
        configuration
            .normalize_and_validate()
            .expect("configuration is valid");
        assert_eq!(configuration.profiles[0].name.as_str(), "alpha");

        configuration.profiles.push(profile("alpha"));
        let error = configuration
            .normalize_and_validate()
            .expect_err("duplicates are rejected");
        assert!(
            error
                .hint()
                .is_some_and(|hint| hint.contains("Back up profiles.json"))
        );
    }

    #[test]
    fn default_must_reference_an_existing_profile() {
        let mut configuration = ProfileConfig {
            default_profile: Some(
                parse_profile_ref("kinsta:missing").expect("syntactically valid reference"),
            ),
            ..ProfileConfig::default()
        };
        let error = configuration
            .normalize_and_validate()
            .expect_err("missing default is rejected");
        assert_eq!(error.code(), ErrorCode::InvalidInput);
        assert!(error.hint().is_some_and(|hint| hint.contains("hb login")));
    }

    #[test]
    fn invalid_environment_names_explain_the_complete_safe_grammar() {
        let invalid = "SECRET=token-canary";
        let error = validate_environment_variable(invalid).expect_err("name is invalid");

        assert!(error.hint().is_some_and(|hint| hint.contains("1–128")));
        assert!(
            error
                .hint()
                .is_some_and(|hint| hint.contains("do not start with a digit"))
        );
        assert!(!error.message().contains(invalid));
        assert!(!error.hint().is_some_and(|hint| hint.contains(invalid)));
    }

    #[test]
    fn invalid_provider_credential_metadata_has_provider_remediation() {
        let invalid = "secret-company-canary\n";
        let error =
            ValidatedCredential::new(invalid, None).expect_err("provider metadata is invalid");

        assert_eq!(error.code(), ErrorCode::InvalidInput);
        assert!(
            error
                .hint()
                .is_some_and(|hint| hint.contains("Update HostBraid"))
        );
        assert!(!error.message().contains(invalid));
        assert!(!error.hint().is_some_and(|hint| hint.contains(invalid)));
    }
}
