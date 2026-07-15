use super::model::{CredentialSource, ProfileRecord, validate_environment_variable};
use super::{format_profile_ref, parse_profile_ref};
use hostbraid_core::{AppError, ErrorCode, ProviderProfileRef, Result};
use std::ffi::OsString;
use std::fmt;
use std::io::{self, Read};
use zeroize::{Zeroize, Zeroizing};

const MAX_TOKEN_BYTES: u64 = 64 * 1024;
const KEYRING_SERVICE: &str = "hostbraid";

pub(crate) struct SecretToken(String);

impl SecretToken {
    pub(crate) fn new(mut value: String) -> Result<Self> {
        if value.is_empty()
            || value.len() > MAX_TOKEN_BYTES as usize
            || value
                .chars()
                .any(|character| matches!(character, '\r' | '\n' | '\0') || character.is_control())
        {
            value.zeroize();
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "credential token must be one non-empty line with no control characters",
            )
            .with_hint(
                "Enter the token at the hidden prompt, or pipe exactly one UTF-8 token with `--token-stdin`; tokens are never accepted as command arguments.",
            ));
        }
        Ok(Self(value))
    }

    pub(crate) fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretToken([REDACTED])")
    }
}

impl Drop for SecretToken {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

pub(crate) trait TokenInput {
    fn read_stdin(&self) -> Result<SecretToken>;
    fn prompt_hidden(&self) -> Result<SecretToken>;
}

pub(crate) struct TerminalTokenInput;

impl TokenInput for TerminalTokenInput {
    fn read_stdin(&self) -> Result<SecretToken> {
        read_bounded_token(io::stdin().lock())
    }

    fn prompt_hidden(&self) -> Result<SecretToken> {
        let token = rpassword::prompt_password("Kinsta API token: ").map_err(|error| {
            AppError::io(
                "failed to read a hidden credential from the terminal",
                &error,
            )
            .with_hint(
                "Run the command in an interactive terminal, or use `--token-stdin` or `--credential-env <NAME>`.",
            )
        })?;
        SecretToken::new(token)
    }
}

pub(crate) trait EnvironmentReader {
    fn read(&self, variable: &str) -> Option<OsString>;
}

pub(crate) struct ProcessEnvironment;

impl EnvironmentReader for ProcessEnvironment {
    fn read(&self, variable: &str) -> Option<OsString> {
        std::env::var_os(variable)
    }
}

pub(crate) struct CredentialCandidate {
    source: CredentialSource,
    token: SecretToken,
}

impl CredentialCandidate {
    pub(crate) fn new(source: CredentialSource, token: SecretToken) -> Self {
        Self { source, token }
    }

    pub(crate) fn source(&self) -> &CredentialSource {
        &self.source
    }

    pub(crate) fn token(&self) -> &SecretToken {
        &self.token
    }
}

impl fmt::Debug for CredentialCandidate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialCandidate")
            .field("source", &self.source)
            .field("token", &"[REDACTED]")
            .finish()
    }
}

pub(crate) fn collect_credential(
    token_stdin: bool,
    credential_env: Option<&str>,
    interactive: bool,
    input: &dyn TokenInput,
    environment: &dyn EnvironmentReader,
) -> Result<CredentialCandidate> {
    match (token_stdin, credential_env) {
        (true, Some(_)) => Err(AppError::new(
            ErrorCode::InvalidArguments,
            "choose exactly one credential source",
        )
        .with_hint("Use either `--token-stdin` or `--credential-env <NAME>`, not both.")),
        (true, None) => Ok(CredentialCandidate::new(
            CredentialSource::Keyring,
            input.read_stdin()?,
        )),
        (false, Some(variable)) => {
            validate_environment_variable(variable)?;
            let token = token_from_environment(variable, environment)?;
            Ok(CredentialCandidate::new(
                CredentialSource::environment(variable)?,
                token,
            ))
        }
        (false, None) if interactive => Ok(CredentialCandidate::new(
            CredentialSource::Keyring,
            input.prompt_hidden()?,
        )),
        (false, None) => Err(AppError::new(
            ErrorCode::InvalidInput,
            "a credential source is required in non-interactive mode",
        )
        .with_hint(
            "Pipe a token with `--token-stdin`, or name an existing variable with `--credential-env <NAME>`.",
        )),
    }
}

