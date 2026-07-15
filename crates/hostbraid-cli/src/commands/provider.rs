use super::catalog::{
    EnvironmentSelection, ResolvedEnvironment, resolve_environment, resolve_site,
    select_environments, validate_opaque_selector, validate_selection_syntax,
};
use crate::CommandOutcome;
use crate::cli::{
    Commands, EnvironmentCommand, EnvironmentListArgs, EnvironmentShowArgs, InventoryCommand,
    InventoryListArgs, ProfileAddArgs, ProfileCommand, ProfileCredentialCommand,
    ProfileCredentialSetArgs, ProfileRefArgs, ProfileRemoveArgs, ProfileSelectionArgs, SiteCommand,
    SshCommand, SshOpenArgs, SshRunArgs,
};
use crate::context::Context;
use crate::output;
use crate::profiles::{
    ConfigPaths, CredentialSource, OsCredentialKeyring, ProcessEnvironment, ProfileRecord,
    ProfileService, ProfileSnapshot, ProfileStore, TerminalTokenInput, ValidatedCredential,
    collect_credential, format_profile_ref, parse_profile_ref, resolve_credential,
};
use crate::ssh::{
    BatchOptions, BatchReport, CaptureEncoding, ExecutionFailureCode, ExecutionState,
    ExecutionTarget, OpenSsh, ProcessSignalGuard, RunOptions,
};
use crate::text::{terminal_output_safe, terminal_safe};
use futures_util::{StreamExt, stream};
use hostbraid_core::{
    AppError, CapabilitySource, EnvironmentKind, ErrorCode, MachineEnvironmentListData,
    MachineEnvironmentShowData, MachineInventoryData, MachineSshCaptureEncoding,
    MachineSshCapturedStream, MachineSshExecutionState, MachineSshFailure, MachineSshFailureCode,
    MachineSshRunData, MachineSshTargetResult, MachineWarning, OpaqueId, ProviderProfileRef,
    Result, SiteSummary, WordPressComponentInventory, WordPressComponentKind,
};
use hostbraid_provider_kinsta::KinstaProvider;
use serde::Serialize;
use std::collections::VecDeque;

pub(crate) async fn run(command: Commands, context: &Context) -> Result<CommandOutcome> {
    match command {
        Commands::Login(arguments) => add_profile(arguments.into(), context).await,
        Commands::Profiles => list_profiles(context),
        Commands::Use(arguments) => set_default_profile(arguments, context),
        Commands::Logout(arguments) => remove_profile(arguments, context),
        Commands::Profile(arguments) => run_profile(arguments.command, context).await,
        Commands::Site(arguments) => run_site(arguments.command, context).await,
        Commands::Environment(arguments) => run_environment(arguments.command, context).await,
        Commands::Ssh(arguments) => run_ssh(arguments.command, context).await,
        Commands::Inventory(arguments) => run_inventory(arguments.command, context).await,
        Commands::Guide(_) | Commands::Search(_) | Commands::Doctor | Commands::Completion(_) => {
            Err(AppError::new(
                ErrorCode::Internal,
                "a non-provider command reached the provider dispatcher",
            ))
        }
    }
}

#[derive(Debug, Serialize)]
struct ProfileView {
    reference: ProviderProfileRef,
    company_id: OpaqueId,
    credential_source: CredentialSource,
    credential_expires_at: Option<String>,
    is_default: bool,
}

impl ProfileView {
    fn new(profile: ProfileRecord, default: Option<&ProviderProfileRef>) -> Self {
        let reference = profile.reference();
        let is_default = default == Some(&reference);
        Self {
            reference,
            company_id: profile.company_id,
            credential_source: profile.credential_source,
            credential_expires_at: profile.credential_expires_at,
            is_default,
        }
    }
}

#[derive(Debug, Serialize)]
struct ProfileListOutput {
    default_profile: Option<ProviderProfileRef>,
    profiles: Vec<ProfileView>,
}

#[derive(Debug, Serialize)]
struct ProfileMutationOutput {
    profile: ProfileView,
    credential_cleanup_failed: bool,
}

async fn run_profile(command: ProfileCommand, context: &Context) -> Result<CommandOutcome> {
    match command {
        ProfileCommand::Add(arguments) => add_profile(arguments, context).await,
        ProfileCommand::List => list_profiles(context),
        ProfileCommand::Show(arguments) => show_profile(arguments, context),
        ProfileCommand::Default(arguments) => set_default_profile(arguments, context),
        ProfileCommand::Remove(arguments) => remove_profile(arguments, context),
        ProfileCommand::Credential(arguments) => match arguments.command {
            ProfileCredentialCommand::Set(arguments) => {
                set_profile_credential(arguments, context).await
            }
        },
    }
}

async fn add_profile(arguments: ProfileAddArgs, context: &Context) -> Result<CommandOutcome> {
    let reference = ProviderProfileRef::try_new(arguments.provider.as_str(), arguments.name)?;
    let store = profile_store()?;
    if store.load()?.find(&reference).is_some() {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "a profile with that exact reference already exists",
        )
        .with_hint(format!(
            "Use `hostbraid profile credential set {}` to rotate its credential.",
            format_profile_ref(&reference)
        )));
    }

    let candidate = collect_credential(
        arguments.token_stdin,
        arguments.credential_env.as_deref(),
        context.interactive,
        &TerminalTokenInput,
        &ProcessEnvironment,
    )?;
    let spinner = context.spinner("Validating Kinsta API credential");
    let authentication = KinstaProvider::authenticate(candidate.token().expose_secret()).await;
    spinner.finish_and_clear();
    let (_provider, validation) = authentication?;
    let validated =
        ValidatedCredential::new(validation.company_id.as_str(), validation.expires_at)?;
    let keyring = OsCredentialKeyring;
    let service = ProfileService::new(&store, &keyring);
    let profile = service.add(reference.clone(), &candidate, validated, arguments.default)?;
    let view = ProfileView::new(profile, arguments.default.then_some(&reference));
    write_profile_result("profile.add", &view, "Added", context)
}

