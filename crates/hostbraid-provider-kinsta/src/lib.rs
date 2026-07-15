//! Kinsta's built-in HostBraid provider adapter.
//!
//! Wire response types stay private to this crate. Public methods return provider-neutral core
//! values, and HTTP failures are mapped without retaining response bodies, request URLs, or tokens.

mod wire;

use async_trait::async_trait;
use hostbraid_core::{
    AppError, Capability, CapabilitySource, Catalog, CatalogSite, CatalogSnapshot, EnvironmentKind,
    EnvironmentRef, EnvironmentSummary, ErrorCode, OpaqueId, ProviderDescriptor, ProviderId,
    ProviderIdentity, ProviderProfileRef, Result, SiteLabel, SiteRef, SiteSummary, SshAccess,
    SshTarget, WordPressComponent, WordPressComponentInstallation, WordPressComponentInventory,
    WordPressComponentKind, WordPressInventory,
};
use reqwest::{Client, RequestBuilder, StatusCode, Url};
use serde::de::DeserializeOwned;
use std::{
    collections::{HashMap, HashSet},
    fmt,
    time::Duration,
};
use zeroize::Zeroizing;

use crate::wire::{
    InventoryItem, InventoryOrderBy, InventoryRequest, InventoryResponse, SitesResponse,
    SshConfigResponse, SshStatusResponse, ValidateResponse,
};

const PRODUCTION_BASE_URL: &str = "https://api.kinsta.com/v2";
const MAX_HTTP_RESPONSE_BODY_BYTES: usize = 16 * 1024 * 1024;
const INVENTORY_PAGE_SIZE: u64 = 100;
const MAX_INVENTORY_ITEMS: u64 = 100_000;
const MAX_INVENTORY_PAGES: u64 = 1_000;
const MAX_INVENTORY_BODY_BYTES: usize = 128 * 1024 * 1024;
const INVENTORY_PAGING_TIMEOUT: Duration = Duration::from_secs(120);
const SSH_CAPABILITY: &str = "connection.ssh";

#[derive(Clone, Copy)]
struct InventoryLimits {
    max_items: u64,
    max_pages: u64,
    max_body_bytes: usize,
}

const INVENTORY_LIMITS: InventoryLimits = InventoryLimits {
    max_items: MAX_INVENTORY_ITEMS,
    max_pages: MAX_INVENTORY_PAGES,
    max_body_bytes: MAX_INVENTORY_BODY_BYTES,
};

/// Secret-free API-key metadata returned by Kinsta's validation endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiKeyValidation {
    pub key_name: String,
    pub expires_at: Option<String>,
    pub company_id: OpaqueId,
    pub status: String,
}

struct ApiToken(Zeroizing<String>);

impl ApiToken {
    fn new(value: impl Into<String>) -> Result<Self> {
        let value = Zeroizing::new(value.into());
        if value.is_empty() || value.len() > 16 * 1024 || value.chars().any(char::is_control) {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Kinsta API token is empty or invalid",
            ));
        }
        Ok(Self(value))
    }

    fn expose(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Debug for ApiToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

struct Transport {
    http: Client,
    base_url: Url,
    token: ApiToken,
}

impl fmt::Debug for Transport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Transport")
            .field("base_url", &self.base_url)
            .field("token", &self.token)
            .finish_non_exhaustive()
    }
}

impl Transport {
    fn production(token: impl Into<String>) -> Result<Self> {
        let base_url = Url::parse(PRODUCTION_BASE_URL).map_err(|_| {
            AppError::new(
                ErrorCode::Internal,
                "Kinsta API endpoint configuration is invalid",
            )
        })?;
        Self::new(token, base_url, build_http_client()?)
    }

    fn new(token: impl Into<String>, base_url: Url, http: Client) -> Result<Self> {
        if base_url.cannot_be_a_base() {
            return Err(AppError::new(
                ErrorCode::Internal,
                "Kinsta API endpoint configuration is invalid",
            ));
        }
        Ok(Self {
            http,
            base_url,
            token: ApiToken::new(token)?,
        })
    }

    fn endpoint(&self, segments: &[&str]) -> Result<Url> {
        let mut url = self.base_url.clone();
        let mut path = url.path_segments_mut().map_err(|()| {
            AppError::new(
                ErrorCode::Internal,
                "Kinsta API endpoint configuration is invalid",
            )
        })?;
        path.pop_if_empty();
        path.extend(segments.iter().copied());
        drop(path);
        Ok(url)
    }

    fn authorized_get(&self, url: Url) -> RequestBuilder {
        self.http.get(url).bearer_auth(self.token.expose())
    }

    async fn response_json<T>(&self, request: RequestBuilder) -> Result<T>
    where
        T: DeserializeOwned,
    {
        Ok(self
            .response_json_with_limit(request, MAX_HTTP_RESPONSE_BODY_BYTES)
            .await?
            .value)
    }

    async fn response_json_with_limit<T>(
        &self,
        request: RequestBuilder,
        max_body_bytes: usize,
    ) -> Result<BoundedJson<T>>
    where
        T: DeserializeOwned,
    {
        let mut response = request.send().await.map_err(|_| transport_error())?;

        if !response.status().is_success() {
            return Err(map_http_status(response.status()));
        }

        if response
            .content_length()
            .is_some_and(|length| length > max_body_bytes as u64)
        {
            return Err(provider_contract_error());
        }

        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| transport_error())? {
            let next_len = body
                .len()
                .checked_add(chunk.len())
                .filter(|length| *length <= max_body_bytes)
                .ok_or_else(provider_contract_error)?;
            body.reserve(next_len - body.len());
            body.extend_from_slice(&chunk);
        }

        let body_len = body.len();
        let value = serde_json::from_slice(&body).map_err(|_| provider_contract_error())?;
        Ok(BoundedJson { value, body_len })
    }

    async fn validate_api_key(&self) -> Result<ApiKeyValidation> {
        let url = self.endpoint(&["validate"])?;
        let response: ValidateResponse = self.response_json(self.authorized_get(url)).await?;
        if !response.status.eq_ignore_ascii_case("active") {
            return Err(AppError::new(
                ErrorCode::AuthenticationFailed,
                "Kinsta API token is not active",
            )
            .with_hint("Create or activate a Kinsta API key and update the profile credential"));
        }
        if response.expires_at.as_ref().is_some_and(|value| {
            value.len() > 128 || value.trim() != value || value.chars().any(char::is_control)
        }) {
            return Err(provider_contract_error());
        }
        let company_id = OpaqueId::new(response.company).map_err(|_| provider_contract_error())?;
        Ok(ApiKeyValidation {
            key_name: response.name,
            expires_at: response.expires_at,
            company_id,
            status: response.status,
        })
    }
}

