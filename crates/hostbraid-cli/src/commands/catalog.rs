use crate::cli::{EnvironmentKindArg, SshRunArgs};
use hostbraid_core::{
    AppError, CatalogSite, CatalogSnapshot, EnvironmentKind, EnvironmentSummary, ErrorCode,
    OpaqueId, Result, SiteSummary,
};
use std::collections::HashSet;

#[derive(Debug, Clone)]
pub(crate) struct ResolvedEnvironment {
    pub site: SiteSummary,
    pub environment: EnvironmentSummary,
}

#[derive(Debug)]
pub(crate) struct EnvironmentSelection {
    pub targets: Vec<ResolvedEnvironment>,
    pub broad: bool,
}

pub(crate) fn resolve_site(snapshot: &CatalogSnapshot, site_id: &str) -> Result<CatalogSite> {
    validate_opaque_selector(site_id)?;
    let matches: Vec<&CatalogSite> = snapshot
        .sites
        .iter()
        .filter(|site| site.site.reference.site_id.as_str() == site_id)
        .collect();
    match matches.as_slice() {
        [] => Err(AppError::new(ErrorCode::NotFound, "site ID was not found")),
        [site] => Ok((*site).clone()),
        _ => Err(AppError::new(
            ErrorCode::AmbiguousTarget,
            "site ID matched more than one site",
        )),
    }
}

pub(crate) fn resolve_environment(
    snapshot: &CatalogSnapshot,
    environment_id: &str,
) -> Result<ResolvedEnvironment> {
    validate_opaque_selector(environment_id)?;
    let matches: Vec<ResolvedEnvironment> = all_environments(snapshot)
        .filter(|target| target.environment.reference.environment_id.as_str() == environment_id)
        .collect();
    match matches.as_slice() {
        [] => Err(AppError::new(
            ErrorCode::NotFound,
            "environment ID was not found",
        )),
        [environment] => Ok(environment.clone()),
        _ => Err(AppError::new(
            ErrorCode::AmbiguousTarget,
            "environment ID matched more than one environment",
        )),
    }
}

pub(crate) fn select_environments(
    snapshot: &CatalogSnapshot,
    arguments: &SshRunArgs,
) -> Result<EnvironmentSelection> {
    validate_selection_syntax(arguments)?;

    validate_requested_environment_ids(snapshot, &arguments.environment_ids)?;
    validate_requested_site_ids(snapshot, &arguments.site_ids)?;
    validate_requested_labels(snapshot, &arguments.label)?;

    let environment_ids: HashSet<&str> = arguments
        .environment_ids
        .iter()
        .map(String::as_str)
        .collect();
    let site_ids: HashSet<&str> = arguments.site_ids.iter().map(String::as_str).collect();
    let labels: HashSet<&str> = arguments.label.iter().map(String::as_str).collect();
    let kinds: Vec<EnvironmentKind> = arguments.kind.iter().copied().map(kind).collect();

    let mut targets = Vec::new();
    for site in &snapshot.sites {
        let site_matches =
            site_ids.is_empty() || site_ids.contains(site.site.reference.site_id.as_str());
        let label_matches = labels.is_empty()
            || site
                .site
                .labels
                .iter()
                .any(|label| labels.contains(label.name.as_str()));
        if !site_matches || !label_matches {
            continue;
        }

        for environment in &site.environments {
            let environment_matches = environment_ids.is_empty()
                || environment_ids.contains(environment.reference.environment_id.as_str());
            let kind_matches = kinds.is_empty() || kinds.contains(&environment.kind);
            if environment_matches && kind_matches {
                targets.push(ResolvedEnvironment {
                    site: site.site.clone(),
                    environment: environment.clone(),
                });
            }
        }
    }

    if targets.is_empty() {
        return Err(AppError::new(
            ErrorCode::NotFound,
            "no environments matched every selector category",
        )
        .with_hint("Repeated selectors are ORed; different selector categories are ANDed."));
    }

    Ok(EnvironmentSelection {
        targets,
        broad: arguments.all
            || !arguments.site_ids.is_empty()
            || !arguments.kind.is_empty()
            || !arguments.label.is_empty(),
    })
}

pub(crate) fn validate_selection_syntax(arguments: &SshRunArgs) -> Result<()> {
    let has_selector = arguments.all
        || !arguments.environment_ids.is_empty()
        || !arguments.site_ids.is_empty()
        || !arguments.kind.is_empty()
        || !arguments.label.is_empty();
    if !has_selector {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "at least one SSH target selector is required",
        )
        .with_hint(
            "Use --environment-id, --site-id, --kind, --label, or the deliberate --all selector.",
        ));
    }

    for id in &arguments.environment_ids {
        validate_opaque_selector(id)?;
    }
    for id in &arguments.site_ids {
        validate_opaque_selector(id)?;
    }
    for label in &arguments.label {
        validate_label_selector(label)?;
    }
    Ok(())
}

fn validate_requested_environment_ids(
    snapshot: &CatalogSnapshot,
    requested: &[String],
) -> Result<()> {
    for id in requested.iter().map(String::as_str).collect::<HashSet<_>>() {
        let _ = resolve_environment(snapshot, id)?;
    }
    Ok(())
}

fn validate_requested_site_ids(snapshot: &CatalogSnapshot, requested: &[String]) -> Result<()> {
    for id in requested.iter().map(String::as_str).collect::<HashSet<_>>() {
        let _ = resolve_site(snapshot, id)?;
    }
    Ok(())
}