fn list_profiles(context: &Context) -> Result<CommandOutcome> {
    let store = profile_store()?;
    let configuration = store.load()?;
    let default_profile = configuration.default_profile.clone();
    let data = ProfileListOutput {
        profiles: configuration
            .profiles
            .into_iter()
            .map(|profile| ProfileView::new(profile, default_profile.as_ref()))
            .collect(),
        default_profile,
    };

    if context.output.is_machine() {
        output::write_machine_success("profile.list", &data, Vec::new())?;
        return Ok(CommandOutcome::Success);
    }
    if data.profiles.is_empty() {
        output::write_human(
            "No provider profiles are configured.\nLog in with `hb login kinsta <name>`.\n",
        )?;
        return Ok(CommandOutcome::Success);
    }

    let mut contents = String::from("Provider profiles\n\n");
    for profile in &data.profiles {
        contents.push_str(&format!(
            "  {}{}  company={}  credential={}\n",
            terminal_safe(&format_profile_ref(&profile.reference)),
            if profile.is_default { " (default)" } else { "" },
            terminal_safe(profile.company_id.as_str()),
            credential_source_label(&profile.credential_source),
        ));
    }
    output::write_human(&contents)?;
    Ok(CommandOutcome::Success)
}

fn show_profile(arguments: ProfileRefArgs, context: &Context) -> Result<CommandOutcome> {
    let reference = parse_profile_ref(&arguments.profile)?;
    let store = profile_store()?;
    let keyring = OsCredentialKeyring;
    let service = ProfileService::new(&store, &keyring);
    let ProfileSnapshot {
        profile,
        is_default,
    } = service.show_snapshot(&reference)?;
    let view = ProfileView::new(profile, is_default.then_some(&reference));
    write_profile_result("profile.show", &view, "Profile", context)
}

fn set_default_profile(arguments: ProfileRefArgs, context: &Context) -> Result<CommandOutcome> {
    let reference = parse_profile_ref(&arguments.profile)?;
    let store = profile_store()?;
    let keyring = OsCredentialKeyring;
    let service = ProfileService::new(&store, &keyring);
    let profile = service.set_default(&reference)?;
    let view = ProfileView::new(profile, Some(&reference));
    write_profile_result("profile.default", &view, "Default profile", context)
}

fn remove_profile(arguments: ProfileRemoveArgs, context: &Context) -> Result<CommandOutcome> {
    let reference = parse_profile_ref(&arguments.profile)?;
    let store = profile_store()?;
    let keyring = OsCredentialKeyring;
    let service = ProfileService::new(&store, &keyring);
    let _profile = service.show(&reference)?;

    if !arguments.yes {
        let confirmed = context.confirm(&format!(
            "Remove profile {}?",
            terminal_safe(&format_profile_ref(&reference))
        ))?;
        if !confirmed {
            return Err(AppError::new(
                ErrorCode::PolicyDenied,
                "profile removal was not confirmed",
            ));
        }
    }

    let outcome = service.remove(&reference)?;
    let cleanup_failed = outcome.credential_cleanup_failed;
    let data = ProfileMutationOutput {
        profile: ProfileView::new(outcome.profile, None),
        credential_cleanup_failed: cleanup_failed,
    };
    let warnings = cleanup_warning(cleanup_failed);
    if context.output.is_machine() {
        output::write_machine_success("profile.remove", &data, warnings)?;
    } else {
        output::write_human(&format!(
            "Removed profile {}.\n",
            terminal_safe(&format_profile_ref(&data.profile.reference))
        ))?;
        write_human_warnings(&warnings)?;
    }
    Ok(CommandOutcome::Success)
}

async fn set_profile_credential(
    arguments: ProfileCredentialSetArgs,
    context: &Context,
) -> Result<CommandOutcome> {
    let reference = parse_profile_ref(&arguments.profile)?;
    let store = profile_store()?;
    let keyring = OsCredentialKeyring;
    let service = ProfileService::new(&store, &keyring);
    let existing = service.show(&reference)?;
    if existing.provider.as_str() != "kinsta" {
        return Err(AppError::new(
            ErrorCode::Unsupported,
            "credential rotation is not implemented for the selected provider",
        ));
    }
    let candidate = collect_credential(
        arguments.token_stdin,
        arguments.credential_env.as_deref(),
        context.interactive,
        &TerminalTokenInput,
        &ProcessEnvironment,
    )?;

    let spinner = context.spinner("Validating replacement Kinsta API credential");
    let authentication = KinstaProvider::authenticate(candidate.token().expose_secret()).await;
    spinner.finish_and_clear();
    let (_provider, validation) = authentication?;
    let validated =
        ValidatedCredential::new(validation.company_id.as_str(), validation.expires_at)?;
    let outcome = service.set_credential(&reference, &candidate, &validated)?;
    let cleanup_failed = outcome.credential_cleanup_failed;
    let configuration = store.load()?;
    let data = ProfileMutationOutput {
        profile: ProfileView::new(outcome.profile, configuration.default_profile.as_ref()),
        credential_cleanup_failed: cleanup_failed,
    };
    let warnings = cleanup_warning(cleanup_failed);
    if context.output.is_machine() {
        output::write_machine_success("profile.credential.set", &data, warnings)?;
    } else {
        output::write_human(&format!(
            "Updated the credential for {}.\n",
            terminal_safe(&format_profile_ref(&data.profile.reference))
        ))?;
        write_human_warnings(&warnings)?;
    }
    Ok(CommandOutcome::Success)
}