struct BoundedJson<T> {
    value: T,
    body_len: usize,
}

fn transport_error() -> AppError {
    AppError::new(ErrorCode::ProviderUnavailable, "Kinsta API request failed")
        .with_hint("Check the network connection and try again")
}

fn build_http_client() -> Result<Client> {
    Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("hostbraid/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| {
            AppError::new(
                ErrorCode::Internal,
                "could not initialize Kinsta API client",
            )
        })
}

fn map_http_status(status: StatusCode) -> AppError {
    match status {
        StatusCode::BAD_REQUEST => {
            AppError::new(ErrorCode::InvalidInput, "Kinsta rejected the API request")
        }
        StatusCode::UNAUTHORIZED => AppError::new(
            ErrorCode::AuthenticationFailed,
            "Kinsta API authentication failed",
        )
        .with_hint("Update the profile credential with an active Kinsta API key"),
        StatusCode::FORBIDDEN => AppError::new(
            ErrorCode::PolicyDenied,
            "Kinsta denied access to the requested resource",
        )
        .with_hint("Check the API key role and company access"),
        StatusCode::NOT_FOUND => AppError::new(
            ErrorCode::NotFound,
            "Kinsta resource was not found or is not accessible",
        ),
        StatusCode::TOO_MANY_REQUESTS => AppError::new(
            ErrorCode::ProviderUnavailable,
            "Kinsta API rate limit was reached",
        )
        .with_hint("Wait before retrying the request"),
        status if status.is_server_error() => AppError::new(
            ErrorCode::ProviderUnavailable,
            "Kinsta API is temporarily unavailable",
        ),
        _ => AppError::new(
            ErrorCode::ProviderUnavailable,
            "Kinsta API returned an unexpected status",
        ),
    }
}

fn provider_contract_error() -> AppError {
    AppError::new(
        ErrorCode::ProviderUnavailable,
        "Kinsta returned a response HostBraid could not understand",
    )
    .with_hint("Update HostBraid and try again")
}

fn inventory_timeout_error() -> AppError {
    AppError::new(
        ErrorCode::ProviderUnavailable,
        "Kinsta inventory request timed out",
    )
    .with_hint("Try the inventory request again")
}

/// A company-bound Kinsta adapter using the fixed production API endpoint.
pub struct KinstaProvider {
    transport: Transport,
    company_id: OpaqueId,
    descriptor: ProviderDescriptor,
}

impl fmt::Debug for KinstaProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("KinstaProvider")
            .field("company_id", &self.company_id)
            .field("transport", &self.transport)
            .finish_non_exhaustive()
    }
}

impl KinstaProvider {
    /// Validate a token and bind a provider instance to the company returned by Kinsta.
    pub async fn authenticate(token: impl Into<String>) -> Result<(Self, ApiKeyValidation)> {
        let transport = Transport::production(token)?;
        Self::authenticate_transport(transport).await
    }

    /// Build an adapter for a previously validated profile.
    pub fn for_company(token: impl Into<String>, company_id: impl Into<String>) -> Result<Self> {
        let company_id = OpaqueId::new(company_id)?;
        Ok(Self::from_transport(
            Transport::production(token)?,
            company_id,
        ))
    }

    async fn authenticate_transport(transport: Transport) -> Result<(Self, ApiKeyValidation)> {
        let validation = transport.validate_api_key().await?;
        let provider = Self::from_transport(transport, validation.company_id.clone());
        Ok((provider, validation))
    }

    fn from_transport(transport: Transport, company_id: OpaqueId) -> Self {
        Self {
            transport,
            company_id,
            descriptor: ProviderDescriptor {
                id: ProviderId::new("kinsta").expect("the built-in provider id is valid"),
                display_name: "Kinsta".to_owned(),
                documentation_url: Some("https://api-docs.kinsta.com/".to_owned()),
            },
        }
    }

    /// Revalidate this instance's credential without changing its configured company.
    pub async fn validate_api_key(&self) -> Result<ApiKeyValidation> {
        self.transport.validate_api_key().await
    }

    #[must_use]
    pub fn company_id(&self) -> &OpaqueId {
        &self.company_id
    }

    /// Fetch sites and their environments in one coherent provider response.
    pub async fn catalog_snapshot(&self, profile: &ProviderProfileRef) -> Result<CatalogSnapshot> {
        self.ensure_profile(profile)?;
        let url = self.transport.endpoint(&["sites"])?;
        let request = self.transport.authorized_get(url).query(&[
            ("company", self.company_id.as_str()),
            ("include_environments", "true"),
        ]);
        let response: SitesResponse = self.transport.response_json(request).await?;
        map_catalog(profile, response)
    }

    /// Check current SSH availability without changing it.
    pub async fn ssh_status(&self, environment: &EnvironmentRef) -> Result<bool> {
        self.ensure_environment(environment)?;
        let url = self.transport.endpoint(&[
            "sites",
            "environments",
            environment.environment_id.as_str(),
            "ssh",
            "get-status",
        ])?;
        let response: SshStatusResponse = self
            .transport
            .response_json(self.transport.authorized_get(url))
            .await?;
        Ok(response.environment.active_container.is_ssh_enabled)
    }

    /// Return validated SSH coordinates. Kinsta's `ssh_command` response field is ignored.
    pub async fn ssh_target(&self, environment: &EnvironmentRef) -> Result<SshTarget> {
        if !self.ssh_status(environment).await? {
            return Err(AppError::new(
                ErrorCode::Unavailable,
                "SSH is disabled for this Kinsta environment",
            )
            .with_hint("Enable SSH in MyKinsta before connecting"));
        }

        let url = self.transport.endpoint(&[
            "sites",
            environment.site_id.as_str(),
            "environments",
            environment.environment_id.as_str(),
            "ssh",
            "config",
        ])?;
        let response: SshConfigResponse = self
            .transport
            .response_json(self.transport.authorized_get(url))
            .await?;
        if response.site.id != environment.site_id.as_str()
            || response.site.environment.id != environment.environment_id.as_str()
        {
            return Err(provider_contract_error());
        }
        let port = response
            .port
            .parse::<u16>()
            .map_err(|_| provider_contract_error())?;
        SshTarget::try_new(response.host, port, response.user, None)
            .map_err(|_| provider_contract_error())
    }

    /// Fetch and normalize all company plugin pages.
    pub async fn plugin_inventory(
        &self,
        profile: &ProviderProfileRef,
    ) -> Result<WordPressComponentInventory> {
        self.component_inventory(profile, InventoryKind::Plugin)
            .await
    }