pub(crate) trait CredentialKeyring: Send + Sync {
    fn set(&self, profile: &ProviderProfileRef, token: &SecretToken) -> Result<()>;
    fn get_optional(&self, profile: &ProviderProfileRef) -> Result<Option<SecretToken>>;
    fn delete(&self, profile: &ProviderProfileRef) -> Result<()>;
}

pub(crate) struct OsCredentialKeyring;

impl CredentialKeyring for OsCredentialKeyring {
    fn set(&self, profile: &ProviderProfileRef, token: &SecretToken) -> Result<()> {
        let entry = keyring_entry(profile)?;
        entry.set_password(token.expose_secret()).map_err(|_| {
            AppError::new(
                ErrorCode::Unavailable,
                "the OS credential store could not save the profile credential",
            )
            .with_hint(
                "Unlock or configure the OS credential store, or use `--credential-env <NAME>` instead.",
            )
        })
    }

    fn get_optional(&self, profile: &ProviderProfileRef) -> Result<Option<SecretToken>> {
        let entry = keyring_entry(profile)?;
        match entry.get_password() {
            Ok(token) => SecretToken::new(token).map(Some),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(AppError::new(
                ErrorCode::Unavailable,
                "the OS credential store could not read the profile credential",
            )
            .with_hint(format!(
                "Unlock the OS credential store; if the entry was removed, run `hb profile credential set {}`.",
                format_profile_ref(profile)
            ))),
        }
    }

    fn delete(&self, profile: &ProviderProfileRef) -> Result<()> {
        let entry = keyring_entry(profile)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(AppError::new(
                ErrorCode::Unavailable,
                "the OS credential store could not remove the profile credential",
            )
            .with_hint(format!(
                "Unlock the OS credential store and remove the HostBraid entry for {} manually if cleanup continues to fail.",
                format_profile_ref(profile)
            ))),
        }
    }
}

pub(crate) fn resolve_credential(
    profile: &ProfileRecord,
    keyring: &dyn CredentialKeyring,
    environment: &dyn EnvironmentReader,
) -> Result<SecretToken> {
    match &profile.credential_source {
        CredentialSource::Keyring => keyring.get_optional(&profile.reference())?.ok_or_else(|| {
            AppError::new(
                ErrorCode::AuthenticationFailed,
                "the profile credential is missing from the OS credential store",
            )
            .with_hint(format!(
                "Run `hb profile credential set {}` and enter the replacement token securely.",
                format_profile_ref(&profile.reference())
            ))
        }),
        CredentialSource::Environment { variable } => token_from_environment(variable, environment),
    }
}

fn token_from_environment(
    variable: &str,
    environment: &dyn EnvironmentReader,
) -> Result<SecretToken> {
    let value = environment.read(variable).ok_or_else(|| {
        AppError::new(
            ErrorCode::AuthenticationFailed,
            "the configured credential environment variable is not set",
        )
        .with_hint(format!(
            "Set `{variable}` in the current process environment, or change the profile credential source."
        ))
    })?;
    let value = value.into_string().map_err(|_| {
        AppError::new(
            ErrorCode::AuthenticationFailed,
            "the configured credential environment variable is not valid UTF-8",
        )
        .with_hint("Set the variable to one valid UTF-8 token, then retry.")
    })?;
    SecretToken::new(value).map_err(|_| {
        AppError::new(
            ErrorCode::AuthenticationFailed,
            "the configured credential environment variable is empty or malformed",
        )
        .with_hint(
            "Set the variable to one non-empty token without control characters or embedded newlines.",
        )
    })
}

fn read_bounded_token(reader: impl Read) -> Result<SecretToken> {
    // `read_to_end` can leave a partially filled buffer when it returns an error. Keep that
    // buffer zeroizing so even the error path scrubs any credential bytes already received.
    let mut bytes = Zeroizing::new(Vec::new());
    reader
        .take(MAX_TOKEN_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            AppError::io("failed to read a credential from stdin", &error)
                .with_hint("Pipe exactly one token followed by EOF, then retry.")
        })?;
    if bytes.len() > MAX_TOKEN_BYTES as usize {
        bytes.zeroize();
        return Err(
            AppError::new(ErrorCode::InvalidInput, "credential token is too long").with_hint(
                "Provide the API token only; do not pipe JSON, headers, or command output.",
            ),
        );
    }

    let mut value = match String::from_utf8(std::mem::take(&mut *bytes)) {
        Ok(value) => value,
        Err(error) => {
            let mut bytes = error.into_bytes();
            bytes.zeroize();
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "credential token from stdin is not valid UTF-8",
            )
            .with_hint("Pipe the token as UTF-8 text."));
        }
    };
    if value.ends_with('\n') {
        value.pop();
        if value.ends_with('\r') {
            value.pop();
        }
    }
    SecretToken::new(value)
}