fn write_profile_result(
    command: &str,
    profile: &ProfileView,
    action: &str,
    context: &Context,
) -> Result<CommandOutcome> {
    if context.output.is_machine() {
        output::write_machine_success(command, profile, Vec::new())?;
    } else {
        output::write_human(&format!(
            "{action}: {}\n  company: {}\n  credential: {}\n  default: {}\n",
            terminal_safe(&format_profile_ref(&profile.reference)),
            terminal_safe(profile.company_id.as_str()),
            credential_source_label(&profile.credential_source),
            if profile.is_default { "yes" } else { "no" },
        ))?;
    }
    Ok(CommandOutcome::Success)
}

fn credential_source_label(source: &CredentialSource) -> String {
    match source {
        CredentialSource::Keyring => "OS credential store".to_owned(),
        CredentialSource::Environment { variable } => {
            format!("environment: {}", terminal_safe(variable))
        }
    }
}

fn cleanup_warning(failed: bool) -> Vec<MachineWarning> {
    if failed {
        vec![MachineWarning::new(
            "credential_cleanup_failed",
            "The profile changed, but its previous OS credential could not be removed",
        )]
    } else {
        Vec::new()
    }
}

fn write_human_warnings(warnings: &[MachineWarning]) -> Result<()> {
    for warning in warnings {
        output::write_human_stderr(&format!("warning: {}\n", warning.message))?;
    }
    Ok(())
}

fn profile_store() -> Result<ProfileStore> {
    Ok(ProfileStore::new(ConfigPaths::discover()?))
}

struct KinstaSession {
    profile: ProfileRecord,
    provider: KinstaProvider,
}

fn kinsta_session(selection: &ProfileSelectionArgs) -> Result<KinstaSession> {
    let store = profile_store()?;
    let keyring = OsCredentialKeyring;
    let service = ProfileService::new(&store, &keyring);
    let profile = service.select(selection.profile.as_deref())?;
    if profile.provider.as_str() != "kinsta" {
        return Err(AppError::new(
            ErrorCode::Unsupported,
            "the selected profile uses a provider that is not compiled into this build",
        ));
    }
    let token = resolve_credential(&profile, &keyring, &ProcessEnvironment)?;
    let provider = KinstaProvider::for_company(token.expose_secret(), profile.company_id.as_str())?;
    Ok(KinstaSession { profile, provider })
}

async fn run_site(command: SiteCommand, context: &Context) -> Result<CommandOutcome> {
    match command {
        SiteCommand::List(selection) => {
            let session = kinsta_session(&selection)?;
            let reference = session.profile.reference();
            let spinner = context.spinner("Loading Kinsta sites and environments");
            let result = session.provider.catalog_snapshot(&reference).await;
            spinner.finish_and_clear();
            let snapshot = result?;
            let sites: Vec<SiteSummary> =
                snapshot.sites.into_iter().map(|site| site.site).collect();
            if context.output.is_machine() {
                output::write_machine_success("site.list", &sites, Vec::new())?;
            } else {
                render_sites(&sites)?;
            }
            Ok(CommandOutcome::Success)
        }
    }
}

async fn run_environment(command: EnvironmentCommand, context: &Context) -> Result<CommandOutcome> {
    match command {
        EnvironmentCommand::List(arguments) => list_environments(arguments, context).await,
        EnvironmentCommand::Show(arguments) => show_environment(arguments, context).await,
    }
}

async fn list_environments(
    arguments: EnvironmentListArgs,
    context: &Context,
) -> Result<CommandOutcome> {
    validate_opaque_selector(&arguments.site_id)?;
    let session = kinsta_session(&arguments.selection)?;
    let reference = session.profile.reference();
    let spinner = context.spinner("Loading Kinsta environments");
    let result = session.provider.catalog_snapshot(&reference).await;
    spinner.finish_and_clear();
    let site = resolve_site(&result?, &arguments.site_id)?;
    let data = MachineEnvironmentListData {
        site: site.site,
        environments: site.environments,
    };
    if context.output.is_machine() {
        output::write_machine_success("environment.list", &data, Vec::new())?;
    } else {
        render_environments(&data)?;
    }
    Ok(CommandOutcome::Success)
}

async fn show_environment(
    arguments: EnvironmentShowArgs,
    context: &Context,
) -> Result<CommandOutcome> {
    validate_opaque_selector(&arguments.environment_id)?;
    let session = kinsta_session(&arguments.selection)?;
    let reference = session.profile.reference();
    let spinner = context.spinner("Loading Kinsta environment");
    let snapshot_result = session.provider.catalog_snapshot(&reference).await;
    spinner.finish_and_clear();
    let resolved = resolve_environment(&snapshot_result?, &arguments.environment_id)?;
    let capabilities =
        CapabilitySource::capabilities(&session.provider, &resolved.environment.reference).await?;
    let data = MachineEnvironmentShowData {
        site: resolved.site,
        environment: resolved.environment,
        capabilities,
    };
    if context.output.is_machine() {
        output::write_machine_success("environment.show", &data, Vec::new())?;
    } else {
        render_environment(&data)?;
    }
    Ok(CommandOutcome::Success)
}