    /// Fetch and normalize all company theme pages.
    pub async fn theme_inventory(
        &self,
        profile: &ProviderProfileRef,
    ) -> Result<WordPressComponentInventory> {
        self.component_inventory(profile, InventoryKind::Theme)
            .await
    }

    async fn component_inventory(
        &self,
        profile: &ProviderProfileRef,
        kind: InventoryKind,
    ) -> Result<WordPressComponentInventory> {
        self.ensure_profile(profile)?;
        let raw = self.fetch_inventory_pages(kind).await?;
        let mut catalog = self.catalog_snapshot(profile).await?;
        let mut index = environment_index(&catalog)?;

        if raw.items.iter().any(|item| {
            item.environments
                .iter()
                .any(|installation| !index.contains_key(&installation.id))
        }) {
            catalog = self.catalog_snapshot(profile).await?;
            index = environment_index(&catalog)?;
        }

        map_inventory(kind, raw, &index)
    }

    async fn fetch_inventory_pages(&self, kind: InventoryKind) -> Result<RawInventory> {
        self.fetch_inventory_pages_with_limits(kind, INVENTORY_PAGING_TIMEOUT, INVENTORY_LIMITS)
            .await
    }

    async fn fetch_inventory_pages_with_limits(
        &self,
        kind: InventoryKind,
        timeout: Duration,
        limits: InventoryLimits,
    ) -> Result<RawInventory> {
        tokio::time::timeout(timeout, self.fetch_inventory_pages_inner(kind, limits))
            .await
            .map_err(|_| inventory_timeout_error())?
    }

    async fn fetch_inventory_pages_inner(
        &self,
        kind: InventoryKind,
        limits: InventoryLimits,
    ) -> Result<RawInventory> {
        let url =
            self.transport
                .endpoint(&["company", self.company_id.as_str(), kind.path_segment()])?;
        let mut offset = 0_u64;
        let mut expected_total = None;
        let mut refreshed_at = None;
        let mut items = Vec::new();
        let mut component_names = HashSet::new();
        let mut pages = 0_u64;
        let mut aggregate_body_bytes = 0_usize;

        loop {
            if pages >= limits.max_pages {
                return Err(provider_contract_error());
            }
            pages += 1;

            let request = self
                .transport
                .authorized_get(url.clone())
                .json(&InventoryRequest {
                    offset,
                    limit: INVENTORY_PAGE_SIZE,
                    order_by: InventoryOrderBy {
                        field: "name",
                        order: "ascend",
                    },
                });
            let response: BoundedJson<InventoryResponse> = self
                .transport
                .response_json_with_limit(request, MAX_HTTP_RESPONSE_BODY_BYTES)
                .await?;
            aggregate_body_bytes = aggregate_body_bytes
                .checked_add(response.body_len)
                .filter(|size| *size <= limits.max_body_bytes)
                .ok_or_else(provider_contract_error)?;

            let page = response.value.company.inventory;
            if page.total > limits.max_items
                || expected_total.is_some_and(|total| total != page.total)
            {
                return Err(provider_contract_error());
            }
            expected_total = Some(page.total);
            if pages == 1 {
                refreshed_at = page.last_updated_at.clone();
            } else if page.last_updated_at != refreshed_at {
                return Err(provider_contract_error());
            }
            if page
                .items
                .iter()
                .any(|item| !component_names.insert(item.name.clone()))
            {
                return Err(provider_contract_error());
            }
            let page_len =
                u64::try_from(page.items.len()).map_err(|_| provider_contract_error())?;
            if page_len > INVENTORY_PAGE_SIZE {
                return Err(provider_contract_error());
            }

            let aggregate_len = u64::try_from(items.len())
                .map_err(|_| provider_contract_error())?
                .checked_add(page_len)
                .filter(|length| *length <= limits.max_items && *length <= page.total)
                .ok_or_else(provider_contract_error)?;
            items.extend(page.items);

            if aggregate_len == page.total {
                return Ok(RawInventory {
                    total: page.total,
                    refreshed_at,
                    items,
                });
            }
            if page_len == 0 {
                return Err(provider_contract_error());
            }
            offset = aggregate_len;
        }
    }

    fn ensure_profile(&self, profile: &ProviderProfileRef) -> Result<()> {
        if profile.provider == self.descriptor.id {
            return Ok(());
        }
        Err(AppError::new(
            ErrorCode::InvalidInput,
            "profile does not belong to the Kinsta provider",
        ))
    }

    fn ensure_environment(&self, environment: &EnvironmentRef) -> Result<()> {
        if environment.provider == self.descriptor.id {
            return Ok(());
        }
        Err(AppError::new(
            ErrorCode::InvalidInput,
            "environment does not belong to the Kinsta provider",
        ))
    }

    #[cfg(test)]
    fn for_company_at(
        token: impl Into<String>,
        company_id: impl Into<String>,
        base_url: Url,
    ) -> Result<Self> {
        let company_id = OpaqueId::new(company_id)?;
        let transport = Transport::new(token, base_url, build_http_client()?)?;
        Ok(Self::from_transport(transport, company_id))
    }

    #[cfg(test)]
    async fn authenticate_at(
        token: impl Into<String>,
        base_url: Url,
    ) -> Result<(Self, ApiKeyValidation)> {
        let transport = Transport::new(token, base_url, build_http_client()?)?;
        Self::authenticate_transport(transport).await
    }
}

impl ProviderIdentity for KinstaProvider {
    fn descriptor(&self) -> &ProviderDescriptor {
        &self.descriptor
    }
}

#[async_trait]
impl Catalog for KinstaProvider {
    async fn catalog_snapshot(&self, profile: &ProviderProfileRef) -> Result<CatalogSnapshot> {
        KinstaProvider::catalog_snapshot(self, profile).await
    }
}

#[async_trait]
impl CapabilitySource for KinstaProvider {
    async fn capabilities(&self, environment: &EnvironmentRef) -> Result<Vec<Capability>> {
        let capability = if self.ssh_status(environment).await? {
            Capability::available(SSH_CAPABILITY, 1)
        } else {
            Capability::unavailable(
                SSH_CAPABILITY,
                1,
                "ssh_disabled",
                Some("Enable SSH in MyKinsta".to_owned()),
            )
        };
        Ok(vec![capability])
    }
}

#[async_trait]
impl SshAccess for KinstaProvider {
    async fn ssh_target(&self, environment: &EnvironmentRef) -> Result<SshTarget> {
        KinstaProvider::ssh_target(self, environment).await
    }
}

