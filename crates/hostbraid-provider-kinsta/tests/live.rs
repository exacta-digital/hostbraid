use hostbraid_core::{CapabilitySource, EnvironmentKind, ProviderProfileRef};
use hostbraid_provider_kinsta::KinstaProvider;

/// Read-only release validation against an explicitly supplied restricted Kinsta account.
///
/// This test is ignored by the normal handoff suite. It never prints the credential or provider
/// payloads, and it performs no remote command or provider mutation.
#[tokio::test]
#[ignore = "requires HOSTBRAID_KINSTA_TEST_TOKEN for a restricted test company"]
async fn restricted_account_read_only_vertical_slice() {
    let token = std::env::var("HOSTBRAID_KINSTA_TEST_TOKEN")
        .expect("HOSTBRAID_KINSTA_TEST_TOKEN must be set for the ignored live test");
    let (provider, validation) = KinstaProvider::authenticate(token)
        .await
        .expect("restricted Kinsta credential validates");
    let profile = ProviderProfileRef::try_new("kinsta", "live-validation")
        .expect("live validation profile reference is valid");

    let catalog = provider
        .catalog_snapshot(&profile)
        .await
        .expect("restricted account catalog loads");
    assert_eq!(catalog.profile, profile);
    assert_eq!(provider.company_id(), &validation.company_id);
    for site in &catalog.sites {
        assert_eq!(site.site.reference.profile_ref(), profile);
        for environment in &site.environments {
            assert_eq!(environment.reference.site_ref(), site.site.reference);
            assert!(matches!(
                environment.kind,
                EnvironmentKind::Production
                    | EnvironmentKind::Staging
                    | EnvironmentKind::Development
                    | EnvironmentKind::Other
            ));
        }
    }

    if let Some(environment) = catalog
        .sites
        .iter()
        .flat_map(|site| &site.environments)
        .next()
    {
        let capabilities = CapabilitySource::capabilities(&provider, &environment.reference)
            .await
            .expect("environment capabilities load");
        assert!(
            capabilities
                .iter()
                .any(|capability| capability.name == "connection.ssh")
        );
    }

    let plugins = provider
        .plugin_inventory(&profile)
        .await
        .expect("company plugin inventory loads");
    let themes = provider
        .theme_inventory(&profile)
        .await
        .expect("company theme inventory loads");
    assert_eq!(plugins.total as usize, plugins.components.len());
    assert_eq!(themes.total as usize, themes.components.len());
}