fn render_sites(sites: &[SiteSummary]) -> Result<()> {
    if sites.is_empty() {
        return output::write_human("No sites were returned by Kinsta.\n");
    }
    let mut contents = format!("Sites ({})\n\n", sites.len());
    for site in sites {
        let labels = if site.labels.is_empty() {
            "—".to_owned()
        } else {
            site.labels
                .iter()
                .map(|label| terminal_safe(&label.name))
                .collect::<Vec<_>>()
                .join(", ")
        };
        contents.push_str(&format!(
            "  {}\n    id: {}\n    domain: {}\n    labels: {}\n",
            terminal_safe(&site.display_name),
            terminal_safe(site.reference.site_id.as_str()),
            safe_optional(site.primary_domain.as_deref()),
            labels,
        ));
    }
    output::write_human(&contents)
}

fn render_environments(data: &MachineEnvironmentListData) -> Result<()> {
    let mut contents = format!(
        "{} environments ({})\n\n",
        terminal_safe(&data.site.display_name),
        data.environments.len()
    );
    for environment in &data.environments {
        contents.push_str(&format!(
            "  {}  {}\n    id: {}\n    domain: {}\n",
            terminal_safe(&environment.display_name),
            environment_kind_label(environment.kind),
            terminal_safe(environment.reference.environment_id.as_str()),
            safe_optional(environment.primary_domain.as_deref()),
        ));
    }
    output::write_human(&contents)
}

fn render_environment(data: &MachineEnvironmentShowData) -> Result<()> {
    let mut contents = format!(
        "{} / {}\n  environment id: {}\n  site id: {}\n  kind: {}\n  provider kind: {}\n  domain: {}\n  capabilities:\n",
        terminal_safe(&data.site.display_name),
        terminal_safe(&data.environment.display_name),
        terminal_safe(data.environment.reference.environment_id.as_str()),
        terminal_safe(data.environment.reference.site_id.as_str()),
        environment_kind_label(data.environment.kind),
        safe_optional(data.environment.provider_kind.as_deref()),
        safe_optional(data.environment.primary_domain.as_deref()),
    );
    for capability in &data.capabilities {
        contents.push_str(&format!(
            "    {} v{}: {}\n",
            terminal_safe(&capability.name),
            capability.version,
            if capability.available {
                "available"
            } else if capability.supported {
                "unavailable"
            } else {
                "unsupported"
            }
        ));
    }
    output::write_human(&contents)
}

fn safe_optional(value: Option<&str>) -> String {
    value.map_or_else(|| "—".to_owned(), terminal_safe)
}

const fn environment_kind_label(kind: EnvironmentKind) -> &'static str {
    match kind {
        EnvironmentKind::Production => "production",
        EnvironmentKind::Staging => "staging",
        EnvironmentKind::Development => "development",
        EnvironmentKind::Other => "other",
        _ => "other",
    }
}

async fn run_ssh(command: SshCommand, context: &Context) -> Result<CommandOutcome> {
    match command {
        SshCommand::Open(arguments) => open_ssh(arguments, context).await,
        SshCommand::Run(arguments) => run_remote_command(arguments, context).await,
    }
}

async fn open_ssh(arguments: SshOpenArgs, context: &Context) -> Result<CommandOutcome> {
    if context.output.is_machine() {
        return Err(AppError::new(
            ErrorCode::Unsupported,
            "interactive SSH cannot be represented as JSON",
        )
        .with_hint("Run `hostbraid ssh open` with human output in a terminal."));
    }
    if !context.interactive {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "interactive SSH requires an input terminal",
        )
        .with_hint("Run `hostbraid ssh open` in a terminal without `--no-input`."));
    }
    validate_opaque_selector(&arguments.environment_id)?;

    let session = kinsta_session(&arguments.selection)?;
    let reference = session.profile.reference();
    let snapshot = session.provider.catalog_snapshot(&reference).await?;
    let resolved = resolve_environment(&snapshot, &arguments.environment_id)?;
    let transport = OpenSsh::system();
    transport.check_available()?;
    write_pool_warning(&transport, !arguments.no_pool, context)?;
    let target = session
        .provider
        .ssh_target(&resolved.environment.reference)
        .await?;
    let status = transport.open_interactive(&target, !arguments.no_pool)?;
    Ok(CommandOutcome::Exit(process_status_code(status.code())))
}