fn validate_requested_labels(snapshot: &CatalogSnapshot, requested: &[String]) -> Result<()> {
    for requested_label in requested.iter().map(String::as_str).collect::<HashSet<_>>() {
        validate_label_selector(requested_label)?;
        if !snapshot.sites.iter().any(|site| {
            site.site
                .labels
                .iter()
                .any(|label| label.name == requested_label)
        }) {
            return Err(AppError::new(
                ErrorCode::NotFound,
                "site label was not found",
            ));
        }
    }
    Ok(())
}

fn validate_label_selector(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 512
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "site label selector is empty or malformed",
        ));
    }
    Ok(())
}

pub(crate) fn validate_opaque_selector(value: &str) -> Result<()> {
    OpaqueId::new(value).map(|_| ()).map_err(|_| {
        AppError::new(
            ErrorCode::InvalidInput,
            "provider ID selector is empty or malformed",
        )
    })
}

fn all_environments(snapshot: &CatalogSnapshot) -> impl Iterator<Item = ResolvedEnvironment> + '_ {
    snapshot.sites.iter().flat_map(|site| {
        site.environments
            .iter()
            .map(|environment| ResolvedEnvironment {
                site: site.site.clone(),
                environment: environment.clone(),
            })
    })
}

const fn kind(value: EnvironmentKindArg) -> EnvironmentKind {
    match value {
        EnvironmentKindArg::Production => EnvironmentKind::Production,
        EnvironmentKindArg::Staging => EnvironmentKind::Staging,
        EnvironmentKindArg::Development => EnvironmentKind::Development,
        EnvironmentKindArg::Other => EnvironmentKind::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::{resolve_environment, select_environments, validate_selection_syntax};
    use crate::cli::{EnvironmentKindArg, ProfileSelectionArgs, SshRunArgs};
    use hostbraid_core::{
        CatalogSite, CatalogSnapshot, EnvironmentKind, EnvironmentRef, EnvironmentSummary,
        OpaqueId, ProviderProfileRef, SiteLabel, SiteRef, SiteSummary,
    };
    use std::ffi::OsString;

    fn snapshot() -> CatalogSnapshot {
        let profile = ProviderProfileRef::try_new("kinsta", "agency").expect("profile");
        CatalogSnapshot {
            profile,
            sites: vec![
                site("site-1", "Alpha", &["customer-a"], &["env-1", "env-2"]),
                site("site-2", "Beta", &["customer-b"], &["env-3"]),
            ],
        }
    }

    fn site(id: &str, name: &str, labels: &[&str], environments: &[&str]) -> CatalogSite {
        let site_ref = SiteRef::try_new("kinsta", "agency", id).expect("site ref");
        CatalogSite {
            site: SiteSummary {
                reference: site_ref.clone(),
                display_name: name.to_owned(),
                primary_domain: None,
                labels: labels
                    .iter()
                    .map(|label| SiteLabel {
                        id: OpaqueId::new(format!("label-{label}")).expect("label id"),
                        name: (*label).to_owned(),
                    })
                    .collect(),
            },
            environments: environments
                .iter()
                .enumerate()
                .map(|(index, id)| EnvironmentSummary {
                    reference: EnvironmentRef::try_new(
                        "kinsta",
                        "agency",
                        site_ref.site_id.as_str(),
                        *id,
                    )
                    .expect("environment ref"),
                    display_name: (*id).to_owned(),
                    kind: if index == 0 {
                        EnvironmentKind::Production
                    } else {
                        EnvironmentKind::Staging
                    },
                    provider_kind: None,
                    primary_domain: None,
                })
                .collect(),
        }
    }

    fn arguments() -> SshRunArgs {
        SshRunArgs {
            selection: ProfileSelectionArgs { profile: None },
            environment_ids: Vec::new(),
            site_ids: Vec::new(),
            kind: Vec::new(),
            label: Vec::new(),
            all: false,
            jobs: 8,
            timeout: None,
            fail_fast: false,
            yes: false,
            no_pool: false,
            remote_command: vec![OsString::from("true")],
        }
    }

    #[test]
    fn exact_environment_ids_need_no_broad_confirmation() {
        let mut arguments = arguments();
        arguments.environment_ids = vec!["env-1".to_owned(), "env-3".to_owned()];

        let selection = select_environments(&snapshot(), &arguments).expect("selection");

        assert!(!selection.broad);
        assert_eq!(selection.targets.len(), 2);
    }

    #[test]
    fn categories_are_anded_and_repeats_are_ored() {
        let mut arguments = arguments();
        arguments.site_ids = vec!["site-1".to_owned(), "site-2".to_owned()];
        arguments.kind = vec![EnvironmentKindArg::Production];
        arguments.label = vec!["customer-a".to_owned(), "customer-b".to_owned()];

        let selection = select_environments(&snapshot(), &arguments).expect("selection");

        assert!(selection.broad);
        assert_eq!(selection.targets.len(), 2);
        assert!(
            selection
                .targets
                .iter()
                .all(|target| target.environment.kind == EnvironmentKind::Production)
        );
    }

    #[test]
    fn unknown_exact_ids_and_labels_are_rejected() {
        assert!(resolve_environment(&snapshot(), "missing").is_err());

        let mut arguments = arguments();
        arguments.label = vec!["missing".to_owned()];
        assert!(select_environments(&snapshot(), &arguments).is_err());
    }

    #[test]
    fn missing_or_malformed_selectors_fail_without_a_catalog() {
        let mut missing = arguments();
        let error = validate_selection_syntax(&missing).expect_err("selector is required");
        assert_eq!(
            error.message(),
            "at least one SSH target selector is required"
        );

        missing.environment_ids = vec![" leading-space".to_owned()];
        let error = validate_selection_syntax(&missing).expect_err("selector is malformed");
        assert_eq!(
            error.message(),
            "provider ID selector is empty or malformed"
        );
    }
}
