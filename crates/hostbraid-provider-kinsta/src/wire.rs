use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize)]
pub(crate) struct ValidateResponse {
    pub(crate) name: String,
    pub(crate) expires_at: Option<String>,
    pub(crate) company: String,
    pub(crate) status: String,
}

#[derive(Deserialize)]
pub(crate) struct SitesResponse {
    pub(crate) company: SitesCompany,
}

#[derive(Deserialize)]
pub(crate) struct SitesCompany {
    pub(crate) sites: Vec<Site>,
}

#[derive(Deserialize)]
pub(crate) struct Site {
    pub(crate) id: String,
    pub(crate) display_name: String,
    #[serde(default, rename = "siteLabels")]
    pub(crate) labels: Vec<SiteLabel>,
    #[serde(default)]
    pub(crate) environments: Vec<Environment>,
}

#[derive(Deserialize)]
pub(crate) struct SiteLabel {
    pub(crate) id: String,
    pub(crate) name: String,
}

#[derive(Deserialize)]
pub(crate) struct Environment {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) display_name: String,
    #[serde(default, rename = "primaryDomain")]
    pub(crate) primary_domain: Option<Domain>,
}

#[derive(Deserialize)]
pub(crate) struct Domain {
    pub(crate) name: String,
}

#[derive(Deserialize)]
pub(crate) struct SshStatusResponse {
    pub(crate) environment: SshStatusEnvironment,
}

#[derive(Deserialize)]
pub(crate) struct SshStatusEnvironment {
    pub(crate) active_container: SshStatusContainer,
}

#[derive(Deserialize)]
pub(crate) struct SshStatusContainer {
    pub(crate) is_ssh_enabled: bool,
}

#[derive(Deserialize)]
pub(crate) struct SshConfigResponse {
    pub(crate) site: SshConfigSite,
    pub(crate) port: String,
    pub(crate) host: String,
    pub(crate) user: String,
}

#[derive(Deserialize)]
pub(crate) struct SshConfigSite {
    pub(crate) id: String,
    pub(crate) environment: SshConfigEnvironment,
}

#[derive(Deserialize)]
pub(crate) struct SshConfigEnvironment {
    pub(crate) id: String,
}

#[derive(Serialize)]
pub(crate) struct InventoryRequest {
    pub(crate) offset: u64,
    pub(crate) limit: u64,
    pub(crate) order_by: InventoryOrderBy,
}

#[derive(Serialize)]
pub(crate) struct InventoryOrderBy {
    pub(crate) field: &'static str,
    pub(crate) order: &'static str,
}

#[derive(Clone, Deserialize)]
pub(crate) struct InventoryResponse {
    pub(crate) company: InventoryCompany,
}

#[derive(Clone, Deserialize)]
pub(crate) struct InventoryCompany {
    #[serde(alias = "plugins", alias = "themes")]
    pub(crate) inventory: InventoryPage,
}

#[derive(Clone, Deserialize)]
pub(crate) struct InventoryPage {
    pub(crate) total: u64,
    pub(crate) last_updated_at: Option<String>,
    pub(crate) items: Vec<InventoryItem>,
}

#[derive(Clone, Deserialize)]
pub(crate) struct InventoryItem {
    pub(crate) name: String,
    pub(crate) title: String,
    pub(crate) description: Option<String>,
    pub(crate) latest_version: Option<String>,
    pub(crate) is_latest_version_vulnerable: bool,
    pub(crate) environment_count: u64,
    pub(crate) update_count: u64,
    pub(crate) environments: Vec<InventoryInstallation>,
}

#[derive(Clone, Deserialize)]
pub(crate) struct InventoryInstallation {
    pub(crate) id: String,
    #[serde(alias = "plugin_status", alias = "theme_status")]
    pub(crate) status: String,
    #[serde(alias = "plugin_update", alias = "theme_update")]
    pub(crate) update: Option<String>,
    #[serde(alias = "plugin_version", alias = "theme_version")]
    pub(crate) version: String,
    #[serde(
        alias = "is_plugin_version_vulnerable",
        alias = "is_theme_version_vulnerable"
    )]
    pub(crate) is_version_vulnerable: bool,
    #[serde(alias = "plugin_update_version", alias = "theme_update_version")]
    pub(crate) update_version: Option<String>,
    #[serde(
        alias = "is_plugin_update_version_vulnerable",
        alias = "is_theme_update_version_vulnerable"
    )]
    pub(crate) is_update_version_vulnerable: bool,
    #[serde(alias = "plugin_update_status", alias = "theme_update_status")]
    pub(crate) update_status: Option<String>,
    pub(crate) auto_update_type: Option<String>,
}