fn keyring_entry(profile: &ProviderProfileRef) -> Result<keyring::Entry> {
    // Reparse our own canonical value before handing it to a platform keyring. This keeps the
    // keyring account derivation coupled to the exact public profile-reference grammar.
    let account = format_profile_ref(profile);
    parse_profile_ref(&account)?;
    keyring::Entry::new(KEYRING_SERVICE, &account).map_err(|_| {
        AppError::new(
            ErrorCode::Unavailable,
            "the OS credential store is unavailable",
        )
        .with_hint(
            "Configure a supported desktop credential store, or use `--credential-env <NAME>` instead.",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::{
        CredentialCandidate, CredentialKeyring, EnvironmentReader, SecretToken, TokenInput,
        collect_credential, read_bounded_token, resolve_credential,
    };
    use crate::profiles::{CredentialSource, ProfileRecord, parse_profile_ref};
    use hostbraid_core::{
        ErrorCode, OpaqueId, ProfileName, ProviderId, ProviderProfileRef, Result,
    };
    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::io::{self, Read};
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeEnvironment(HashMap<String, OsString>);

    impl EnvironmentReader for FakeEnvironment {
        fn read(&self, variable: &str) -> Option<OsString> {
            self.0.get(variable).cloned()
        }
    }

    struct FakeInput;

    impl TokenInput for FakeInput {
        fn read_stdin(&self) -> Result<SecretToken> {
            SecretToken::new("stdin-secret".to_owned())
        }

        fn prompt_hidden(&self) -> Result<SecretToken> {
            SecretToken::new("prompt-secret".to_owned())
        }
    }

    #[derive(Default)]
    struct FakeKeyring(Mutex<HashMap<String, String>>);

    impl CredentialKeyring for FakeKeyring {
        fn set(&self, profile: &ProviderProfileRef, token: &SecretToken) -> Result<()> {
            self.0.lock().expect("keyring lock").insert(
                crate::profiles::format_profile_ref(profile),
                token.expose_secret().to_owned(),
            );
            Ok(())
        }

        fn get_optional(&self, profile: &ProviderProfileRef) -> Result<Option<SecretToken>> {
            self.0
                .lock()
                .expect("keyring lock")
                .get(&crate::profiles::format_profile_ref(profile))
                .cloned()
                .map(SecretToken::new)
                .transpose()
        }

        fn delete(&self, profile: &ProviderProfileRef) -> Result<()> {
            self.0
                .lock()
                .expect("keyring lock")
                .remove(&crate::profiles::format_profile_ref(profile));
            Ok(())
        }
    }

    fn profile(source: CredentialSource) -> ProfileRecord {
        ProfileRecord {
            provider: ProviderId::new("kinsta").expect("provider"),
            name: ProfileName::new("agency").expect("profile"),
            company_id: OpaqueId::new("company-1").expect("company"),
            credential_source: source,
            credential_expires_at: None,
        }
    }

    #[test]
    fn stdin_accepts_one_line_and_never_exposes_it_in_debug() {
        let token = read_bounded_token("super-secret\r\n".as_bytes()).expect("token");
        assert_eq!(token.expose_secret(), "super-secret");
        assert!(!format!("{token:?}").contains("super-secret"));
    }

    #[test]
    fn invalid_tokens_explain_safe_input_without_echoing_the_value() {
        for secret_canary in [
            "secret-token-canary-never-render\n",
            "secret-token-canary\tnever-render",
        ] {
            let error = SecretToken::new(secret_canary.to_owned()).expect_err("token is malformed");

            assert_eq!(
                error.message(),
                "credential token must be one non-empty line with no control characters"
            );
            assert!(
                error
                    .hint()
                    .is_some_and(|hint| hint.contains("--token-stdin"))
            );
            assert!(!error.message().contains(secret_canary));
            assert!(
                !error
                    .hint()
                    .is_some_and(|hint| hint.contains(secret_canary))
            );
        }
    }

    #[test]
    fn stdin_read_errors_after_partial_input_are_secret_safe() {
        struct PartialErrorReader(bool);

        impl Read for PartialErrorReader {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                if self.0 {
                    return Err(io::Error::other("injected read failure"));
                }
                self.0 = true;
                let secret = b"partial-secret-canary";
                buffer[..secret.len()].copy_from_slice(secret);
                Ok(secret.len())
            }
        }

        let error = read_bounded_token(PartialErrorReader(false)).expect_err("read must fail");

        assert_eq!(error.code(), ErrorCode::Io);
        assert!(error.hint().is_some_and(|hint| hint.contains("EOF")));
        assert!(!error.to_string().contains("partial-secret-canary"));
        assert!(!format!("{error:?}").contains("partial-secret-canary"));
    }

    #[test]
    fn credential_collection_requires_an_explicit_noninteractive_source() {
        let environment = FakeEnvironment::default();
        let error = collect_credential(false, None, false, &FakeInput, &environment)
            .expect_err("source is required");
        assert_eq!(error.code(), ErrorCode::InvalidInput);
        assert!(
            error
                .hint()
                .is_some_and(|hint| hint.contains("--credential-env <NAME>"))
        );

        let conflict =
            collect_credential(true, Some("KINSTA_TOKEN"), false, &FakeInput, &environment)
                .expect_err("sources conflict");
        assert_eq!(conflict.code(), ErrorCode::InvalidArguments);
        assert!(
            conflict
                .hint()
                .is_some_and(|hint| hint.contains("not both"))
        );

        let candidate = collect_credential(true, None, false, &FakeInput, &environment)
            .expect("stdin credential");
        assert!(matches!(candidate.source(), CredentialSource::Keyring));
        assert_eq!(candidate.token().expose_secret(), "stdin-secret");
    }

    #[test]
    fn environment_source_is_named_but_token_is_not_serializable() {
        let environment = FakeEnvironment(HashMap::from([(
            "KINSTA_TOKEN".to_owned(),
            OsString::from("environment-secret"),
        )]));
        let candidate =
            collect_credential(false, Some("KINSTA_TOKEN"), false, &FakeInput, &environment)
                .expect("environment credential");
        assert_eq!(candidate.token().expose_secret(), "environment-secret");
        let json = serde_json::to_string(candidate.source()).expect("serialize source");
        assert!(json.contains("KINSTA_TOKEN"));
        assert!(!json.contains("environment-secret"));
    }

    #[test]
    fn keyring_and_environment_resolution_are_injectable() {
        let keyring = FakeKeyring::default();
        let reference = parse_profile_ref("kinsta:agency").expect("reference");
        let token = SecretToken::new("stored-secret".to_owned()).expect("token");
        keyring.set(&reference, &token).expect("store token");

        let resolved = resolve_credential(
            &profile(CredentialSource::Keyring),
            &keyring,
            &FakeEnvironment::default(),
        )
        .expect("resolve keyring");
        assert_eq!(resolved.expose_secret(), "stored-secret");

        let missing = profile(CredentialSource::environment("KINSTA_TOKEN").expect("source"));
        let error = resolve_credential(&missing, &keyring, &FakeEnvironment::default())
            .expect_err("environment is missing");
        assert_eq!(error.code(), ErrorCode::AuthenticationFailed);
        assert!(
            error
                .hint()
                .is_some_and(|hint| hint.contains("KINSTA_TOKEN"))
        );

        let missing = profile(CredentialSource::Keyring);
        let error = resolve_credential(
            &missing,
            &FakeKeyring::default(),
            &FakeEnvironment::default(),
        )
        .expect_err("keyring entry is missing");
        assert_eq!(error.code(), ErrorCode::AuthenticationFailed);
        assert!(
            error
                .hint()
                .is_some_and(|hint| hint.contains("hb profile credential set kinsta:agency"))
        );
    }

    #[test]
    fn candidate_debug_redacts_token() {
        let candidate = CredentialCandidate::new(
            CredentialSource::Keyring,
            SecretToken::new("do-not-print".to_owned()).expect("token"),
        );
        assert!(!format!("{candidate:?}").contains("do-not-print"));
    }
}
