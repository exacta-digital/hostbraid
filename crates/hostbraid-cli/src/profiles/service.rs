use super::credential::{CredentialCandidate, CredentialKeyring, SecretToken};
use super::model::{CredentialSource, ProfileRecord, ValidatedCredential};
use super::store::ProfileStore;
use super::{format_profile_ref, parse_profile_ref};
use hostbraid_core::{AppError, ErrorCode, ProviderProfileRef, Result};
use serde::Serialize;
use std::cell::{Cell, RefCell};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ProfileRemovalOutcome {
    pub profile: ProfileRecord,
    pub credential_cleanup_failed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct CredentialUpdateOutcome {
    pub profile: ProfileRecord,
    pub credential_cleanup_failed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProfileSnapshot {
    pub profile: ProfileRecord,
    pub is_default: bool,
}

pub(crate) struct ProfileService<'a> {
    store: &'a ProfileStore,
    keyring: &'a dyn CredentialKeyring,
}

impl<'a> ProfileService<'a> {
    pub(crate) const fn new(store: &'a ProfileStore, keyring: &'a dyn CredentialKeyring) -> Self {
        Self { store, keyring }
    }

    #[cfg(test)]
    pub(crate) fn list(&self) -> Result<Vec<ProfileRecord>> {
        Ok(self.store.load()?.profiles)
    }

    pub(crate) fn show(&self, reference: &ProviderProfileRef) -> Result<ProfileRecord> {
        Ok(self.show_snapshot(reference)?.profile)
    }

    pub(crate) fn show_snapshot(&self, reference: &ProviderProfileRef) -> Result<ProfileSnapshot> {
        let configuration = self.store.load()?;
        let profile = configuration
            .find(reference)
            .cloned()
            .ok_or_else(profile_not_found)?;
        Ok(ProfileSnapshot {
            profile,
            is_default: configuration.default_profile.as_ref() == Some(reference),
        })
    }

    pub(crate) fn select(&self, explicit: Option<&str>) -> Result<ProfileRecord> {
        let configuration = self.store.load()?;
        let reference = if let Some(explicit) = explicit {
            parse_profile_ref(explicit)?
        } else {
            configuration.default_profile.clone().ok_or_else(|| {
                AppError::new(
                    ErrorCode::InvalidInput,
                    "no default provider profile is configured",
                )
                .with_hint("Pass --profile provider:name or run `hb use provider:name`.")
            })?
        };
        configuration
            .find(&reference)
            .cloned()
            .ok_or_else(profile_not_found)
    }

    pub(crate) fn add(
        &self,
        reference: ProviderProfileRef,
        credential: &CredentialCandidate,
        validated: ValidatedCredential,
        make_default: bool,
    ) -> Result<ProfileRecord> {
        let profile = ProfileRecord {
            provider: reference.provider.clone(),
            name: reference.profile.clone(),
            company_id: validated.company_id,
            credential_source: credential.source().clone(),
            credential_expires_at: validated.expires_at,
        };
        let keyring_written = Cell::new(false);
        let result = self.store.update(|configuration| {
            if configuration.find(&reference).is_some() {
                return Err(AppError::new(
                    ErrorCode::InvalidInput,
                    "a profile with that exact reference already exists",
                )
                .with_hint(format!(
                    "Use `hostbraid profile credential set {}` to rotate its credential.",
                    format_profile_ref(&reference)
                )));
            }

            if matches!(credential.source(), CredentialSource::Keyring) {
                self.keyring.set(&reference, credential.token())?;
                keyring_written.set(true);
            }
            configuration.profiles.push(profile.clone());
            if make_default {
                configuration.default_profile = Some(reference.clone());
            }
            Ok(profile.clone())
        });

        if result.is_err() && keyring_written.get() && self.keyring.delete(&reference).is_err() {
            return Err(rollback_failed());
        }
        result
    }

    pub(crate) fn set_default(&self, reference: &ProviderProfileRef) -> Result<ProfileRecord> {
        self.store.update(|configuration| {
            let profile = configuration
                .find(reference)
                .cloned()
                .ok_or_else(profile_not_found)?;
            configuration.default_profile = Some(reference.clone());
            Ok(profile)
        })
    }

    pub(crate) fn remove(&self, reference: &ProviderProfileRef) -> Result<ProfileRemovalOutcome> {
        let profile = self.store.update(|configuration| {
            let index = configuration
                .profiles
                .iter()
                .position(|profile| profile.reference() == *reference)
                .ok_or_else(profile_not_found)?;
            let profile = configuration.profiles.remove(index);
            if configuration.default_profile.as_ref() == Some(reference) {
                configuration.default_profile = None;
            }
            Ok(profile)
        })?;

        let credential_cleanup_failed =
            matches!(profile.credential_source, CredentialSource::Keyring)
                && self.keyring.delete(reference).is_err();
        Ok(ProfileRemovalOutcome {
            profile,
            credential_cleanup_failed,
        })
    }

    pub(crate) fn set_credential(
        &self,
        reference: &ProviderProfileRef,
        credential: &CredentialCandidate,
        validated: &ValidatedCredential,
    ) -> Result<CredentialUpdateOutcome> {
        let previous_token = RefCell::new(None::<SecretToken>);
        let previous_was_keyring = Cell::new(false);
        let new_keyring_written = Cell::new(false);

        let result = self.store.update(|configuration| {
            let profile = configuration
                .profiles
                .iter_mut()
                .find(|profile| profile.reference() == *reference)
                .ok_or_else(profile_not_found)?;
            if profile.company_id != validated.company_id {
                return Err(AppError::new(
                    ErrorCode::PolicyDenied,
                    "the replacement credential belongs to a different provider company",
                )
                .with_hint("Create a new profile for credentials belonging to another company."));
            }

            let old_keyring = matches!(profile.credential_source, CredentialSource::Keyring);
            previous_was_keyring.set(old_keyring);
            if matches!(credential.source(), CredentialSource::Keyring) {
                if old_keyring {
                    *previous_token.borrow_mut() = self.keyring.get_optional(reference)?;
                }
                self.keyring.set(reference, credential.token())?;
                new_keyring_written.set(true);
            }

            profile.credential_source = credential.source().clone();
            profile.credential_expires_at = validated.expires_at.clone();
            Ok(profile.clone())
        });

        let profile = match result {
            Ok(profile) => profile,
            Err(error) => {
                if new_keyring_written.get() {
                    let rollback = if let Some(previous) = previous_token.borrow().as_ref() {
                        self.keyring.set(reference, previous)
                    } else {
                        self.keyring.delete(reference)
                    };
                    if rollback.is_err() {
                        return Err(rollback_failed());
                    }
                }
                return Err(error);
            }
        };

        let changed_away_from_keyring =
            previous_was_keyring.get() && !matches!(credential.source(), CredentialSource::Keyring);
        let credential_cleanup_failed =
            changed_away_from_keyring && self.keyring.delete(reference).is_err();
        Ok(CredentialUpdateOutcome {
            profile,
            credential_cleanup_failed,
        })
    }
}

fn profile_not_found() -> AppError {
    AppError::new(
        ErrorCode::NotFound,
        "the exact provider profile was not found",
    )
}

fn rollback_failed() -> AppError {
    AppError::new(
        ErrorCode::Internal,
        "the profile configuration was not changed, but credential rollback failed",
    )
    .with_hint("Run `hostbraid profile credential set provider:name` before using the profile.")
}

#[cfg(test)]
mod tests {
    use super::ProfileService;
    use crate::profiles::{
        ConfigPaths, CredentialCandidate, CredentialKeyring, CredentialSource, ProfileStore,
        SecretToken, ValidatedCredential, parse_profile_ref,
    };
    use hostbraid_core::{ErrorCode, ProviderProfileRef, Result};
    use std::collections::HashMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    static TEST_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let sequence = TEST_DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "hostbraid-profile-service-test-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("create test directory");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[derive(Default)]
    struct FakeKeyring {
        values: Mutex<HashMap<String, String>>,
        fail_delete: AtomicBool,
    }

    impl FakeKeyring {
        fn value(&self, reference: &ProviderProfileRef) -> Option<String> {
            self.values
                .lock()
                .expect("keyring lock")
                .get(&crate::profiles::format_profile_ref(reference))
                .cloned()
        }
    }

    impl CredentialKeyring for FakeKeyring {
        fn set(&self, profile: &ProviderProfileRef, token: &SecretToken) -> Result<()> {
            self.values.lock().expect("keyring lock").insert(
                crate::profiles::format_profile_ref(profile),
                token.expose_secret().to_owned(),
            );
            Ok(())
        }

        fn get_optional(&self, profile: &ProviderProfileRef) -> Result<Option<SecretToken>> {
            self.value(profile).map(SecretToken::new).transpose()
        }

        fn delete(&self, profile: &ProviderProfileRef) -> Result<()> {
            if self.fail_delete.load(Ordering::Relaxed) {
                return Err(hostbraid_core::AppError::new(
                    ErrorCode::Unavailable,
                    "test keyring delete failed",
                ));
            }
            self.values
                .lock()
                .expect("keyring lock")
                .remove(&crate::profiles::format_profile_ref(profile));
            Ok(())
        }
    }

    fn keyring_candidate(value: &str) -> CredentialCandidate {
        CredentialCandidate::new(
            CredentialSource::Keyring,
            SecretToken::new(value.to_owned()).expect("token"),
        )
    }

    fn validated(company: &str) -> ValidatedCredential {
        ValidatedCredential::new(company, None).expect("validated credential")
    }

    #[test]
    fn add_list_show_and_default_are_secret_free() {
        let directory = TestDirectory::new();
        let paths = ConfigPaths::from_home(directory.path());
        let store = ProfileStore::new(paths.clone());
        let keyring = FakeKeyring::default();
        let service = ProfileService::new(&store, &keyring);
        let reference = parse_profile_ref("kinsta:agency").expect("reference");
        let profile = service
            .add(
                reference.clone(),
                &keyring_candidate("never-write-this"),
                validated("company-1"),
                true,
            )
            .expect("add profile");

        let profiles = service.list().expect("list");
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0], profile);
        assert_eq!(service.show(&reference).expect("show"), profile);
        let snapshot = service
            .show_snapshot(&reference)
            .expect("coherent profile snapshot");
        assert_eq!(snapshot.profile, profile);
        assert!(snapshot.is_default);
        assert_eq!(
            service.select(None).expect("default").name.as_str(),
            "agency"
        );
        let contents = fs::read_to_string(paths.config_file()).expect("configuration");
        assert!(!contents.contains("never-write-this"));
    }

    #[test]
    fn sole_profile_is_not_inferred_without_a_default() {
        let directory = TestDirectory::new();
        let store = ProfileStore::new(ConfigPaths::from_home(directory.path()));
        let keyring = FakeKeyring::default();
        let service = ProfileService::new(&store, &keyring);
        let reference = parse_profile_ref("kinsta:agency").expect("reference");
        service
            .add(
                reference.clone(),
                &keyring_candidate("secret"),
                validated("company-1"),
                false,
            )
            .expect("add profile");

        assert!(
            !service
                .show_snapshot(&reference)
                .expect("coherent profile snapshot")
                .is_default
        );
        let error = service.select(None).expect_err("default is required");
        assert_eq!(error.code(), ErrorCode::InvalidInput);
    }

    #[test]
    fn rotation_rejects_a_different_company_without_changing_secret() {
        let directory = TestDirectory::new();
        let store = ProfileStore::new(ConfigPaths::from_home(directory.path()));
        let keyring = FakeKeyring::default();
        let service = ProfileService::new(&store, &keyring);
        let reference = parse_profile_ref("kinsta:agency").expect("reference");
        service
            .add(
                reference.clone(),
                &keyring_candidate("old-secret"),
                validated("company-1"),
                false,
            )
            .expect("add profile");

        let error = service
            .set_credential(
                &reference,
                &keyring_candidate("new-secret"),
                &validated("company-2"),
            )
            .expect_err("company change is rejected");
        assert_eq!(error.code(), ErrorCode::PolicyDenied);
        assert_eq!(keyring.value(&reference).as_deref(), Some("old-secret"));
    }

    #[test]
    fn changing_to_environment_removes_managed_keyring_credential() {
        let directory = TestDirectory::new();
        let store = ProfileStore::new(ConfigPaths::from_home(directory.path()));
        let keyring = FakeKeyring::default();
        let service = ProfileService::new(&store, &keyring);
        let reference = parse_profile_ref("kinsta:agency").expect("reference");
        service
            .add(
                reference.clone(),
                &keyring_candidate("old-secret"),
                validated("company-1"),
                false,
            )
            .expect("add profile");
        let candidate = CredentialCandidate::new(
            CredentialSource::environment("KINSTA_TOKEN").expect("source"),
            SecretToken::new("environment-secret".to_owned()).expect("token"),
        );

        let outcome = service
            .set_credential(&reference, &candidate, &validated("company-1"))
            .expect("change source");
        assert!(!outcome.credential_cleanup_failed);
        assert!(keyring.value(&reference).is_none());
    }

    #[test]
    fn removal_clears_default_and_surfaces_keyring_cleanup_failure() {
        let directory = TestDirectory::new();
        let store = ProfileStore::new(ConfigPaths::from_home(directory.path()));
        let keyring = FakeKeyring::default();
        let service = ProfileService::new(&store, &keyring);
        let reference = parse_profile_ref("kinsta:agency").expect("reference");
        service
            .add(
                reference.clone(),
                &keyring_candidate("secret"),
                validated("company-1"),
                true,
            )
            .expect("add profile");
        keyring.fail_delete.store(true, Ordering::Relaxed);

        let outcome = service.remove(&reference).expect("remove profile");
        assert!(outcome.credential_cleanup_failed);
        assert!(service.list().expect("list").is_empty());
        assert_eq!(
            service.select(None).expect_err("default removed").code(),
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn committed_metadata_does_not_trigger_keyring_rollback_when_parent_sync_fails() {
        let directory = TestDirectory::new();
        let store =
            ProfileStore::new(ConfigPaths::from_home(directory.path())).with_parent_sync_failure();
        let keyring = FakeKeyring::default();
        let service = ProfileService::new(&store, &keyring);
        let reference = parse_profile_ref("kinsta:agency").expect("reference");

        let profile = service
            .add(
                reference.clone(),
                &keyring_candidate("committed-secret"),
                validated("company-1"),
                false,
            )
            .expect("rename commits the profile despite a later sync failure");

        assert_eq!(
            service.show(&reference).expect("committed profile"),
            profile
        );
        assert_eq!(
            keyring.value(&reference).as_deref(),
            Some("committed-secret")
        );
    }
}