async fn run_remote_command(arguments: SshRunArgs, context: &Context) -> Result<CommandOutcome> {
    validate_selection_syntax(&arguments)?;
    let syntactically_broad = arguments.all
        || !arguments.site_ids.is_empty()
        || !arguments.kind.is_empty()
        || !arguments.label.is_empty();
    if !context.interactive && syntactically_broad && !arguments.yes {
        return Err(AppError::new(
            ErrorCode::PolicyDenied,
            "broad SSH selectors require explicit non-interactive confirmation",
        )
        .with_hint("Resolve and review the target scope, then rerun with `--yes`."));
    }

    let session = kinsta_session(&arguments.selection)?;
    let reference = session.profile.reference();
    let spinner = context.spinner("Resolving Kinsta SSH targets");
    let snapshot_result = session.provider.catalog_snapshot(&reference).await;
    spinner.finish_and_clear();
    let snapshot = snapshot_result?;
    let selection = select_environments(&snapshot, &arguments)?;
    confirm_selection(&selection, &arguments, context)?;

    let transport = OpenSsh::system();
    transport.check_available()?;
    let warnings = pool_warnings(&transport, !arguments.no_pool);
    if !context.output.is_machine() {
        write_human_warnings(&warnings)?;
    }

    let spinner = context.spinner(format!(
        "Loading SSH coordinates for {} environment(s)",
        selection.targets.len()
    ));
    let mut prepared = ssh_targets(
        &session.provider,
        &selection.targets,
        usize::from(arguments.jobs),
    )
    .await;
    prepared.apply_fail_fast(arguments.fail_fast);
    spinner.finish_and_clear();
    let targets = &prepared.ready;

    if targets.len() == 1 && prepared.failures.iter().all(Option::is_none) && context.interactive {
        let outcome = transport.run_inherited(
            &targets[0].ssh,
            &arguments.remote_command,
            &RunOptions {
                timeout: arguments.timeout,
                pooling: !arguments.no_pool,
                ..RunOptions::default()
            },
        )?;
        return match outcome.state {
            ExecutionState::Succeeded | ExecutionState::Failed => {
                Ok(CommandOutcome::Exit(process_status_code(outcome.exit_code)))
            }
            ExecutionState::TimedOut => Err(AppError::new(
                ErrorCode::RemoteExecutionFailed,
                "the SSH command exceeded its timeout",
            )),
            ExecutionState::Cancelled => Err(AppError::new(
                ErrorCode::RemoteExecutionFailed,
                "the SSH command was cancelled",
            )),
            ExecutionState::Skipped => Err(AppError::new(
                ErrorCode::Internal,
                "a single SSH command was unexpectedly skipped",
            )),
        };
    }

    let mut signal_exit_code = None;
    let executed = if targets.is_empty() {
        None
    } else {
        let signals = ProcessSignalGuard::install()?;
        let report = transport.run_batch(
            targets,
            &arguments.remote_command,
            &BatchOptions {
                jobs: usize::from(arguments.jobs),
                timeout: arguments.timeout,
                fail_fast: arguments.fail_fast,
                pooling: !arguments.no_pool,
                cancellation: signals.cancellation(),
            },
        )?;
        if report
            .results
            .iter()
            .any(|result| result.state == ExecutionState::Cancelled)
        {
            signal_exit_code = signals.exit_code();
        }
        Some(report)
    };
    let report = prepared.merge(executed);
    let succeeded = report.succeeded();
    if context.output.is_machine() {
        let data = machine_ssh_run_data(&report);
        if succeeded {
            output::write_machine_success("ssh.run", &data, warnings)?;
            return Ok(CommandOutcome::Success);
        }
        let error = AppError::new(
            ErrorCode::RemoteExecutionFailed,
            "one or more remote commands failed",
        );
        output::write_machine_partial_failure("ssh.run", &error, &data, warnings)?;
        return Ok(CommandOutcome::Exit(signal_exit_code.unwrap_or(1)));
    }

    render_batch_report(&report, &selection)?;
    if succeeded {
        Ok(CommandOutcome::Success)
    } else {
        Ok(CommandOutcome::Exit(signal_exit_code.unwrap_or(1)))
    }
}

fn confirm_selection(
    selection: &EnvironmentSelection,
    arguments: &SshRunArgs,
    context: &Context,
) -> Result<()> {
    if !selection.broad {
        return Ok(());
    }
    if !context.output.is_machine() {
        let mut preview = format!("Selected {} environment(s):\n", selection.targets.len());
        for target in &selection.targets {
            preview.push_str(&format!(
                "  {} / {} ({})\n",
                terminal_safe(&target.site.display_name),
                terminal_safe(&target.environment.display_name),
                terminal_safe(target.environment.reference.environment_id.as_str()),
            ));
        }
        output::write_human_stderr(&preview)?;
    }
    if arguments.yes {
        return Ok(());
    }
    if context.confirm(&format!(
        "Run the remote command on {} environment(s)?",
        selection.targets.len()
    ))? {
        Ok(())
    } else {
        Err(AppError::new(
            ErrorCode::PolicyDenied,
            "remote command execution was not confirmed",
        ))
    }
}

struct PreparedSshTargets {
    ready: Vec<ExecutionTarget>,
    failures: Vec<Option<crate::ssh::TargetExecution>>,
}

impl PreparedSshTargets {
    fn apply_fail_fast(&mut self, enabled: bool) {
        let Some(first_failure) = enabled
            .then(|| self.failures.iter().position(Option::is_some))
            .flatten()
        else {
            return;
        };

        let mut ready = std::mem::take(&mut self.ready).into_iter();
        let mut retained = Vec::with_capacity(first_failure);
        for (index, failure) in self.failures.iter_mut().enumerate() {
            if index < first_failure && failure.is_none() {
                if let Some(target) = ready.next() {
                    retained.push(target);
                }
            } else if index > first_failure {
                let environment = if let Some(failure) = failure.as_ref() {
                    failure.environment.clone()
                } else {
                    ready
                        .next()
                        .expect("ready SSH target exists for every empty preparation slot")
                        .environment
                };
                *failure = Some(crate::ssh::TargetExecution::fail_fast_skipped(environment));
            }
        }
        debug_assert!(ready.next().is_none());
        self.ready = retained;
    }