#[async_trait]
impl WordPressInventory for KinstaProvider {
    async fn plugin_inventory(
        &self,
        profile: &ProviderProfileRef,
    ) -> Result<WordPressComponentInventory> {
        KinstaProvider::plugin_inventory(self, profile).await
    }

    async fn theme_inventory(
        &self,
        profile: &ProviderProfileRef,
    ) -> Result<WordPressComponentInventory> {
        KinstaProvider::theme_inventory(self, profile).await
    }
}

fn map_catalog(profile: &ProviderProfileRef, response: SitesResponse) -> Result<CatalogSnapshot> {
    let mut sites = Vec::with_capacity(response.company.sites.len());
    for site in response.company.sites {
        let reference = SiteRef::try_new(
            profile.provider.as_str(),
            profile.profile.as_str(),
            site.id.clone(),
        )
        .map_err(|_| provider_contract_error())?;
        let mut environments = site
            .environments
            .into_iter()
            .map(|environment| map_environment(profile, &site.id, environment))
            .collect::<Result<Vec<_>>>()?;
        sort_environments(&mut environments);
        let primary_domain = environments
            .iter()
            .find(|environment| environment.kind == EnvironmentKind::Production)
            .and_then(|environment| environment.primary_domain.clone());

        let mut labels = site
            .labels
            .into_iter()
            .map(|label| {
                Ok(SiteLabel {
                    id: OpaqueId::new(label.id).map_err(|_| provider_contract_error())?,
                    name: label.name,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        labels.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| left.id.as_str().cmp(right.id.as_str()))
        });

        sites.push(CatalogSite {
            site: SiteSummary {
                reference,
                display_name: site.display_name,
                primary_domain,
                labels,
            },
            environments,
        });
    }
    sites.sort_by(|left, right| {
        left.site
            .display_name
            .cmp(&right.site.display_name)
            .then_with(|| {
                left.site
                    .reference
                    .site_id
                    .as_str()
                    .cmp(right.site.reference.site_id.as_str())
            })
    });
    Ok(CatalogSnapshot {
        profile: profile.clone(),
        sites,
    })
}

fn map_environment(
    profile: &ProviderProfileRef,
    site_id: &str,
    environment: wire::Environment,
) -> Result<EnvironmentSummary> {
    let provider_kind = environment.name;
    Ok(EnvironmentSummary {
        reference: EnvironmentRef::try_new(
            profile.provider.as_str(),
            profile.profile.as_str(),
            site_id,
            environment.id,
        )
        .map_err(|_| provider_contract_error())?,
        display_name: environment.display_name,
        kind: normalize_environment_kind(&provider_kind),
        provider_kind: Some(provider_kind),
        primary_domain: environment.primary_domain.map(|domain| domain.name),
    })
}

fn normalize_environment_kind(value: &str) -> EnvironmentKind {
    match value.to_ascii_lowercase().as_str() {
        "live" | "prod" | "production" => EnvironmentKind::Production,
        "stage" | "staging" => EnvironmentKind::Staging,
        "dev" | "development" => EnvironmentKind::Development,
        _ => EnvironmentKind::Other,
    }
}

fn sort_environments(environments: &mut [EnvironmentSummary]) {
    environments.sort_by(|left, right| {
        environment_kind_rank(left.kind)
            .cmp(&environment_kind_rank(right.kind))
            .then_with(|| left.display_name.cmp(&right.display_name))
            .then_with(|| {
                left.reference
                    .environment_id
                    .as_str()
                    .cmp(right.reference.environment_id.as_str())
            })
    });
}

fn environment_kind_rank(kind: EnvironmentKind) -> u8 {
    match kind {
        EnvironmentKind::Production => 0,
        EnvironmentKind::Staging => 1,
        EnvironmentKind::Development => 2,
        EnvironmentKind::Other => 3,
        _ => 4,
    }
}

#[derive(Clone, Copy)]
enum InventoryKind {
    Plugin,
    Theme,
}

impl InventoryKind {
    const fn path_segment(self) -> &'static str {
        match self {
            Self::Plugin => "wp-plugins",
            Self::Theme => "wp-themes",
        }
    }

    const fn core_kind(self) -> WordPressComponentKind {
        match self {
            Self::Plugin => WordPressComponentKind::Plugin,
            Self::Theme => WordPressComponentKind::Theme,
        }
    }
}

struct RawInventory {
    total: u64,
    refreshed_at: Option<String>,
    items: Vec<InventoryItem>,
}

fn environment_index(snapshot: &CatalogSnapshot) -> Result<HashMap<String, EnvironmentRef>> {
    let mut index = HashMap::new();
    for site in &snapshot.sites {
        for environment in &site.environments {
            let id = environment.reference.environment_id.as_str().to_owned();
            if index.insert(id, environment.reference.clone()).is_some() {
                return Err(provider_contract_error());
            }
        }
    }
    Ok(index)
}

