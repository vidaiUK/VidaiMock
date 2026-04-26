/*
 * Copyright (c) 2025 Vidai UK.
 * Author: n@gu
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 *
 * VidaiMock: High-performance LLM API Mock Server.
 */

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::{ChaosConfig, LatencyConfig};

pub const DEFAULT_TENANT_ID: &str = "default";
const TENANT_METADATA_FILE_NAME: &str = "tenant.toml";

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum TenancyMode {
    #[default]
    Single,
    Multi,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default, PartialEq, Eq)]
pub struct TenancyConfig {
    #[serde(default)]
    pub mode: TenancyMode,
    #[serde(default = "default_tenants_dir")]
    pub tenants_dir: PathBuf,
    #[serde(default = "default_tenant_header")]
    pub tenant_header: String,
    #[serde(default)]
    pub admin_auth: AdminAuthConfig,
}

fn default_tenants_dir() -> PathBuf {
    PathBuf::from("tenants")
}

fn default_tenant_header() -> String {
    "x-tenant".to_string()
}

fn default_admin_auth_header() -> String {
    "x-admin-key".to_string()
}

fn default_tenant_management_auth_header() -> String {
    "x-tenant-admin-key".to_string()
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
pub struct AdminAuthConfig {
    #[serde(default = "default_admin_auth_header")]
    pub header: String,
    #[serde(default, skip_serializing)]
    pub value: String,
    #[serde(default, skip_serializing)]
    pub value_file: Option<PathBuf>,
    #[serde(default, skip_serializing)]
    pub value_env: Option<String>,
}

impl Default for AdminAuthConfig {
    fn default() -> Self {
        Self {
            header: default_admin_auth_header(),
            value: String::new(),
            value_file: None,
            value_env: None,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
pub struct TenantManagementAuthConfig {
    #[serde(default = "default_tenant_management_auth_header")]
    pub header: String,
    #[serde(default, skip_serializing)]
    pub value: String,
    #[serde(default, skip_serializing)]
    pub value_file: Option<PathBuf>,
    #[serde(default, skip_serializing)]
    pub value_env: Option<String>,
}

impl Default for TenantManagementAuthConfig {
    fn default() -> Self {
        Self {
            header: default_tenant_management_auth_header(),
            value: String::new(),
            value_file: None,
            value_env: None,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, Default, PartialEq)]
pub struct TenantConfig {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub labels: HashMap<String, String>,
    #[serde(default)]
    pub keys: Vec<TenantKeyConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub management_auth: Option<TenantManagementAuthConfig>,
    #[serde(default)]
    pub latency: Option<TenantLatencyPolicy>,
    #[serde(default)]
    pub chaos: Option<TenantChaosPolicy>,
}

#[derive(Debug, Serialize, Clone, Default, PartialEq, Eq)]
pub struct TenantTemplateMetadata {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub labels: HashMap<String, String>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default, PartialEq)]
pub struct TenantLatencyPolicy {
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub base_ms: Option<u64>,
    #[serde(default)]
    pub jitter_pct: Option<f64>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default, PartialEq)]
pub struct TenantChaosPolicy {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub malformed_pct: Option<f64>,
    #[serde(default)]
    pub drop_pct: Option<f64>,
    #[serde(default)]
    pub trickle_ms: Option<u64>,
    #[serde(default)]
    pub disconnect_pct: Option<f64>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default, PartialEq, Eq, Hash)]
pub struct TenantKeyConfig {
    pub source: TenantKeySource,
    pub name: String,
    #[serde(default, skip_serializing)]
    pub value: String,
    #[serde(default, skip_serializing)]
    pub value_file: Option<PathBuf>,
    #[serde(default, skip_serializing)]
    pub value_env: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum TenantKeySource {
    #[default]
    Header,
    Query,
    /// Parsed from config for compatibility, but rejected by validation in this implementation.
    Host,
    /// Parsed from config for compatibility, but rejected by validation in this implementation.
    Path,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantSchema {
    pub tenant_id: Option<String>,
    pub root_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq)]
struct LoadedTenantConfig {
    dir_name: String,
    metadata_path: PathBuf,
    tenant: TenantConfig,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct DiscoveredTenants {
    pub(crate) default_tenant: Option<TenantConfig>,
    pub(crate) named_tenants: Vec<TenantConfig>,
}

impl TenancyConfig {
    pub fn normalize_secret_paths(&mut self, base_dir: &Path) {
        self.admin_auth.normalize_value_file(base_dir);
    }

    pub fn validate(&self) -> Result<(), String> {
        self.admin_auth.validate()?;
        if self.mode == TenancyMode::Multi {
            let discovered = self.load_multi_mode_tenants()?;
            self.validate_lookup_uniqueness(&discovered.named_tenants)?;
            self.validate_management_auth_uniqueness(
                discovered.default_tenant.as_ref(),
                &discovered.named_tenants,
            )?;
        }
        Ok(())
    }

    pub fn schema_roots(&self, single_config_dir: &Path) -> Result<Vec<TenantSchema>, String> {
        match self.mode {
            TenancyMode::Single => Ok(vec![TenantSchema {
                tenant_id: None,
                root_dir: single_config_dir.to_path_buf(),
            }]),
            TenancyMode::Multi => {
                let discovered = self.load_multi_mode_tenants()?;
                let mut schemas = vec![TenantSchema {
                    tenant_id: Some(DEFAULT_TENANT_ID.to_string()),
                    root_dir: self.tenants_dir.join(DEFAULT_TENANT_ID),
                }];
                schemas.extend(
                    discovered
                        .named_tenants
                        .into_iter()
                        .map(|tenant| TenantSchema {
                            tenant_id: Some(tenant.id.clone()),
                            root_dir: self.tenants_dir.join(&tenant.id),
                        }),
                );
                Ok(schemas)
            }
        }
    }

    pub fn normalized_tenant_header(&self) -> String {
        normalize_name(&self.tenant_header)
    }

    pub fn discover_named_tenants(&self) -> Result<Vec<TenantConfig>, String> {
        Ok(self.load_multi_mode_tenants()?.named_tenants)
    }

    pub(crate) fn load_discovered_tenants(&self) -> Result<DiscoveredTenants, String> {
        self.load_multi_mode_tenants()
    }

    pub(crate) fn load_default_tenant(&self) -> Result<Option<TenantConfig>, String> {
        Ok(self.load_multi_mode_tenants()?.default_tenant)
    }

    pub fn load_named_tenant(&self, tenant_id: &str) -> Result<TenantConfig, String> {
        let normalized_id = normalize_header_value(tenant_id);
        if normalized_id.is_empty() {
            return Err("tenant id must be non-empty".to_string());
        }

        if normalized_id == DEFAULT_TENANT_ID {
            return Err(
                "default tenant metadata is reserved for the internal fallback tenant".to_string(),
            );
        }

        let root_dir = self.tenants_dir.join(tenant_id);
        let metadata_path = root_dir.join(TENANT_METADATA_FILE_NAME);
        if !metadata_path.is_file() {
            return Err(format!("unknown tenant '{}'", tenant_id));
        }

        let loaded = LoadedTenantConfig {
            dir_name: tenant_id.to_string(),
            metadata_path: metadata_path.clone(),
            tenant: load_tenant_metadata_file(&metadata_path)?,
        };

        let discovered = self.classify_loaded_tenants(vec![loaded])?;
        discovered
            .named_tenants
            .into_iter()
            .next()
            .ok_or_else(|| format!("unknown tenant '{}'", tenant_id))
    }

    fn load_multi_mode_tenants(&self) -> Result<DiscoveredTenants, String> {
        let loaded = self.read_tenant_metadata_files()?;
        self.classify_loaded_tenants(loaded)
    }

    fn read_tenant_metadata_files(&self) -> Result<Vec<LoadedTenantConfig>, String> {
        if self.mode != TenancyMode::Multi || !self.tenants_dir.exists() {
            return Ok(Vec::new());
        }

        let mut entries = fs::read_dir(&self.tenants_dir)
            .map_err(|error| {
                format!(
                    "failed to read tenants directory '{}': {}",
                    self.tenants_dir.display(),
                    error
                )
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| {
                format!(
                    "failed to enumerate tenants directory '{}': {}",
                    self.tenants_dir.display(),
                    error
                )
            })?;
        entries.sort_by_key(|entry| entry.file_name());

        let mut loaded = Vec::new();
        for entry in entries {
            let file_type = entry.file_type().map_err(|error| {
                format!(
                    "failed to inspect tenants entry '{}': {}",
                    entry.path().display(),
                    error
                )
            })?;
            if !file_type.is_dir() {
                continue;
            }

            let root_dir = entry.path();
            let metadata_path = root_dir.join(TENANT_METADATA_FILE_NAME);
            if !metadata_path.is_file() {
                continue;
            }

            let dir_name = entry.file_name().into_string().map_err(|_| {
                format!(
                    "tenant directory '{}' is not valid UTF-8",
                    root_dir.display()
                )
            })?;
            let tenant = load_tenant_metadata_file(&metadata_path)?;
            loaded.push(LoadedTenantConfig {
                dir_name,
                metadata_path,
                tenant,
            });
        }

        Ok(loaded)
    }

    fn classify_loaded_tenants(
        &self,
        loaded_tenants: Vec<LoadedTenantConfig>,
    ) -> Result<DiscoveredTenants, String> {
        self.validate_duplicate_tenant_ids(&loaded_tenants)?;

        let mut default_tenant = None;
        let mut named_tenants = Vec::new();

        for loaded in loaded_tenants {
            if let Some(management_auth) = loaded.tenant.management_auth.as_ref() {
                management_auth.validate()?;
            }

            let tenant_id = loaded.tenant.id.trim();
            let normalized_id = normalize_header_value(tenant_id);
            let normalized_dir = normalize_header_value(&loaded.dir_name);

            if normalized_id == DEFAULT_TENANT_ID {
                if normalized_dir != DEFAULT_TENANT_ID {
                    return Err(format!(
                        "tenant metadata '{}' uses reserved id 'default'; only tenants/default/tenant.toml may declare the internal default tenant",
                        loaded.metadata_path.display()
                    ));
                }
                if !loaded.tenant.keys.is_empty() {
                    return Err(format!(
                        "default tenant metadata '{}' cannot declare tenant lookup keys because the default tenant is fallback-only",
                        loaded.metadata_path.display()
                    ));
                }
                default_tenant = Some(loaded.tenant);
                continue;
            }

            // Tenant-owned metadata lives beside that tenant's providers/templates
            // so each workspace is self-describing on disk in multi mode.
            if normalized_dir == DEFAULT_TENANT_ID {
                return Err(format!(
                    "tenant metadata '{}' must use id 'default' when stored under tenants/default/",
                    loaded.metadata_path.display()
                ));
            }

            if normalized_dir != normalized_id {
                return Err(format!(
                    "tenant metadata '{}' declares id '{}' but lives under tenants/{}/; tenant metadata must live in tenants/<id>/tenant.toml",
                    loaded.metadata_path.display(),
                    tenant_id,
                    loaded.dir_name
                ));
            }

            self.validate_supported_key_sources(&loaded.tenant)?;
            named_tenants.push(loaded.tenant);
        }

        named_tenants.sort_by(|left, right| left.id.cmp(&right.id));

        Ok(DiscoveredTenants {
            default_tenant,
            named_tenants,
        })
    }

    fn validate_duplicate_tenant_ids(
        &self,
        loaded_tenants: &[LoadedTenantConfig],
    ) -> Result<(), String> {
        let mut seen = HashSet::new();

        for loaded in loaded_tenants {
            let tenant_id = loaded.tenant.id.trim();
            if tenant_id.is_empty() {
                return Err(format!(
                    "tenant metadata '{}' must include a non-empty id",
                    loaded.metadata_path.display()
                ));
            }

            let normalized_id = normalize_header_value(tenant_id);
            if !seen.insert(normalized_id) {
                return Err(format!("duplicate tenant id: '{}'", tenant_id));
            }
        }

        Ok(())
    }

    fn validate_lookup_uniqueness(&self, named_tenants: &[TenantConfig]) -> Result<(), String> {
        let tenant_header = self.normalized_tenant_header();
        let mut seen_header_values: HashMap<NormalizedLookupKey, String> = HashMap::new();
        let mut seen_api_keys: HashMap<NormalizedLookupKey, String> = HashMap::new();

        for tenant in named_tenants {
            let implicit_header = NormalizedLookupKey {
                source: TenantKeySource::Header,
                name: tenant_header.clone(),
                value: normalize_header_value(tenant.id.trim()),
            };
            register_lookup(
                &mut seen_header_values,
                implicit_header,
                tenant,
                format!("Header '{}'='{}'", self.tenant_header, tenant.id),
            )?;

            for key in &tenant.keys {
                if !key.supports_runtime_resolution(&tenant_header) {
                    return Err(format!(
                        "unsupported tenant key source {:?} for tenant '{}'; only header and query are supported",
                        key.source, tenant.id,
                    ));
                }

                let resolved_value = key.resolved_value()?;
                let normalized = if key.is_tenant_header_key(&tenant_header) {
                    NormalizedLookupKey {
                        source: TenantKeySource::Header,
                        name: tenant_header.clone(),
                        value: normalize_header_value(&resolved_value),
                    }
                } else {
                    NormalizedLookupKey {
                        source: key.source.clone(),
                        name: normalize_name(&key.name),
                        value: normalize_secret_value(&resolved_value),
                    }
                };

                if key.is_tenant_header_key(&tenant_header) {
                    register_lookup(
                        &mut seen_header_values,
                        normalized,
                        tenant,
                        key.describe(),
                    )?;
                } else {
                    register_lookup(
                        &mut seen_api_keys,
                        normalized,
                        tenant,
                        key.describe(),
                    )?;
                }
            }
        }

        Ok(())
    }

    fn validate_management_auth_uniqueness(
        &self,
        default_tenant: Option<&TenantConfig>,
        named_tenants: &[TenantConfig],
    ) -> Result<(), String> {
        let mut seen_identities: HashMap<ManagementAuthIdentity, String> = HashMap::new();

        if let Some(default_tenant) = default_tenant {
            register_management_auth_identity(&mut seen_identities, default_tenant)?;
        }

        for tenant in named_tenants {
            register_management_auth_identity(&mut seen_identities, tenant)?;
        }

        Ok(())
    }

    fn validate_supported_key_sources(&self, tenant: &TenantConfig) -> Result<(), String> {
        let tenant_header = self.normalized_tenant_header();
        for key in &tenant.keys {
            if !key.supports_runtime_resolution(&tenant_header) {
                return Err(format!(
                    "unsupported tenant key source {:?} for tenant '{}'; only header and query are supported",
                    key.source, tenant.id,
                ));
            }
        }

        Ok(())
    }
}

impl AdminAuthConfig {
    pub fn normalize_value_file(&mut self, base_dir: &Path) {
        normalize_relative_secret_path(&mut self.value_file, base_dir);
    }

    pub fn resolved_value(&self) -> Result<Option<String>, String> {
        if self.value_file.is_none() && self.value_env.is_none() && self.value.trim().is_empty() {
            return Ok(None);
        }

        resolve_secret_value(
            "admin auth",
            &self.header,
            &self.value,
            self.value_file.as_ref(),
            self.value_env.as_deref(),
        )
        .map(Some)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.header.trim().is_empty() {
            return Err("tenancy.admin_auth.header must be non-empty".to_string());
        }

        self.resolved_value().map(|_| ())
    }
}

impl TenantManagementAuthConfig {
    pub fn normalize_value_file(&mut self, base_dir: &Path) {
        normalize_relative_secret_path(&mut self.value_file, base_dir);
    }

    pub fn resolved_value(&self) -> Result<Option<String>, String> {
        if self.value_file.is_none() && self.value_env.is_none() && self.value.trim().is_empty() {
            return Ok(None);
        }

        resolve_secret_value(
            "tenant management auth",
            &self.header,
            &self.value,
            self.value_file.as_ref(),
            self.value_env.as_deref(),
        )
        .map(Some)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.header.trim().is_empty() {
            return Err("tenant management_auth.header must be non-empty".to_string());
        }

        self.resolved_value().map(|_| ())
    }

    pub fn normalized_header(&self) -> String {
        normalize_name(&self.header)
    }
}

impl TenantConfig {
    pub fn normalize_secret_paths(&mut self, base_dir: &Path) {
        for key in &mut self.keys {
            key.normalize_value_file(base_dir);
        }

        if let Some(management_auth) = self.management_auth.as_mut() {
            management_auth.normalize_value_file(base_dir);
        }
    }

    pub fn template_metadata(&self) -> TenantTemplateMetadata {
        // Template context gets only safe tenant metadata, never auth or key material.
        TenantTemplateMetadata {
            id: self.id.clone(),
            display_name: self.display_name.clone(),
            labels: self.labels.clone(),
        }
    }

    pub fn requires_key(&self, tenant_header: &str) -> bool {
        self.keys
            .iter()
            .any(|key| !key.is_tenant_header_key(tenant_header))
    }

    pub fn explicit_header_values(&self, tenant_header: &str) -> Result<Vec<String>, String> {
        let mut values = Vec::new();

        for key in &self.keys {
            if key.is_tenant_header_key(tenant_header) {
                values.push(key.resolved_value()?);
            }
        }

        Ok(values)
    }

    pub fn api_keys(&self, tenant_header: &str) -> Result<Vec<ResolvedTenantKey>, String> {
        let mut keys = Vec::new();

        for key in &self.keys {
            if key.is_tenant_header_key(tenant_header) {
                continue;
            }

            keys.push(ResolvedTenantKey {
                source: key.source.clone(),
                name: normalize_name(&key.name),
                value: normalize_secret_value(&key.resolved_value()?),
            });
        }

        Ok(keys)
    }

    pub fn resolved_management_auth(&self) -> Result<Option<ResolvedTenantManagementAuth>, String> {
        let Some(management_auth) = self.management_auth.as_ref() else {
            return Ok(None);
        };

        Ok(management_auth
            .resolved_value()?
            .map(|secret| ResolvedTenantManagementAuth {
                header: management_auth.normalized_header(),
                secret,
            }))
    }

    pub fn effective_latency(&self, global: &LatencyConfig) -> LatencyConfig {
        self.latency
            .as_ref()
            .map(|policy| policy.apply_to(global))
            .unwrap_or_else(|| global.clone())
    }

    pub fn effective_chaos(&self, global: &ChaosConfig) -> ChaosConfig {
        self.chaos
            .as_ref()
            .map(|policy| policy.apply_to(global))
            .unwrap_or_else(|| global.clone())
    }
}

impl TenantLatencyPolicy {
    pub fn apply_to(&self, global: &LatencyConfig) -> LatencyConfig {
        LatencyConfig {
            mode: self.mode.clone().unwrap_or_else(|| global.mode.clone()),
            base_ms: self.base_ms.unwrap_or(global.base_ms),
            jitter_pct: self.jitter_pct.unwrap_or(global.jitter_pct),
        }
    }
}

impl TenantChaosPolicy {
    pub fn apply_to(&self, global: &ChaosConfig) -> ChaosConfig {
        ChaosConfig {
            enabled: self.enabled.unwrap_or(global.enabled),
            malformed_pct: self.malformed_pct.unwrap_or(global.malformed_pct),
            drop_pct: self.drop_pct.unwrap_or(global.drop_pct),
            trickle_ms: self.trickle_ms.unwrap_or(global.trickle_ms),
            disconnect_pct: self.disconnect_pct.unwrap_or(global.disconnect_pct),
        }
    }
}

impl TenantKeyConfig {
    pub fn normalize_value_file(&mut self, base_dir: &Path) {
        normalize_relative_secret_path(&mut self.value_file, base_dir);
    }

    pub fn supports_runtime_resolution(&self, tenant_header: &str) -> bool {
        matches!(
            self.source,
            TenantKeySource::Header | TenantKeySource::Query
        ) || self.is_tenant_header_key(tenant_header)
    }

    pub fn is_tenant_header_key(&self, tenant_header: &str) -> bool {
        matches!(self.source, TenantKeySource::Header)
            && normalize_name(&self.name) == normalize_name(tenant_header)
    }

    pub fn resolved_value(&self) -> Result<String, String> {
        resolve_secret_value(
            "tenant key",
            &self.name,
            &self.value,
            self.value_file.as_ref(),
            self.value_env.as_deref(),
        )
    }

    fn describe(&self) -> String {
        format!("{:?} '{}'", self.source, self.name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResolvedTenantKey {
    pub source: TenantKeySource,
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTenantManagementAuth {
    pub header: String,
    pub secret: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ManagementAuthIdentity {
    header: String,
    secret: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct NormalizedLookupKey {
    source: TenantKeySource,
    name: String,
    value: String,
}

fn register_lookup(
    seen: &mut HashMap<NormalizedLookupKey, String>,
    normalized: NormalizedLookupKey,
    tenant: &TenantConfig,
    description: String,
) -> Result<(), String> {
    if let Some(existing_tenant_id) = seen.get(&normalized) {
        if existing_tenant_id == &tenant.id {
            return Err(format!(
                "duplicate tenant key definition for tenant '{}' on {}",
                tenant.id, description
            ));
        }

        return Err(format!(
            "ambiguous tenant key match between '{}' and '{}' on {}",
            existing_tenant_id, tenant.id, description
        ));
    }

    seen.insert(normalized, tenant.id.clone());
    Ok(())
}

fn register_management_auth_identity(
    seen: &mut HashMap<ManagementAuthIdentity, String>,
    tenant: &TenantConfig,
) -> Result<(), String> {
    let Some(management_auth) = tenant.resolved_management_auth()? else {
        return Ok(());
    };

    // Management auth is keyed by the effective header+secret identity.
    // That lets operators reuse the same secret text under different headers
    // without making two tenants resolve from the same incoming credential.
    let identity = ManagementAuthIdentity {
        header: management_auth.header.clone(),
        secret: management_auth.secret,
    };

    if let Some(existing_tenant_id) = seen.get(&identity) {
        return Err(format!(
            "duplicate tenant management_auth identity between '{}' and '{}' on header '{}'; tenant-admin credentials must be unique per header+secret combination",
            existing_tenant_id, tenant.id, management_auth.header
        ));
    }

    seen.insert(identity, tenant.id.clone());
    Ok(())
}

fn normalize_name(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

fn normalize_header_value(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn normalize_secret_value(value: &str) -> String {
    value.trim().to_string()
}

fn resolve_secret_value(
    kind: &str,
    name: &str,
    inline_value: &str,
    value_file: Option<&PathBuf>,
    value_env: Option<&str>,
) -> Result<String, String> {
    let value = if let Some(path) = value_file {
        fs::read_to_string(path).map_err(|error| {
            format!(
                "failed to read {kind} secret file '{}': {}",
                path.display(),
                error
            )
        })?
    } else if let Some(env_name) = value_env {
        std::env::var(env_name).map_err(|error| {
            format!("failed to read {kind} secret env '{}': {}", env_name, error)
        })?
    } else {
        inline_value.to_string()
    };

    let trimmed = value.trim().to_string();
    if trimmed.is_empty() {
        return Err(format!(
            "{kind} '{}' must resolve to a non-empty value",
            name
        ));
    }

    Ok(trimmed)
}

fn load_tenant_metadata_file(path: &Path) -> Result<TenantConfig, String> {
    let mut tenant: TenantConfig = config::Config::builder()
        .add_source(config::File::from(path.to_path_buf()))
        .build()
        .map_err(|error| {
            format!(
                "failed to load tenant metadata '{}': {}",
                path.display(),
                error
            )
        })?
        .try_deserialize()
        .map_err(|error| {
            format!(
                "failed to parse tenant metadata '{}': {}",
                path.display(),
                error
            )
        })?;
    tenant.normalize_secret_paths(path.parent().unwrap_or_else(|| Path::new(".")));
    Ok(tenant)
}

fn normalize_relative_secret_path(path: &mut Option<PathBuf>, base_dir: &Path) {
    if let Some(path_buf) = path.as_mut() {
        if path_buf.is_relative() {
            *path_buf = base_dir.join(path_buf.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_test_dir(name: &str) -> PathBuf {
        std::env::current_dir().unwrap().join(format!(
            "target/{}_{}",
            name,
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn write_tenant_metadata(base_dir: &Path, tenant_dir: &str, body: &str) {
        let metadata_path = base_dir
            .join("tenants")
            .join(tenant_dir)
            .join(TENANT_METADATA_FILE_NAME);
        fs::create_dir_all(metadata_path.parent().unwrap()).unwrap();
        fs::write(metadata_path, body).unwrap();
    }

    #[test]
    fn single_mode_uses_config_dir_schema() {
        let tenancy = TenancyConfig::default();
        let schemas = tenancy.schema_roots(Path::new("config")).unwrap();

        assert_eq!(
            schemas,
            vec![TenantSchema {
                tenant_id: None,
                root_dir: PathBuf::from("config"),
            }]
        );
    }

    #[test]
    fn multi_mode_uses_tenants_dir_schema_with_internal_default() {
        let temp_base = unique_test_dir("tenancy_schema_roots");
        write_tenant_metadata(
            &temp_base,
            "acme",
            r#"
id = "acme"
"#,
        );
        write_tenant_metadata(
            &temp_base,
            "globex",
            r#"
id = "globex"
"#,
        );

        let tenancy = TenancyConfig {
            mode: TenancyMode::Multi,
            tenants_dir: temp_base.join("tenants"),
            tenant_header: default_tenant_header(),
            admin_auth: AdminAuthConfig::default(),
        };

        let schemas = tenancy.schema_roots(Path::new("config")).unwrap();

        assert_eq!(
            schemas,
            vec![
                TenantSchema {
                    tenant_id: Some(DEFAULT_TENANT_ID.to_string()),
                    root_dir: temp_base.join("tenants/default"),
                },
                TenantSchema {
                    tenant_id: Some("acme".to_string()),
                    root_dir: temp_base.join("tenants/acme"),
                },
                TenantSchema {
                    tenant_id: Some("globex".to_string()),
                    root_dir: temp_base.join("tenants/globex"),
                },
            ]
        );

        fs::remove_dir_all(temp_base).unwrap();
    }

    #[test]
    fn duplicate_tenant_ids_are_rejected() {
        let temp_base = unique_test_dir("duplicate_tenant_ids");
        write_tenant_metadata(
            &temp_base,
            "acme",
            r#"
id = "acme"
"#,
        );
        write_tenant_metadata(
            &temp_base,
            "globex",
            r#"
id = "acme"
"#,
        );

        let tenancy = TenancyConfig {
            mode: TenancyMode::Multi,
            tenants_dir: temp_base.join("tenants"),
            tenant_header: default_tenant_header(),
            admin_auth: AdminAuthConfig::default(),
        };

        let error = tenancy.validate().unwrap_err();
        assert!(error.contains("duplicate tenant id"));

        fs::remove_dir_all(temp_base).unwrap();
    }

    #[test]
    fn ambiguous_tenant_keys_are_rejected() {
        let temp_base = unique_test_dir("ambiguous_tenant_keys");
        write_tenant_metadata(
            &temp_base,
            "acme",
            r#"
id = "acme"

[[keys]]
source = "header"
name = "X-API-Key"
value = "shared"
"#,
        );
        write_tenant_metadata(
            &temp_base,
            "globex",
            r#"
id = "globex"

[[keys]]
source = "header"
name = "x-api-key"
value = "shared"
"#,
        );

        let tenancy = TenancyConfig {
            mode: TenancyMode::Multi,
            tenants_dir: temp_base.join("tenants"),
            tenant_header: default_tenant_header(),
            admin_auth: AdminAuthConfig::default(),
        };

        let error = tenancy.validate().unwrap_err();
        assert!(error.contains("ambiguous tenant key match"));

        fs::remove_dir_all(temp_base).unwrap();
    }

    #[test]
    fn ambiguous_tenant_key_errors_redact_secret_values() {
        let temp_base = unique_test_dir("ambiguous_tenant_keys_redacted");
        write_tenant_metadata(
            &temp_base,
            "acme",
            r#"
id = "acme"

[[keys]]
source = "header"
name = "x-api-key"
value = "shared-secret"
"#,
        );
        write_tenant_metadata(
            &temp_base,
            "globex",
            r#"
id = "globex"

[[keys]]
source = "header"
name = "x-api-key"
value = "shared-secret"
"#,
        );

        let tenancy = TenancyConfig {
            mode: TenancyMode::Multi,
            tenants_dir: temp_base.join("tenants"),
            tenant_header: default_tenant_header(),
            admin_auth: AdminAuthConfig::default(),
        };

        let error = tenancy.validate().unwrap_err();
        assert!(error.contains("x-api-key"));
        assert!(!error.contains("shared-secret"));

        fs::remove_dir_all(temp_base).unwrap();
    }

    #[test]
    fn implicit_and_explicit_header_alias_conflicts_are_rejected() {
        let temp_base = unique_test_dir("header_alias_conflicts");
        write_tenant_metadata(
            &temp_base,
            "acme",
            r#"
id = "acme"
"#,
        );
        write_tenant_metadata(
            &temp_base,
            "globex",
            r#"
id = "globex"

[[keys]]
source = "header"
name = "x-tenant"
value = "acme"
"#,
        );

        let tenancy = TenancyConfig {
            mode: TenancyMode::Multi,
            tenants_dir: temp_base.join("tenants"),
            tenant_header: default_tenant_header(),
            admin_auth: AdminAuthConfig::default(),
        };

        let error = tenancy.validate().unwrap_err();
        assert!(error.contains("ambiguous tenant key match"));

        fs::remove_dir_all(temp_base).unwrap();
    }

    #[test]
    fn host_and_path_tenant_key_sources_are_rejected() {
        for source in [TenantKeySource::Host, TenantKeySource::Path] {
            let temp_base = unique_test_dir("unsupported_tenant_key_source");
            let source_name = match source {
                TenantKeySource::Host => "host",
                TenantKeySource::Path => "path",
                _ => unreachable!(),
            };
            write_tenant_metadata(
                &temp_base,
                "acme",
                &format!(
                    r#"
id = "acme"

[[keys]]
source = "{source_name}"
name = "tenant"
value = "acme"
"#
                ),
            );

            let tenancy = TenancyConfig {
                mode: TenancyMode::Multi,
                tenants_dir: temp_base.join("tenants"),
                tenant_header: default_tenant_header(),
                admin_auth: AdminAuthConfig::default(),
            };

            let error = tenancy.validate().unwrap_err();
            assert!(error.contains("only header and query are supported"));

            fs::remove_dir_all(temp_base).unwrap();
        }
    }

    #[test]
    fn duplicate_tenant_management_auth_identities_are_rejected() {
        let temp_base = unique_test_dir("duplicate_tenant_management_auth");
        write_tenant_metadata(
            &temp_base,
            "acme",
            r#"
id = "acme"

[management_auth]
header = "x-tenant-admin-key"
value = "shared-admin"
"#,
        );
        write_tenant_metadata(
            &temp_base,
            "globex",
            r#"
id = "globex"

[management_auth]
header = "x-tenant-admin-key"
value = "shared-admin"
"#,
        );

        let tenancy = TenancyConfig {
            mode: TenancyMode::Multi,
            tenants_dir: temp_base.join("tenants"),
            tenant_header: default_tenant_header(),
            admin_auth: AdminAuthConfig::default(),
        };

        let error = tenancy.validate().unwrap_err();
        assert!(error.contains("duplicate tenant management_auth identity"));

        fs::remove_dir_all(temp_base).unwrap();
    }

    #[test]
    fn same_management_secret_under_different_headers_is_allowed() {
        let temp_base = unique_test_dir("same_management_secret_different_headers");
        write_tenant_metadata(
            &temp_base,
            "acme",
            r#"
id = "acme"

[management_auth]
header = "x-tenant-admin-key"
value = "shared-admin"
"#,
        );
        write_tenant_metadata(
            &temp_base,
            "globex",
            r#"
id = "globex"

[management_auth]
header = "x-globex-admin-key"
value = "shared-admin"
"#,
        );

        let tenancy = TenancyConfig {
            mode: TenancyMode::Multi,
            tenants_dir: temp_base.join("tenants"),
            tenant_header: default_tenant_header(),
            admin_auth: AdminAuthConfig::default(),
        };

        tenancy.validate().unwrap();

        fs::remove_dir_all(temp_base).unwrap();
    }

    #[test]
    fn default_tenant_metadata_is_optional_and_fallback_only() {
        let temp_base = unique_test_dir("default_tenant_optional");
        write_tenant_metadata(
            &temp_base,
            "acme",
            r#"
id = "acme"
"#,
        );

        let tenancy = TenancyConfig {
            mode: TenancyMode::Multi,
            tenants_dir: temp_base.join("tenants"),
            tenant_header: default_tenant_header(),
            admin_auth: AdminAuthConfig::default(),
        };

        let discovered = tenancy.discover_named_tenants().unwrap();
        assert_eq!(
            discovered,
            vec![TenantConfig {
                id: "acme".to_string(),
                display_name: None,
                labels: HashMap::new(),
                keys: Vec::new(),
                management_auth: None,
                latency: None,
                chaos: None,
            }]
        );

        fs::remove_dir_all(temp_base).unwrap();
    }

    #[test]
    fn default_tenant_metadata_cannot_define_lookup_keys() {
        let temp_base = unique_test_dir("default_tenant_keys");
        write_tenant_metadata(
            &temp_base,
            DEFAULT_TENANT_ID,
            r#"
id = "default"

[[keys]]
source = "header"
name = "x-api-key"
value = "secret-default"
"#,
        );

        let tenancy = TenancyConfig {
            mode: TenancyMode::Multi,
            tenants_dir: temp_base.join("tenants"),
            tenant_header: default_tenant_header(),
            admin_auth: AdminAuthConfig::default(),
        };

        let error = tenancy.validate().unwrap_err();
        assert!(error.contains("fallback-only"));

        fs::remove_dir_all(temp_base).unwrap();
    }

    #[test]
    fn tenant_management_auth_serialization_omits_secret_fields() {
        let tenant: TenantConfig = serde_json::from_value(serde_json::json!({
            "id": "acme",
            "management_auth": {
                "header": "x-tenant-admin-key",
                "value": "tenant-admin-secret",
                "value_file": "secrets/acme-admin.key",
                "value_env": "ACME_TENANT_ADMIN_KEY"
            }
        }))
        .unwrap();

        let serialized = serde_json::to_value(&tenant).unwrap();
        let management_auth = &serialized["management_auth"];

        assert_eq!(management_auth["header"], "x-tenant-admin-key");
        assert!(management_auth.get("value").is_none());
        assert!(management_auth.get("value_file").is_none());
        assert!(management_auth.get("value_env").is_none());
    }

    #[test]
    fn tenant_value_file_paths_are_resolved_relative_to_tenant_metadata() {
        let temp_base = unique_test_dir("tenant_value_file_relative_to_metadata");
        let secret_path = temp_base.join("tenants/acme/secrets/acme.key");
        fs::create_dir_all(secret_path.parent().unwrap()).unwrap();
        fs::write(&secret_path, "secret-acme").unwrap();
        write_tenant_metadata(
            &temp_base,
            "acme",
            r#"
id = "acme"

[[keys]]
source = "header"
name = "x-api-key"
value_file = "secrets/acme.key"
"#,
        );

        let tenancy = TenancyConfig {
            mode: TenancyMode::Multi,
            tenants_dir: temp_base.join("tenants"),
            tenant_header: default_tenant_header(),
            admin_auth: AdminAuthConfig::default(),
        };

        let tenant = tenancy.load_named_tenant("acme").unwrap();
        assert_eq!(
            tenant.keys[0].value_file.as_ref().unwrap(),
            &temp_base.join("tenants/acme/secrets/acme.key")
        );
        assert_eq!(
            tenant.api_keys(&tenancy.normalized_tenant_header()).unwrap()[0].value,
            "secret-acme"
        );

        fs::remove_dir_all(temp_base).unwrap();
    }
}