    fn merge(self, executed: Option<BatchReport>) -> BatchReport {
        let mut executed = executed.map(|report| {
            (
                report.stream_capture_limit_bytes,
                VecDeque::from(report.results),
            )
        });
        let mut results = Vec::with_capacity(self.failures.len());
        for failure in self.failures {
            if let Some(failure) = failure {
                results.push(failure);
            } else if let Some((_, executed)) = executed.as_mut() {
                if let Some(result) = executed.pop_front() {
                    results.push(result);
                }
            }
        }

        if let Some((stream_capture_limit_bytes, remaining)) = executed {
            debug_assert!(remaining.is_empty());
            BatchReport {
                results,
                stream_capture_limit_bytes,
            }
        } else {
            BatchReport::from_results(results)
        }
    }
}

async fn ssh_targets(
    provider: &KinstaProvider,
    selected: &[ResolvedEnvironment],
    jobs: usize,
) -> PreparedSshTargets {
    let resolved = stream::iter(selected.iter().enumerate())
        .map(|(index, selected)| async move {
            let environment = selected.environment.reference.clone();
            let target = provider.ssh_target(&environment).await;
            (index, environment, target)
        })
        .buffer_unordered(jobs.max(1))
        .collect::<Vec<_>>()
        .await;

    let mut ordered = (0..selected.len()).map(|_| None).collect::<Vec<_>>();
    for (index, environment, target) in resolved {
        ordered[index] = Some((environment, target));
    }

    let mut ready = Vec::with_capacity(selected.len());
    let mut failures = Vec::with_capacity(selected.len());
    for result in ordered {
        let (environment, target) = result.expect("every SSH preparation future completes");
        match target {
            Ok(ssh) => {
                ready.push(ExecutionTarget { environment, ssh });
                failures.push(None);
            }
            Err(_) => failures.push(Some(crate::ssh::TargetExecution::target_unavailable(
                environment,
            ))),
        }
    }
    PreparedSshTargets { ready, failures }
}

fn pool_warnings(transport: &OpenSsh, pooling: bool) -> Vec<MachineWarning> {
    if !pooling {
        return Vec::new();
    }
    transport.pool_warning().map_or_else(Vec::new, |warning| {
        vec![MachineWarning::new(warning.code, warning.message)]
    })
}

fn write_pool_warning(transport: &OpenSsh, pooling: bool, context: &Context) -> Result<()> {
    if !context.output.is_machine() {
        write_human_warnings(&pool_warnings(transport, pooling))?;
    }
    Ok(())
}

fn process_status_code(code: Option<i32>) -> u8 {
    code.and_then(|code| u8::try_from(code).ok()).unwrap_or(1)
}

fn machine_ssh_run_data(report: &BatchReport) -> MachineSshRunData {
    let results = report
        .results
        .iter()
        .map(|result| MachineSshTargetResult {
            environment: result.environment.clone(),
            state: match result.state {
                ExecutionState::Succeeded => MachineSshExecutionState::Succeeded,
                ExecutionState::Failed => MachineSshExecutionState::Failed,
                ExecutionState::TimedOut => MachineSshExecutionState::TimedOut,
                ExecutionState::Cancelled => MachineSshExecutionState::Cancelled,
                ExecutionState::Skipped => MachineSshExecutionState::Skipped,
            },
            exit_code: result.exit_code,
            duration_ms: result.duration_ms,
            stdout: machine_captured_stream(&result.stdout),
            stderr: machine_captured_stream(&result.stderr),
            failure: result.failure.as_ref().map(|failure| MachineSshFailure {
                code: match failure.code {
                    ExecutionFailureCode::TargetUnavailable => {
                        MachineSshFailureCode::TargetUnavailable
                    }
                    ExecutionFailureCode::InvalidTarget => MachineSshFailureCode::InvalidTarget,
                    ExecutionFailureCode::SpawnFailed => MachineSshFailureCode::SpawnFailed,
                    ExecutionFailureCode::WaitFailed => MachineSshFailureCode::WaitFailed,
                    ExecutionFailureCode::CaptureFailed => MachineSshFailureCode::CaptureFailed,
                    ExecutionFailureCode::RemoteExit => MachineSshFailureCode::RemoteExit,
                    ExecutionFailureCode::TimedOut => MachineSshFailureCode::TimedOut,
                    ExecutionFailureCode::Cancelled => MachineSshFailureCode::Cancelled,
                    ExecutionFailureCode::FailFast => MachineSshFailureCode::FailFast,
                },
                message: failure.message.clone(),
            }),
        })
        .collect();
    MachineSshRunData {
        results,
        stream_capture_limit_bytes: report.stream_capture_limit_bytes,
    }
}

fn machine_captured_stream(stream: &crate::ssh::CapturedStream) -> MachineSshCapturedStream {
    MachineSshCapturedStream {
        encoding: match stream.encoding {
            CaptureEncoding::Text => MachineSshCaptureEncoding::Text,
            CaptureEncoding::Base64 => MachineSshCaptureEncoding::Base64,
        },
        data: stream.data.clone(),
        truncated: stream.truncated,
        captured_bytes: stream.captured_bytes,
    }
}