fn map_inventory(
    kind: InventoryKind,
    raw: RawInventory,
    environments: &HashMap<String, EnvironmentRef>,
) -> Result<WordPressComponentInventory> {
    let mut components = raw
        .items
        .into_iter()
        .map(|item| {
            let mut installations = item
                .environments
                .into_iter()
                .map(|installation| {
                    let environment = environments
                        .get(&installation.id)
                        .cloned()
                        .ok_or_else(provider_contract_error)?;
                    Ok(WordPressComponentInstallation {
                        environment,
                        status: installation.status,
                        installed_version: installation.version,
                        installed_version_vulnerable: installation.is_version_vulnerable,
                        update_state: installation.update,
                        available_version: installation.update_version,
                        available_version_vulnerable: installation.is_update_version_vulnerable,
                        update_status: installation.update_status,
                        auto_update_type: installation.auto_update_type,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            installations.sort_by(|left, right| {
                left.environment
                    .site_id
                    .as_str()
                    .cmp(right.environment.site_id.as_str())
                    .then_with(|| {
                        left.environment
                            .environment_id
                            .as_str()
                            .cmp(right.environment.environment_id.as_str())
                    })
            });
            Ok(WordPressComponent {
                slug: item.name,
                title: item.title,
                description: item.description,
                latest_version: item.latest_version,
                latest_version_vulnerable: item.is_latest_version_vulnerable,
                environment_count: item.environment_count,
                update_count: item.update_count,
                installations,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    components.sort_by(|left, right| {
        left.title
            .cmp(&right.title)
            .then_with(|| left.slug.cmp(&right.slug))
    });

    Ok(WordPressComponentInventory {
        kind: kind.core_kind(),
        total: raw.total,
        refreshed_at: raw.refreshed_at,
        components,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use hostbraid_core::ErrorCode;
    use httpmock::prelude::*;
    use serde_json::json;

    fn test_base_url(server: &MockServer) -> Url {
        Url::parse(&format!("{}/v2", server.base_url())).expect("mock base URL is valid")
    }

    fn profile() -> ProviderProfileRef {
        ProviderProfileRef::try_new("kinsta", "agency").expect("test profile is valid")
    }

    fn environment(site_id: &str, environment_id: &str) -> EnvironmentRef {
        EnvironmentRef::try_new("kinsta", "agency", site_id, environment_id)
            .expect("test environment reference is valid")
    }

    fn inventory_request(offset: u64) -> serde_json::Value {
        json!({
            "offset": offset,
            "limit": 100,
            "order_by": {"field": "name", "order": "ascend"}
        })
    }

    fn catalog_fixture() -> serde_json::Value {
        json!({
            "company": {
                "sites": [{
                    "id": "site_one",
                    "name": "firstsite",
                    "display_name": "First Site",
                    "status": "live",
                    "siteLabels": [
                        {"id": "label_z", "name": "Zeta"},
                        {"id": "label_a", "name": "Agency"}
                    ],
                    "environments": [{
                        "id": "env_stage",
                        "name": "staging",
                        "display_name": "Staging",
                        "primaryDomain": {"name": "staging.example.test"},
                        "future_field": "ignored"
                    }, {
                        "id": "env_live",
                        "name": "live",
                        "display_name": "Live",
                        "primaryDomain": {"name": "example.test"}
                    }],
                    "future_field": {"safe": true}
                }]
            }
        })
    }

    #[tokio::test]
    async fn authentication_derives_company_and_redacts_token() {
        let server = MockServer::start_async().await;
        let token = "token-canary-value";
        let validate = server
            .mock_async(|when, then| {
                when.method(GET)
                    .path("/v2/validate")
                    .header("authorization", "Bearer token-canary-value");
                then.status(200).json_body(json!({
                    "name": "Automation key",
                    "expires_at": "1900000000000",
                    "company": "company_one",
                    "status": "active",
                    "future_field": true
                }));
            })
            .await;

        let (provider, validation) = KinstaProvider::authenticate_at(token, test_base_url(&server))
            .await
            .expect("active key validates");

        validate.assert_async().await;
        assert_eq!(validation.company_id.as_str(), "company_one");
        assert_eq!(validation.key_name, "Automation key");
        assert_eq!(provider.company_id().as_str(), "company_one");
        assert!(!format!("{provider:?}").contains(token));
    }

    #[tokio::test]
    async fn inactive_key_is_an_authentication_failure() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/v2/validate");
                then.status(200).json_body(json!({
                    "name": "Expired key",
                    "expires_at": null,
                    "company": "company_one",
                    "status": "expired"
                }));
            })
            .await;

        let error = KinstaProvider::authenticate_at("safe-token", test_base_url(&server))
            .await
            .expect_err("inactive key must fail");
        assert_eq!(error.code(), ErrorCode::AuthenticationFailed);
        assert!(!error.to_string().contains("safe-token"));
    }

    #[tokio::test]
    async fn catalog_maps_labels_domains_and_environment_kinds() {
        let server = MockServer::start_async().await;
        let catalog = server
            .mock_async(|when, then| {
                when.method(GET)
                    .path("/v2/sites")
                    .query_param("company", "company_one")
                    .query_param("include_environments", "true")
                    .header("authorization", "Bearer safe-token");
                then.status(200).json_body(catalog_fixture());
            })
            .await;
        let provider =
            KinstaProvider::for_company_at("safe-token", "company_one", test_base_url(&server))
                .expect("provider initializes");

        let snapshot = provider
            .catalog_snapshot(&profile())
            .await
            .expect("catalog maps");

        assert_eq!(snapshot.sites.len(), 1);
        let site = &snapshot.sites[0];
        assert_eq!(site.site.primary_domain.as_deref(), Some("example.test"));
        assert_eq!(site.site.labels[0].name, "Agency");
        assert_eq!(site.environments[0].kind, EnvironmentKind::Production);
        assert_eq!(site.environments[0].provider_kind.as_deref(), Some("live"));
        assert_eq!(site.environments[1].kind, EnvironmentKind::Staging);

        let sites = provider
            .list_sites(&profile())
            .await
            .expect("derived list helper works");
        assert_eq!(sites[0].reference.site_id.as_str(), "site_one");
        catalog.assert_hits_async(2).await;
    }

    #[tokio::test]
    async fn ssh_target_checks_status_and_ignores_provider_command() {
        let server = MockServer::start_async().await;
        let status = server
            .mock_async(|when, then| {
                when.method(GET)
                    .path("/v2/sites/environments/env_live/ssh/get-status");
                then.status(200).json_body(json!({
                    "environment": {"active_container": {"is_ssh_enabled": true}}
                }));
            })
            .await;
        let config = server
            .mock_async(|when, then| {
                when.method(GET)
                    .path("/v2/sites/site_one/environments/env_live/ssh/config");
                then.status(200).json_body(json!({
                    "site": {
                        "id": "site_one",
                        "name": "firstsite",
                        "displayName": "First Site",
                        "usr": "firstsite",
                        "environment": {"id": "env_live", "activeContainer": {}}
                    },
                    "port": "61022",
                    "name": "First Site",
                    "host": "203.0.113.15",
                    "user": "site_user",
                    "ssh_command": "ssh attacker.example"
                }));
            })
            .await;
        let provider =
            KinstaProvider::for_company_at("safe-token", "company_one", test_base_url(&server))
                .expect("provider initializes");

        let target = provider
            .ssh_target(&environment("site_one", "env_live"))
            .await
            .expect("coordinates map");

        assert_eq!(target.host(), "203.0.113.15");
        assert_eq!(target.port(), 61022);
        assert_eq!(target.user(), "site_user");
        assert_eq!(target.working_directory(), None);
        status.assert_async().await;
        config.assert_async().await;
    }

    #[tokio::test]
    async fn disabled_ssh_never_fetches_connection_config() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(GET)
                    .path("/v2/sites/environments/env_live/ssh/get-status");
                then.status(200).json_body(json!({
                    "environment": {"active_container": {"is_ssh_enabled": false}}
                }));
            })
            .await;
        let config = server
            .mock_async(|when, then| {
                when.method(GET)
                    .path("/v2/sites/site_one/environments/env_live/ssh/config");
                then.status(500);
            })
            .await;
        let provider =
            KinstaProvider::for_company_at("safe-token", "company_one", test_base_url(&server))
                .expect("provider initializes");

        let error = provider
            .ssh_target(&environment("site_one", "env_live"))
            .await
            .expect_err("disabled SSH is unavailable");

        assert_eq!(error.code(), ErrorCode::Unavailable);
        config.assert_hits_async(0).await;
    }

    #[tokio::test]
    async fn malformed_ssh_coordinates_are_provider_contract_errors() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(GET)
                    .path("/v2/sites/environments/env_live/ssh/get-status");
                then.status(200).json_body(json!({
                    "environment": {"active_container": {"is_ssh_enabled": true}}
                }));
            })
            .await;
        server
            .mock_async(|when, then| {
                when.method(GET)
                    .path("/v2/sites/site_one/environments/env_live/ssh/config");
                then.status(200).json_body(json!({
                    "site": {"id": "site_one", "environment": {"id": "env_live"}},
                    "port": "999999",
                    "host": "host.example",
                    "user": "site_user",
                    "ssh_command": "ignored"
                }));
            })
            .await;
        let provider =
            KinstaProvider::for_company_at("safe-token", "company_one", test_base_url(&server))
                .expect("provider initializes");

        let error = provider
            .ssh_target(&environment("site_one", "env_live"))
            .await
            .expect_err("invalid port fails safely");
        assert_eq!(error.code(), ErrorCode::ProviderUnavailable);
    }

    #[tokio::test]
    async fn plugin_inventory_fetches_every_page_and_joins_exact_environment_refs() {
        let server = MockServer::start_async().await;
        let first_page = server
            .mock_async(|when, then| {
                when.method(GET)
                    .path("/v2/company/company_one/wp-plugins")
                    .json_body(inventory_request(0));
                then.status(200).json_body(json!({
                    "company": {"plugins": {
                        "total": 2,
                        "last_updated_at": "2026-07-15T10:00:00.000Z",
                        "items": [plugin_item("alpha", "Alpha", "env_live", "active")]
                    }}
                }));
            })
            .await;
        let second_page = server
            .mock_async(|when, then| {
                when.method(GET)
                    .path("/v2/company/company_one/wp-plugins")
                    .json_body(inventory_request(1));
                then.status(200).json_body(json!({
                    "company": {"plugins": {
                        "total": 2,
                        "last_updated_at": "2026-07-15T10:00:00.000Z",
                        "items": [plugin_item("beta", "Beta", "env_stage", "must-use")]
                    }}
                }));
            })
            .await;
        let catalog = server
            .mock_async(|when, then| {
                when.method(GET).path("/v2/sites");
                then.status(200).json_body(catalog_fixture());
            })
            .await;
        let provider =
            KinstaProvider::for_company_at("safe-token", "company_one", test_base_url(&server))
                .expect("provider initializes");

        let inventory = provider
            .plugin_inventory(&profile())
            .await
            .expect("inventory maps");

        assert_eq!(inventory.kind, WordPressComponentKind::Plugin);
        assert_eq!(inventory.total, 2);
        assert_eq!(inventory.components.len(), 2);
        assert_eq!(
            inventory.components[0].installations[0]
                .environment
                .site_id
                .as_str(),
            "site_one"
        );
        assert_eq!(
            inventory.components[0].installations[0]
                .auto_update_type
                .as_deref(),
            Some("Future Beta Channel")
        );
        assert_eq!(inventory.components[1].installations[0].status, "must-use");
        first_page.assert_async().await;
        second_page.assert_async().await;
        catalog.assert_async().await;
    }

    #[tokio::test]
    async fn inventory_requests_pin_deterministic_name_ordering() {
        let server = MockServer::start_async().await;
        let request = server
            .mock_async(|when, then| {
                when.method(GET)
                    .path("/v2/company/company_one/wp-plugins")
                    .json_body(inventory_request(0));
                then.status(200).json_body(json!({
                    "company": {"plugins": {
                        "total": 0,
                        "last_updated_at": "2026-07-15T10:00:00.000Z",
                        "items": []
                    }}
                }));
            })
            .await;
        let provider =
            KinstaProvider::for_company_at("safe-token", "company_one", test_base_url(&server))
                .expect("provider initializes");

        let inventory = provider
            .fetch_inventory_pages(InventoryKind::Plugin)
            .await
            .expect("empty inventory maps");

        assert_eq!(inventory.total, 0);
        request.assert_async().await;
    }

    #[tokio::test]
    async fn inventory_rejects_snapshot_timestamp_drift_between_pages() {
        let server = MockServer::start_async().await;
        let first_page = server
            .mock_async(|when, then| {
                when.method(GET)
                    .path("/v2/company/company_one/wp-plugins")
                    .json_body(inventory_request(0));
                then.status(200).json_body(json!({
                    "company": {"plugins": {
                        "total": 2,
                        "last_updated_at": "2026-07-15T10:00:00.000Z",
                        "items": [plugin_item("alpha", "Alpha", "env_live", "active")]
                    }}
                }));
            })
            .await;
        let second_page = server
            .mock_async(|when, then| {
                when.method(GET)
                    .path("/v2/company/company_one/wp-plugins")
                    .json_body(inventory_request(1));
                then.status(200).json_body(json!({
                    "company": {"plugins": {
                        "total": 2,
                        "last_updated_at": "2026-07-15T10:01:00.000Z",
                        "items": [plugin_item("beta", "Beta", "env_stage", "active")]
                    }}
                }));
            })
            .await;
        let provider =
            KinstaProvider::for_company_at("safe-token", "company_one", test_base_url(&server))
                .expect("provider initializes");

        let error = provider
            .fetch_inventory_pages(InventoryKind::Plugin)
            .await
            .err()
            .expect("mixed inventory snapshots must fail");

        assert_eq!(error.code(), ErrorCode::ProviderUnavailable);
        first_page.assert_async().await;
        second_page.assert_async().await;
    }

    #[tokio::test]
    async fn inventory_rejects_duplicate_component_names_across_pages() {
        let server = MockServer::start_async().await;
        let first_page = server
            .mock_async(|when, then| {
                when.method(GET)
                    .path("/v2/company/company_one/wp-themes")
                    .json_body(inventory_request(0));
                then.status(200).json_body(json!({
                    "company": {"themes": {
                        "total": 2,
                        "last_updated_at": "2026-07-15T10:00:00.000Z",
                        "items": [theme_item("shared", "Shared", "env_live")]
                    }}
                }));
            })
            .await;
        let second_page = server
            .mock_async(|when, then| {
                when.method(GET)
                    .path("/v2/company/company_one/wp-themes")
                    .json_body(inventory_request(1));
                then.status(200).json_body(json!({
                    "company": {"themes": {
                        "total": 2,
                        "last_updated_at": "2026-07-15T10:00:00.000Z",
                        "items": [theme_item("shared", "Shared duplicate", "env_stage")]
                    }}
                }));
            })
            .await;
        let provider =
            KinstaProvider::for_company_at("safe-token", "company_one", test_base_url(&server))
                .expect("provider initializes");

        let error = provider
            .fetch_inventory_pages(InventoryKind::Theme)
            .await
            .err()
            .expect("duplicate component identities must fail");

        assert_eq!(error.code(), ErrorCode::ProviderUnavailable);
        first_page.assert_async().await;
        second_page.assert_async().await;
    }

    #[tokio::test]
    async fn inventory_refetches_catalog_once_before_failing_unknown_environment() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/v2/company/company_one/wp-themes");
                then.status(200).json_body(json!({
                    "company": {"themes": {
                        "total": 1,
                        "last_updated_at": null,
                        "items": [theme_item("future", "Future", "env_missing")]
                    }}
                }));
            })
            .await;
        let catalog = server
            .mock_async(|when, then| {
                when.method(GET).path("/v2/sites");
                then.status(200).json_body(catalog_fixture());
            })
            .await;
        let provider =
            KinstaProvider::for_company_at("safe-token", "company_one", test_base_url(&server))
                .expect("provider initializes");

        let error = provider
            .theme_inventory(&profile())
            .await
            .expect_err("unknown inventory environment must fail");

        assert_eq!(error.code(), ErrorCode::ProviderUnavailable);
        catalog.assert_hits_async(2).await;
    }

    #[tokio::test]
    async fn response_body_limit_is_checked_before_json_deserialization() {
        let server = MockServer::start_async().await;
        let response = json!({
            "value": "provider-body-canary",
            "padding": "x".repeat(256)
        });
        server
            .mock_async(|when, then| {
                when.method(GET).path("/v2/bounded");
                then.status(200).json_body(response);
            })
            .await;
        let transport = Transport::new(
            "safe-token",
            test_base_url(&server),
            build_http_client().expect("HTTP client initializes"),
        )
        .expect("transport initializes");
        let url = transport
            .endpoint(&["bounded"])
            .expect("test endpoint builds");

        let error = transport
            .response_json_with_limit::<serde_json::Value>(transport.authorized_get(url), 64)
            .await
            .err()
            .expect("oversized valid JSON must be rejected");
        let rendered = format!("{error:?} {error}");

        assert_eq!(error.code(), ErrorCode::ProviderUnavailable);
        assert!(!rendered.contains("provider-body-canary"));
    }

    #[tokio::test]
    async fn inventory_rejects_totals_and_page_items_above_bounds() {
        let server = MockServer::start_async().await;
        let excessive_total = server
            .mock_async(|when, then| {
                when.method(GET).path("/v2/company/company_one/wp-plugins");
                then.status(200).json_body(json!({
                    "company": {"plugins": {
                        "total": 11,
                        "last_updated_at": null,
                        "items": []
                    }}
                }));
            })
            .await;
        let provider =
            KinstaProvider::for_company_at("safe-token", "company_one", test_base_url(&server))
                .expect("provider initializes");
        let limits = InventoryLimits {
            max_items: 10,
            max_pages: 10,
            max_body_bytes: MAX_INVENTORY_BODY_BYTES,
        };

        let error = provider
            .fetch_inventory_pages_with_limits(
                InventoryKind::Plugin,
                Duration::from_secs(1),
                limits,
            )
            .await
            .err()
            .expect("provider totals above the item bound must fail");

        assert_eq!(error.code(), ErrorCode::ProviderUnavailable);
        excessive_total.assert_async().await;

        excessive_total.delete_async().await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/v2/company/company_one/wp-plugins");
                then.status(200).json_body(json!({
                    "company": {"plugins": {
                        "total": 1,
                        "last_updated_at": null,
                        "items": [
                            plugin_item("alpha", "Alpha", "env_live", "active"),
                            plugin_item("beta", "Beta", "env_live", "active")
                        ]
                    }}
                }));
            })
            .await;

        let error = provider
            .fetch_inventory_pages_with_limits(
                InventoryKind::Plugin,
                Duration::from_secs(1),
                limits,
            )
            .await
            .err()
            .expect("page items above the reported total must fail");
        assert_eq!(error.code(), ErrorCode::ProviderUnavailable);
    }

    #[tokio::test]
    async fn inventory_caps_pages_and_never_fetches_past_the_bound() {
        let server = MockServer::start_async().await;
        let first_page = server
            .mock_async(|when, then| {
                when.method(GET)
                    .path("/v2/company/company_one/wp-plugins")
                    .json_body(inventory_request(0));
                then.status(200).json_body(json!({
                    "company": {"plugins": {
                        "total": 2,
                        "last_updated_at": null,
                        "items": [plugin_item("alpha", "Alpha", "env_live", "active")]
                    }}
                }));
            })
            .await;
        let second_page = server
            .mock_async(|when, then| {
                when.method(GET)
                    .path("/v2/company/company_one/wp-plugins")
                    .json_body(inventory_request(1));
                then.status(200).json_body(json!({
                    "company": {"plugins": {
                        "total": 2,
                        "last_updated_at": null,
                        "items": [plugin_item("beta", "Beta", "env_live", "active")]
                    }}
                }));
            })
            .await;
        let provider =
            KinstaProvider::for_company_at("safe-token", "company_one", test_base_url(&server))
                .expect("provider initializes");

        let error = provider
            .fetch_inventory_pages_with_limits(
                InventoryKind::Plugin,
                Duration::from_secs(1),
                InventoryLimits {
                    max_items: 10,
                    max_pages: 1,
                    max_body_bytes: MAX_INVENTORY_BODY_BYTES,
                },
            )
            .await
            .err()
            .expect("pagination above the page bound must fail");

        assert_eq!(error.code(), ErrorCode::ProviderUnavailable);
        first_page.assert_async().await;
        second_page.assert_hits_async(0).await;
    }

    #[tokio::test]
    async fn inventory_caps_aggregate_response_bytes_across_pages() {
        let server = MockServer::start_async().await;
        let first_body = json!({
            "company": {"plugins": {
                "total": 2,
                "last_updated_at": null,
                "items": [plugin_item("alpha", "Alpha", "env_live", "active")]
            }}
        });
        let second_body = json!({
            "company": {"plugins": {
                "total": 2,
                "last_updated_at": null,
                "items": [plugin_item("beta", "Beta", "env_live", "active")]
            }}
        });
        let aggregate_size = first_body.to_string().len() + second_body.to_string().len();
        server
            .mock_async(|when, then| {
                when.method(GET)
                    .path("/v2/company/company_one/wp-plugins")
                    .json_body(inventory_request(0));
                then.status(200).json_body(first_body);
            })
            .await;
        let second_page = server
            .mock_async(|when, then| {
                when.method(GET)
                    .path("/v2/company/company_one/wp-plugins")
                    .json_body(inventory_request(1));
                then.status(200).json_body(second_body);
            })
            .await;
        let provider =
            KinstaProvider::for_company_at("safe-token", "company_one", test_base_url(&server))
                .expect("provider initializes");

        let error = provider
            .fetch_inventory_pages_with_limits(
                InventoryKind::Plugin,
                Duration::from_secs(1),
                InventoryLimits {
                    max_items: 10,
                    max_pages: 10,
                    max_body_bytes: aggregate_size - 1,
                },
            )
            .await
            .err()
            .expect("aggregate response bodies above the bound must fail");

        assert_eq!(error.code(), ErrorCode::ProviderUnavailable);
        second_page.assert_async().await;
    }

    #[tokio::test]
    async fn inventory_paging_has_one_overall_deadline() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/v2/company/company_one/wp-themes");
                then.status(200)
                    .delay(Duration::from_millis(200))
                    .json_body(json!({
                        "company": {"themes": {
                            "total": 0,
                            "last_updated_at": null,
                            "items": []
                        }}
                    }));
            })
            .await;
        let provider =
            KinstaProvider::for_company_at("safe-token", "company_one", test_base_url(&server))
                .expect("provider initializes");

        let error = provider
            .fetch_inventory_pages_with_limits(
                InventoryKind::Theme,
                Duration::from_millis(20),
                INVENTORY_LIMITS,
            )
            .await
            .err()
            .expect("the overall inventory deadline must expire");

        assert_eq!(error.code(), ErrorCode::ProviderUnavailable);
        assert_eq!(error.message(), "Kinsta inventory request timed out");
    }

    #[tokio::test]
    async fn response_bodies_and_tokens_never_enter_public_errors() {
        let server = MockServer::start_async().await;
        let canary = "secret-token-canary";
        server
            .mock_async(|when, then| {
                when.method(GET).path("/v2/sites");
                then.status(401)
                    .body("raw-provider-body-secret-token-canary");
            })
            .await;
        let provider =
            KinstaProvider::for_company_at(canary, "company_one", test_base_url(&server))
                .expect("provider initializes");

        let error = provider
            .catalog_snapshot(&profile())
            .await
            .expect_err("unauthorized request fails");
        let rendered = format!("{error:?} {error}");

        assert_eq!(error.code(), ErrorCode::AuthenticationFailed);
        assert!(!rendered.contains(canary));
        assert!(!rendered.contains("raw-provider-body"));
    }

    #[tokio::test]
    async fn rate_limits_have_a_stable_secret_safe_error() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/v2/sites");
                then.status(429).body("arbitrary provider response");
            })
            .await;
        let provider =
            KinstaProvider::for_company_at("safe-token", "company_one", test_base_url(&server))
                .expect("provider initializes");

        let error = provider
            .catalog_snapshot(&profile())
            .await
            .expect_err("rate limit fails");
        assert_eq!(error.code(), ErrorCode::ProviderUnavailable);
        assert_eq!(error.message(), "Kinsta API rate limit was reached");
    }

    #[test]
    fn provider_statuses_map_to_stable_error_categories() {
        assert_eq!(
            map_http_status(StatusCode::NOT_FOUND).code(),
            ErrorCode::NotFound
        );
        assert_eq!(
            map_http_status(StatusCode::INTERNAL_SERVER_ERROR).code(),
            ErrorCode::ProviderUnavailable
        );
        assert_eq!(
            map_http_status(StatusCode::FORBIDDEN).code(),
            ErrorCode::PolicyDenied
        );
    }

    #[tokio::test]
    async fn transport_failures_do_not_expose_reqwest_details() {
        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").expect("ephemeral port can be reserved");
        let address = listener.local_addr().expect("listener has an address");
        drop(listener);
        let base_url = Url::parse(&format!("http://{address}/v2")).expect("test URL is valid");
        let provider = KinstaProvider::for_company_at("safe-token", "company_one", base_url)
            .expect("provider initializes");

        let error = provider
            .catalog_snapshot(&profile())
            .await
            .expect_err("closed port is a transport failure");

        assert_eq!(error.code(), ErrorCode::ProviderUnavailable);
        assert_eq!(error.message(), "Kinsta API request failed");
    }

    #[test]
    fn endpoint_builder_percent_encodes_opaque_ids_as_path_segments() {
        let transport = Transport::new(
            "safe-token",
            Url::parse("https://example.test/v2").expect("test URL is valid"),
            build_http_client().expect("HTTP client initializes"),
        )
        .expect("transport initializes");

        let url = transport
            .endpoint(&["sites", "site/with space"])
            .expect("endpoint builds");
        assert_eq!(
            url.as_str(),
            "https://example.test/v2/sites/site%2Fwith%20space"
        );
    }

    fn plugin_item(
        slug: &str,
        title: &str,
        environment_id: &str,
        status: &str,
    ) -> serde_json::Value {
        json!({
            "name": slug,
            "title": title,
            "description": "Synthetic plugin fixture",
            "latest_version": "2.0.0",
            "is_latest_version_vulnerable": false,
            "environment_count": 1,
            "update_count": 1,
            "environments": [{
                "id": environment_id,
                "site_display_name": "First Site",
                "display_name": "Live",
                "plugin_status": status,
                "plugin_update": "available",
                "plugin_version": "1.0.0",
                "is_plugin_version_vulnerable": true,
                "plugin_update_version": "2.0.0",
                "is_plugin_update_version_vulnerable": false,
                "plugin_update_status": "pending",
                "auto_update_type": "Future Beta Channel"
            }]
        })
    }

    fn theme_item(slug: &str, title: &str, environment_id: &str) -> serde_json::Value {
        json!({
            "name": slug,
            "title": title,
            "description": "Synthetic theme fixture",
            "latest_version": null,
            "is_latest_version_vulnerable": false,
            "environment_count": 1,
            "update_count": 0,
            "environments": [{
                "id": environment_id,
                "site_display_name": "First Site",
                "display_name": "Live",
                "theme_status": "active",
                "theme_update": null,
                "theme_version": "1.0.0",
                "is_theme_version_vulnerable": false,
                "theme_update_version": null,
                "is_theme_update_version_vulnerable": false,
                "theme_update_status": null,
                "auto_update_type": null
            }]
        })
    }
}