fn render_batch_report(report: &BatchReport, selection: &EnvironmentSelection) -> Result<()> {
    for result in &report.results {
        let mut contents = String::new();
        let selected = selection
            .targets
            .iter()
            .find(|target| target.environment.reference == result.environment);
        let heading = selected.map_or_else(
            || result.environment.environment_id.as_str().to_owned(),
            |target| {
                format!(
                    "{} / {}",
                    terminal_safe(&target.site.display_name),
                    terminal_safe(&target.environment.display_name)
                )
            },
        );
        contents.push_str(&format!(
            "== {} ({}) [{}{}] ==\n",
            heading,
            terminal_safe(result.environment.environment_id.as_str()),
            execution_state_label(result.state),
            result
                .exit_code
                .map_or_else(String::new, |code| format!(", exit {code}")),
        ));
        append_captured_stream(&mut contents, "stdout", &result.stdout);
        append_captured_stream(&mut contents, "stderr", &result.stderr);
        if let Some(failure) = &result.failure {
            contents.push_str(&format!("failure: {}\n", terminal_safe(&failure.message)));
        }
        contents.push('\n');
        output::write_human(&contents)?;
    }
    Ok(())
}

fn append_captured_stream(contents: &mut String, label: &str, stream: &crate::ssh::CapturedStream) {
    if stream.captured_bytes == 0 && !stream.truncated {
        return;
    }
    contents.push_str(label);
    if stream.encoding == CaptureEncoding::Base64 {
        contents.push_str(" (base64)");
    }
    if stream.truncated {
        contents.push_str(" (truncated)");
    }
    contents.push_str(":\n");
    let data = if stream.encoding == CaptureEncoding::Text {
        terminal_output_safe(&stream.data)
    } else {
        stream.data.clone()
    };
    contents.push_str(&data);
    if !data.ends_with('\n') {
        contents.push('\n');
    }
}

const fn execution_state_label(state: ExecutionState) -> &'static str {
    match state {
        ExecutionState::Succeeded => "succeeded",
        ExecutionState::Failed => "failed",
        ExecutionState::TimedOut => "timed out",
        ExecutionState::Cancelled => "cancelled",
        ExecutionState::Skipped => "skipped",
    }
}

async fn run_inventory(command: InventoryCommand, context: &Context) -> Result<CommandOutcome> {
    let (arguments, plugins) = match command {
        InventoryCommand::Plugins(arguments) => (arguments, true),
        InventoryCommand::Themes(arguments) => (arguments, false),
    };
    validate_inventory_arguments(&arguments)?;
    let session = kinsta_session(&arguments.selection)?;
    let reference = session.profile.reference();
    let spinner = context.spinner(if plugins {
        "Loading Kinsta plugin inventory"
    } else {
        "Loading Kinsta theme inventory"
    });
    let inventory_result = if plugins {
        session.provider.plugin_inventory(&reference).await
    } else {
        session.provider.theme_inventory(&reference).await
    };
    spinner.finish_and_clear();
    let inventory = filter_inventory(inventory_result?, &arguments)?;
    let command = if plugins {
        "inventory.plugins"
    } else {
        "inventory.themes"
    };
    if context.output.is_machine() {
        output::write_machine_success(command, &inventory, Vec::new())?;
    } else {
        render_inventory(&inventory, arguments.details)?;
    }
    Ok(CommandOutcome::Success)
}

fn filter_inventory(
    inventory: WordPressComponentInventory,
    arguments: &InventoryListArgs,
) -> Result<MachineInventoryData> {
    let search = arguments
        .search
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_lowercase);
    if arguments.search.is_some() && search.is_none() {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "inventory search text cannot be empty",
        ));
    }
    let mut components = inventory.components;
    components.retain(|component| {
        let search_matches = search.as_ref().is_none_or(|search| {
            component.slug.to_lowercase().contains(search)
                || component.title.to_lowercase().contains(search)
        });
        let update_matches = !arguments.updates || component.update_count > 0;
        let vulnerability_matches = !arguments.vulnerable
            || component
                .installations
                .iter()
                .any(|installation| installation.installed_version_vulnerable);
        search_matches && update_matches && vulnerability_matches
    });
    Ok(MachineInventoryData {
        kind: inventory.kind,
        provider_total: inventory.total,
        matched_count: components.len(),
        refreshed_at: inventory.refreshed_at,
        components,
    })
}

fn validate_inventory_arguments(arguments: &InventoryListArgs) -> Result<()> {
    if arguments
        .search
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "inventory search text cannot be empty",
        ));
    }
    Ok(())
}

fn render_inventory(inventory: &MachineInventoryData, details: bool) -> Result<()> {
    let noun = match inventory.kind {
        WordPressComponentKind::Plugin => "plugins",
        WordPressComponentKind::Theme => "themes",
        _ => "components",
    };
    let mut contents = format!(
        "WordPress {noun}: {} matched ({} provider total)\n",
        inventory.matched_count, inventory.provider_total
    );
    if let Some(refreshed_at) = &inventory.refreshed_at {
        contents.push_str(&format!("Refreshed: {}\n", terminal_safe(refreshed_at)));
    }
    contents.push('\n');
    for component in &inventory.components {
        let vulnerable = component
            .installations
            .iter()
            .any(|installation| installation.installed_version_vulnerable);
        contents.push_str(&format!(
            "  {} ({})\n    environments: {}  updates: {}  vulnerable installs: {}\n",
            terminal_safe(&component.title),
            terminal_safe(&component.slug),
            component.environment_count,
            component.update_count,
            if vulnerable { "yes" } else { "no" },
        ));
        if details {
            for installation in &component.installations {
                contents.push_str(&format!(
                    "      {}  version={}  status={}  update={}{}\n",
                    terminal_safe(installation.environment.environment_id.as_str()),
                    terminal_safe(&installation.installed_version),
                    terminal_safe(&installation.status),
                    safe_optional(installation.available_version.as_deref()),
                    if installation.installed_version_vulnerable {
                        "  vulnerable"
                    } else {
                        ""
                    },
                ));
            }
        }
    }
    output::write_human(&contents)
}

#[cfg(test)]
mod tests {
    use super::{PreparedSshTargets, filter_inventory};
    use crate::cli::{InventoryListArgs, ProfileSelectionArgs};
    use crate::ssh::{
        BatchReport, ExecutionFailureCode, ExecutionState, ExecutionTarget, TargetExecution,
    };
    use hostbraid_core::{
        EnvironmentRef, SshTarget, WordPressComponent, WordPressComponentInstallation,
        WordPressComponentInventory, WordPressComponentKind,
    };

    fn component(slug: &str, updates: u64, vulnerable: bool) -> WordPressComponent {
        WordPressComponent {
            slug: slug.to_owned(),
            title: format!("Title {slug}"),
            description: None,
            latest_version: None,
            latest_version_vulnerable: false,
            environment_count: 1,
            update_count: updates,
            installations: vec![WordPressComponentInstallation {
                environment: EnvironmentRef::try_new("kinsta", "agency", "site", "env")
                    .expect("environment"),
                status: "active".to_owned(),
                installed_version: "1.0".to_owned(),
                installed_version_vulnerable: vulnerable,
                update_state: None,
                available_version: None,
                available_version_vulnerable: false,
                update_status: None,
                auto_update_type: None,
            }],
        }
    }

    #[test]
    fn inventory_filters_are_anded() {
        let inventory = WordPressComponentInventory {
            kind: WordPressComponentKind::Plugin,
            total: 3,
            refreshed_at: None,
            components: vec![
                component("keep-me", 1, true),
                component("no-update", 0, true),
                component("not-vulnerable", 1, false),
            ],
        };
        let arguments = InventoryListArgs {
            selection: ProfileSelectionArgs { profile: None },
            search: Some("KEEP".to_owned()),
            updates: true,
            vulnerable: true,
            details: false,
        };

        let filtered = filter_inventory(inventory, &arguments).expect("filter inventory");

        assert_eq!(filtered.provider_total, 3);
        assert_eq!(filtered.matched_count, 1);
        assert_eq!(filtered.components[0].slug, "keep-me");
    }

    #[test]
    fn inventory_search_is_unicode_case_insensitive() {
        let mut matching = component("coffee", 0, false);
        matching.title = "Éclair Tools".to_owned();
        let inventory = WordPressComponentInventory {
            kind: WordPressComponentKind::Plugin,
            total: 1,
            refreshed_at: None,
            components: vec![matching],
        };
        let arguments = InventoryListArgs {
            selection: ProfileSelectionArgs { profile: None },
            search: Some("éCLAIR".to_owned()),
            updates: false,
            vulnerable: false,
            details: false,
        };

        let filtered = filter_inventory(inventory, &arguments).expect("filter inventory");

        assert_eq!(filtered.matched_count, 1);
        assert_eq!(filtered.components[0].title, "Éclair Tools");
    }

    fn execution_target(id: &str) -> ExecutionTarget {
        ExecutionTarget {
            environment: EnvironmentRef::try_new("kinsta", "agency", "site", id)
                .expect("environment"),
            ssh: SshTarget::try_new("ssh.example", 22, "user", None).expect("SSH target"),
        }
    }

    #[test]
    fn preflight_failures_merge_in_selection_order_and_honor_fail_fast() {
        let first = execution_target("env-1");
        let second_environment =
            EnvironmentRef::try_new("kinsta", "agency", "site", "env-2").expect("environment");
        let third = execution_target("env-3");
        let mut prepared = PreparedSshTargets {
            ready: vec![first.clone(), third],
            failures: vec![
                None,
                Some(TargetExecution::target_unavailable(
                    second_environment.clone(),
                )),
                None,
            ],
        };

        prepared.apply_fail_fast(true);
        assert_eq!(prepared.ready, vec![first.clone()]);
        assert_eq!(
            prepared.failures[2].as_ref().map(|result| result.state),
            Some(ExecutionState::Skipped)
        );

        let report = prepared.merge(Some(BatchReport::from_results(vec![
            TargetExecution::target_unavailable(first.environment),
        ])));
        assert_eq!(report.results.len(), 3);
        assert_eq!(
            report
                .results
                .iter()
                .map(|result| result.environment.environment_id.as_str())
                .collect::<Vec<_>>(),
            vec!["env-1", "env-2", "env-3"]
        );
        assert_eq!(
            report.results[1]
                .failure
                .as_ref()
                .map(|failure| failure.code),
            Some(ExecutionFailureCode::TargetUnavailable)
        );
        assert_eq!(
            report.results[2]
                .failure
                .as_ref()
                .map(|failure| failure.code),
            Some(ExecutionFailureCode::FailFast)
        );
    }
}
