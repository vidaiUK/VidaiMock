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

use arc_swap::ArcSwap;
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::config::{AppConfig, ChaosConfig, LatencyConfig};
use crate::provider::{build_registry_from_layers, ProviderRegistry};

use super::config::{
    AdminAuthConfig, TenancyConfig, TenancyMode, TenantConfig, TenantKeySource,
    TenantTemplateMetadata, DEFAULT_TENANT_ID,
};
use super::resolution::ResolvedRequestKey;

pub struct TenantRuntime {
    pub label: String,
    pub template_metadata: TenantTemplateMetadata,
    pub registry: Arc<ProviderRegistry>,
    pub requires_key: bool,
    pub management_auth_header: String,
    pub management_auth_secret: Option<String>,
    pub latency: LatencyConfig,
    pub chaos: ChaosConfig,
}

pub struct TenantStore {
    pub(crate) mode: TenancyMode,
    pub(crate) config_dir: PathBuf,
    pub(crate) tenancy: TenancyConfig,
    pub(crate) global_latency: LatencyConfig,
    pub(crate) global_chaos: ChaosConfig,
    pub(crate) admin_auth_header: String,
    pub(crate) admin_auth_secret: Option<String>,
    pub(crate) tenant_header_name: String,
    pub(crate) default_tenant: Arc<TenantRuntime>,
    pub(crate) tenants_by_id: HashMap<String, Arc<TenantRuntime>>,
    pub(crate) header_lookup: HashMap<String, String>,
    pub(crate) key_lookup: HashMap<ResolvedRequestKey, String>,
    pub(crate) known_header_key_names: HashSet<String>,
    pub(crate) known_query_key_names: HashSet<String>,
}

pub struct TenantStoreHandle {
    current: ArcSwap<TenantStore>,
    reload_lock: Mutex<()>,
}

impl TenantStore {
    pub fn new(
        mode: TenancyMode,
        config_dir: PathBuf,
        tenancy: TenancyConfig,
        global_latency: LatencyConfig,
        global_chaos: ChaosConfig,
        admin_auth_header: String,
        admin_auth_secret: Option<String>,
        tenant_header_name: String,
        default_tenant: Arc<TenantRuntime>,
        tenants_by_id: HashMap<String, Arc<TenantRuntime>>,
        header_lookup: HashMap<String, String>,
        key_lookup: HashMap<ResolvedRequestKey, String>,
        known_header_key_names: HashSet<String>,
        known_query_key_names: HashSet<String>,
    ) -> Self {
        Self {
            mode,
            config_dir,
            tenancy,
            global_latency,
            global_chaos,
            admin_auth_header,
            admin_auth_secret,
            tenant_header_name,
            default_tenant,
            tenants_by_id,
            header_lookup,
            key_lookup,
            known_header_key_names,
            known_query_key_names,
        }
    }

    pub fn default_tenant(&self) -> Arc<TenantRuntime> {
        self.default_tenant.clone()
    }

    pub fn tenant_by_id(&self, tenant_id: &str) -> Option<Arc<TenantRuntime>> {
        if tenant_id == DEFAULT_TENANT_ID {
            return Some(self.default_tenant());
        }

        self.tenants_by_id.get(tenant_id).cloned()
    }

    fn runtime_config(&self) -> AppConfig {
        AppConfig {
            host: String::new(),
            port: 0,
            workers: 0,
            log_level: String::new(),
            config_dir: self.config_dir.clone(),
            tenancy: self.tenancy.clone(),
            latency: self.global_latency.clone(),
            chaos: self.global_chaos.clone(),
            endpoints: Vec::new(),
            response_file: None,
            reload_args: None,
        }
    }
}

impl TenantStoreHandle {
    pub fn new(initial: Arc<TenantStore>) -> Self {
        Self {
            current: ArcSwap::from(initial),
            reload_lock: Mutex::new(()),
        }
    }

    pub fn current(&self) -> Arc<TenantStore> {
        self.current.load_full()
    }

    pub fn reload_all(&self, config: &AppConfig) -> Result<Arc<TenantStore>, Box<dyn Error>> {
        // Serialize the full rebuild+swap sequence so two successful reloads
        // cannot both build from stale state and then race to overwrite the
        // live store. Reads stay lock-free via ArcSwap::load_full().
        let _guard = self.lock_reload_guard();
        let rebuilt = build_runtime_store(config)?;
        self.current.store(rebuilt.clone());
        Ok(rebuilt)
    }

    pub fn reload_tenant(&self, tenant_id: &str) -> Result<Arc<TenantStore>, Box<dyn Error>> {
        // Tenant reload also participates in the same single-writer guard
        // because it clones and mutates the current store before swapping it
        // back in. Without this, a concurrent reload_all/reload_tenant pair
        // could silently lose one successful update.
        let _guard = self.lock_reload_guard();
        let current = self.current();
        let updated = match current.mode {
            TenancyMode::Single => reload_single_mode_store(&current, tenant_id)?,
            TenancyMode::Multi => reload_multi_mode_tenant(&current, tenant_id)?,
        };

        self.current.store(updated.clone());
        Ok(updated)
    }

    fn lock_reload_guard(&self) -> MutexGuard<'_, ()> {
        self.reload_lock.lock().unwrap_or_else(|poison| poison.into_inner())
    }
}

pub fn build_runtime_store(config: &AppConfig) -> Result<Arc<TenantStore>, Box<dyn Error>> {
    config.validate()?;

    match config.tenancy.mode {
        TenancyMode::Single => build_single_mode_store(config),
        TenancyMode::Multi => build_multi_mode_store(config),
    }
}

fn build_single_mode_store(config: &AppConfig) -> Result<Arc<TenantStore>, Box<dyn Error>> {
    let default_tenant = build_single_mode_default_runtime(config)?;
    let admin_auth = resolve_admin_auth(&config.tenancy.admin_auth)?;

    Ok(Arc::new(TenantStore::new(
        TenancyMode::Single,
        config.config_dir.clone(),
        config.tenancy.clone(),
        config.latency.clone(),
        config.chaos.clone(),
        config.tenancy.admin_auth.header.clone(),
        admin_auth,
        config.tenancy.normalized_tenant_header(),
        default_tenant,
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashSet::new(),
        HashSet::new(),
    )))
}

fn build_multi_mode_store(config: &AppConfig) -> Result<Arc<TenantStore>, Box<dyn Error>> {
    let tenancy = &config.tenancy;
    let tenant_header = tenancy.normalized_tenant_header();
    let discovered_tenants = tenancy.load_discovered_tenants()?;
    let default_tenant =
        build_multi_mode_default_runtime(config, discovered_tenants.default_tenant.as_ref())?;
    let tenants_by_id = build_named_tenant_runtimes(
        config,
        tenancy,
        &discovered_tenants.named_tenants,
        &tenant_header,
    )?;
    let lookup_state =
        build_lookup_state(tenancy, &discovered_tenants.named_tenants, &tenant_header)?;
    let admin_auth = resolve_admin_auth(&tenancy.admin_auth)?;

    Ok(Arc::new(TenantStore::new(
        TenancyMode::Multi,
        config.config_dir.clone(),
        tenancy.clone(),
        config.latency.clone(),
        config.chaos.clone(),
        tenancy.admin_auth.header.clone(),
        admin_auth,
        tenant_header,
        default_tenant,
        tenants_by_id,
        lookup_state.header_lookup,
        lookup_state.key_lookup,
        lookup_state.known_header_key_names,
        lookup_state.known_query_key_names,
    )))
}

fn build_single_mode_default_runtime(
    config: &AppConfig,
) -> Result<Arc<TenantRuntime>, Box<dyn Error>> {
    build_single_mode_default_runtime_from_path(config)
}

fn build_single_mode_default_runtime_from_path(
    config: &AppConfig,
) -> Result<Arc<TenantRuntime>, Box<dyn Error>> {
    let registry = build_registry_from_layers(&[config.config_dir.as_path()])?;
    Ok(Arc::new(TenantRuntime {
        label: DEFAULT_TENANT_ID.to_string(),
        template_metadata: TenantTemplateMetadata {
            id: DEFAULT_TENANT_ID.to_string(),
            ..TenantTemplateMetadata::default()
        },
        registry,
        requires_key: false,
        management_auth_header: "x-tenant-admin-key".to_string(),
        management_auth_secret: None,
        latency: config.latency.clone(),
        chaos: config.chaos.clone(),
    }))
}

fn build_multi_mode_default_runtime(
    config: &AppConfig,
    tenant_config: Option<&TenantConfig>,
) -> Result<Arc<TenantRuntime>, Box<dyn Error>> {
    build_multi_mode_default_runtime_from_tenancy(config, tenant_config)
}

fn build_multi_mode_default_runtime_from_tenancy(
    config: &AppConfig,
    tenant_config: Option<&TenantConfig>,
) -> Result<Arc<TenantRuntime>, Box<dyn Error>> {
    let default_root = config.tenancy.tenants_dir.join(DEFAULT_TENANT_ID);
    let registry = build_registry_from_layers(&[default_root.as_path()])?;
    let (management_auth_header, management_auth_secret) =
        resolve_tenant_management_auth(tenant_config)?;
    Ok(Arc::new(TenantRuntime {
        label: DEFAULT_TENANT_ID.to_string(),
        template_metadata: tenant_config
            .map(TenantConfig::template_metadata)
            .unwrap_or_else(|| TenantTemplateMetadata {
                id: DEFAULT_TENANT_ID.to_string(),
                ..TenantTemplateMetadata::default()
            }),
        registry,
        requires_key: false,
        management_auth_header,
        management_auth_secret,
        latency: tenant_config
            .map(|tenant| tenant.effective_latency(&config.latency))
            .unwrap_or_else(|| config.latency.clone()),
        chaos: tenant_config
            .map(|tenant| tenant.effective_chaos(&config.chaos))
            .unwrap_or_else(|| config.chaos.clone()),
    }))
}

fn reload_single_mode_store(
    current: &Arc<TenantStore>,
    tenant_id: &str,
) -> Result<Arc<TenantStore>, Box<dyn Error>> {
    if tenant_id != DEFAULT_TENANT_ID {
        return Err(format!("unknown tenant '{}'", tenant_id).into());
    }

    let config = current.runtime_config();
    let default_tenant = build_single_mode_default_runtime_from_path(&config)?;

    Ok(Arc::new(TenantStore::new(
        TenancyMode::Single,
        current.config_dir.clone(),
        current.tenancy.clone(),
        current.global_latency.clone(),
        current.global_chaos.clone(),
        current.admin_auth_header.clone(),
        current.admin_auth_secret.clone(),
        current.tenant_header_name.clone(),
        default_tenant,
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashSet::new(),
        HashSet::new(),
    )))
}

fn reload_multi_mode_tenant(
    current: &Arc<TenantStore>,
    tenant_id: &str,
) -> Result<Arc<TenantStore>, Box<dyn Error>> {
    let tenancy = &current.tenancy;
    let tenant_header = tenancy.normalized_tenant_header();
    let config = current.runtime_config();
    let mut tenants_by_id = current.tenants_by_id.clone();
    let default_tenant = if tenant_id == DEFAULT_TENANT_ID {
        let default_tenant_config = tenancy.load_default_tenant()?;
        let runtime =
            build_multi_mode_default_runtime_from_tenancy(&config, default_tenant_config.as_ref())?;
        validate_management_auth_uniqueness_for_reload(current, tenant_id, &runtime)?;
        runtime
    } else {
        current.default_tenant()
    };
    let mut header_lookup = current.header_lookup.clone();
    let mut key_lookup = current.key_lookup.clone();

    if tenant_id != DEFAULT_TENANT_ID {
        let tenant_config = tenancy.load_named_tenant(tenant_id)?;
        let runtime = build_named_tenant_runtime(&config, tenancy, &tenant_config, &tenant_header)?;
        validate_management_auth_uniqueness_for_reload(current, tenant_id, &runtime)?;

        validate_and_refresh_tenant_lookup_entries(
            &tenant_config,
            tenant_id,
            &tenant_header,
            &mut header_lookup,
            &mut key_lookup,
        )?;

        tenants_by_id.insert(tenant_id.to_string(), runtime);
    }

    let (known_header_key_names, known_query_key_names) = collect_known_key_names(&key_lookup);

    Ok(Arc::new(TenantStore::new(
        current.mode.clone(),
        current.config_dir.clone(),
        current.tenancy.clone(),
        current.global_latency.clone(),
        current.global_chaos.clone(),
        current.admin_auth_header.clone(),
        current.admin_auth_secret.clone(),
        current.tenant_header_name.clone(),
        default_tenant,
        tenants_by_id,
        header_lookup,
        key_lookup,
        known_header_key_names,
        known_query_key_names,
    )))
}

struct LookupState {
    header_lookup: HashMap<String, String>,
    key_lookup: HashMap<ResolvedRequestKey, String>,
    known_header_key_names: HashSet<String>,
    known_query_key_names: HashSet<String>,
}

fn build_named_tenant_runtimes(
    config: &AppConfig,
    tenancy: &TenancyConfig,
    named_tenants: &[TenantConfig],
    tenant_header: &str,
) -> Result<HashMap<String, Arc<TenantRuntime>>, Box<dyn Error>> {
    let mut tenants_by_id = HashMap::new();

    for tenant in named_tenants {
        let runtime = build_named_tenant_runtime(config, tenancy, tenant, tenant_header)?;
        tenants_by_id.insert(tenant.id.clone(), runtime);
    }

    Ok(tenants_by_id)
}

fn build_named_tenant_runtime(
    config: &AppConfig,
    tenancy: &TenancyConfig,
    tenant: &TenantConfig,
    tenant_header: &str,
) -> Result<Arc<TenantRuntime>, Box<dyn Error>> {
    let root_dir = tenancy.tenants_dir.join(&tenant.id);
    // This is logical isolation inside one shared process: each tenant gets its
    // own registry/templates/policy view, while the engine/server stay global.
    let registry = build_registry_from_layers(&[root_dir.as_path()])?;
    let (management_auth_header, management_auth_secret) =
        resolve_tenant_management_auth(Some(tenant))?;
    Ok(Arc::new(TenantRuntime {
        label: tenant.id.clone(),
        template_metadata: tenant.template_metadata(),
        registry,
        requires_key: tenant.requires_key(tenant_header),
        management_auth_header,
        management_auth_secret,
        latency: tenant.effective_latency(&config.latency),
        chaos: tenant.effective_chaos(&config.chaos),
    }))
}

fn build_lookup_state(
    tenancy: &TenancyConfig,
    named_tenants: &[TenantConfig],
    tenant_header: &str,
) -> Result<LookupState, Box<dyn Error>> {
    let mut header_lookup = HashMap::new();
    let mut key_lookup = HashMap::new();

    for tenant in named_tenants {
        register_header_lookups(tenant, tenancy, &mut header_lookup)?;
        register_key_lookups(tenant, tenant_header, &mut key_lookup)?;
    }

    let (known_header_key_names, known_query_key_names) = collect_known_key_names(&key_lookup);

    Ok(LookupState {
        header_lookup,
        key_lookup,
        known_header_key_names,
        known_query_key_names,
    })
}

fn validate_and_refresh_tenant_lookup_entries(
    tenant: &TenantConfig,
    tenant_id: &str,
    tenant_header: &str,
    header_lookup: &mut HashMap<String, String>,
    key_lookup: &mut HashMap<ResolvedRequestKey, String>,
) -> Result<(), Box<dyn Error>> {
    // Rebuild the target tenant's lookup entries before swapping so auth rotation is atomic.
    header_lookup.retain(|_, mapped_tenant_id| mapped_tenant_id != tenant_id);
    key_lookup.retain(|_, mapped_tenant_id| mapped_tenant_id != tenant_id);

    register_header_lookups_for_tenant_header(tenant, tenant_header, header_lookup)?;
    register_key_lookups(tenant, tenant_header, key_lookup)?;
    Ok(())
}

fn collect_known_key_names(
    key_lookup: &HashMap<ResolvedRequestKey, String>,
) -> (HashSet<String>, HashSet<String>) {
    let mut known_header_key_names = HashSet::new();
    let mut known_query_key_names = HashSet::new();

    for key in key_lookup.keys() {
        match key.source {
            TenantKeySource::Header => {
                known_header_key_names.insert(key.name.clone());
            }
            TenantKeySource::Query => {
                known_query_key_names.insert(key.name.clone());
            }
            _ => {}
        }
    }

    (known_header_key_names, known_query_key_names)
}

fn resolve_admin_auth(admin_auth: &AdminAuthConfig) -> Result<Option<String>, Box<dyn Error>> {
    Ok(admin_auth.resolved_value()?)
}

fn resolve_tenant_management_auth(
    tenant: Option<&TenantConfig>,
) -> Result<(String, Option<String>), Box<dyn Error>> {
    let Some(tenant) = tenant else {
        return Ok(("x-tenant-admin-key".to_string(), None));
    };

    Ok(tenant
        .resolved_management_auth()?
        .map(|management_auth| (management_auth.header, Some(management_auth.secret)))
        .unwrap_or_else(|| ("x-tenant-admin-key".to_string(), None)))
}

fn validate_management_auth_uniqueness_for_reload(
    current: &Arc<TenantStore>,
    reloaded_tenant_id: &str,
    reloaded_runtime: &Arc<TenantRuntime>,
) -> Result<(), Box<dyn Error>> {
    // Tenant reload stays local: compare the reloaded tenant-admin identity
    // against the current live store instead of re-reading unrelated tenants.
    // That keeps /tenant/reload narrow and avoids failing just because another
    // tenant currently has a broken secret source elsewhere on disk.
    let Some(reloaded_secret) = reloaded_runtime.management_auth_secret.as_deref() else {
        return Ok(());
    };

    if current.default_tenant.label != reloaded_tenant_id {
        validate_runtime_management_auth_identity(
            reloaded_tenant_id,
            reloaded_runtime.management_auth_header.as_str(),
            reloaded_secret,
            current.default_tenant.as_ref(),
        )?;
    }

    for existing_runtime in current.tenants_by_id.values() {
        if existing_runtime.label == reloaded_tenant_id {
            continue;
        }

        validate_runtime_management_auth_identity(
            reloaded_tenant_id,
            reloaded_runtime.management_auth_header.as_str(),
            reloaded_secret,
            existing_runtime.as_ref(),
        )?;
    }

    Ok(())
}

fn validate_runtime_management_auth_identity(
    reloaded_tenant_id: &str,
    reloaded_header: &str,
    reloaded_secret: &str,
    existing_runtime: &TenantRuntime,
) -> Result<(), Box<dyn Error>> {
    let Some(existing_secret) = existing_runtime.management_auth_secret.as_deref() else {
        return Ok(());
    };

    if existing_runtime.management_auth_header == reloaded_header
        && existing_secret == reloaded_secret
    {
        return Err(format!(
            "duplicate tenant management_auth identity between '{}' and '{}' on header '{}'; tenant-admin credentials must be unique per header+secret combination",
            existing_runtime.label, reloaded_tenant_id, reloaded_header
        )
        .into());
    }

    Ok(())
}

fn register_header_lookups(
    tenant: &TenantConfig,
    tenancy: &TenancyConfig,
    header_lookup: &mut HashMap<String, String>,
) -> Result<(), Box<dyn Error>> {
    register_header_lookups_for_tenant_header(
        tenant,
        &tenancy.normalized_tenant_header(),
        header_lookup,
    )
}

fn register_header_lookups_for_tenant_header(
    tenant: &TenantConfig,
    tenant_header: &str,
    header_lookup: &mut HashMap<String, String>,
) -> Result<(), Box<dyn Error>> {
    register_header_lookup_value(
        header_lookup,
        tenant_header.to_string(),
        tenant.id.trim().to_ascii_lowercase(),
        &tenant.id,
    )?;

    for value in tenant.explicit_header_values(tenant_header)? {
        register_header_lookup_value(
            header_lookup,
            tenant_header.to_string(),
            value.trim().to_ascii_lowercase(),
            &tenant.id,
        )?;
    }

    Ok(())
}

fn register_key_lookups(
    tenant: &TenantConfig,
    tenant_header: &str,
    key_lookup: &mut HashMap<ResolvedRequestKey, String>,
) -> Result<(), Box<dyn Error>> {
    for api_key in tenant.api_keys(tenant_header)? {
        register_key_lookup_value(
            key_lookup,
            ResolvedRequestKey {
                source: api_key.source,
                name: api_key.name,
                value: api_key.value,
            },
            &tenant.id,
        )?;
    }

    Ok(())
}

fn register_header_lookup_value(
    header_lookup: &mut HashMap<String, String>,
    header_name: String,
    lookup_value: String,
    tenant_id: &str,
) -> Result<(), Box<dyn Error>> {
    if let Some(existing_tenant_id) = header_lookup.get(&lookup_value) {
        if existing_tenant_id != tenant_id {
            return Err(format!(
                "ambiguous tenant key match between '{}' and '{}' on Header '{}={}'",
                existing_tenant_id, tenant_id, header_name, lookup_value
            )
            .into());
        }
    }

    header_lookup.insert(lookup_value, tenant_id.to_string());
    Ok(())
}

fn register_key_lookup_value(
    key_lookup: &mut HashMap<ResolvedRequestKey, String>,
    resolved_key: ResolvedRequestKey,
    tenant_id: &str,
) -> Result<(), Box<dyn Error>> {
    if let Some(existing_tenant_id) = key_lookup.get(&resolved_key) {
        if existing_tenant_id != tenant_id {
            return Err(format!(
                "ambiguous tenant key match between '{}' and '{}' on {:?} '{}'",
                existing_tenant_id,
                tenant_id,
                resolved_key.source,
                resolved_key.name,
            )
            .into());
        }
    }

    key_lookup.insert(resolved_key, tenant_id.to_string());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{EndpointConfig, LatencyConfig};
    use std::fs;
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

    fn write_tenant_metadata(base_dir: &std::path::Path, tenant_id: &str, body: &str) {
        let metadata_path = base_dir.join("tenants").join(tenant_id).join("tenant.toml");
        fs::create_dir_all(metadata_path.parent().unwrap()).unwrap();
        fs::write(metadata_path, body).unwrap();
    }

    fn write_provider(path: &std::path::Path, provider_name: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            path,
            format!(
                "name: \"{provider_name}\"\nmatcher: \"^/v1/chat/completions$\"\nresponse_body: '{{\"tenant\":\"{provider_name}\"}}'\npriority: 100\n"
            ),
        )
        .unwrap();
    }

    fn runtime_test_config(base_dir: &std::path::Path) -> AppConfig {
        AppConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            workers: 1,
            log_level: "debug".to_string(),
            config_dir: base_dir.join("config"),
            tenancy: TenancyConfig {
                mode: TenancyMode::Multi,
                tenants_dir: base_dir.join("tenants"),
                tenant_header: "x-tenant".to_string(),
                admin_auth: AdminAuthConfig::default(),
            },
            latency: LatencyConfig::default(),
            chaos: ChaosConfig::default(),
            endpoints: vec![EndpointConfig {
                path: "/v1/chat/completions".to_string(),
                format: "openai".to_string(),
                content_type: None,
            }],
            response_file: None,
            reload_args: None,
        }
    }

    fn tenant_provider_name(store: &TenantStore, tenant_id: &str) -> String {
        store.tenant_by_id(tenant_id)
            .unwrap()
            .registry
            .providers
            .first()
            .unwrap()
            .name
            .clone()
    }

    #[test]
    fn concurrent_tenant_reloads_preserve_each_successful_update() {
        let temp_base = unique_test_dir("concurrent_tenant_reloads_preserve_updates");
        write_tenant_metadata(&temp_base, "acme", "id = \"acme\"\n");
        write_tenant_metadata(&temp_base, "globex", "id = \"globex\"\n");
        write_provider(
            &temp_base.join("tenants/acme/providers/openai.yaml"),
            "acme-before",
        );
        write_provider(
            &temp_base.join("tenants/globex/providers/openai.yaml"),
            "globex-before",
        );

        let config = runtime_test_config(&temp_base);
        let handle = Arc::new(TenantStoreHandle::new(build_runtime_store(&config).unwrap()));

        write_provider(
            &temp_base.join("tenants/acme/providers/openai.yaml"),
            "acme-after",
        );
        write_provider(
            &temp_base.join("tenants/globex/providers/openai.yaml"),
            "globex-after",
        );

        let guard = handle.lock_reload_guard();

        let acme_handle = Arc::clone(&handle);
        let acme_thread = std::thread::spawn(move || acme_handle.reload_tenant("acme").unwrap());

        let globex_handle = Arc::clone(&handle);
        let globex_thread =
            std::thread::spawn(move || globex_handle.reload_tenant("globex").unwrap());

        drop(guard);

        acme_thread.join().unwrap();
        globex_thread.join().unwrap();

        let current = handle.current();
        assert_eq!(tenant_provider_name(&current, "acme"), "acme-after");
        assert_eq!(tenant_provider_name(&current, "globex"), "globex-after");

        fs::remove_dir_all(temp_base).unwrap();
    }

    #[test]
    fn concurrent_admin_and_tenant_reloads_preserve_each_successful_update() {
        let temp_base = unique_test_dir("concurrent_admin_and_tenant_reloads_preserve_updates");
        write_tenant_metadata(&temp_base, "acme", "id = \"acme\"\n");
        write_tenant_metadata(&temp_base, "globex", "id = \"globex\"\n");
        write_provider(
            &temp_base.join("tenants/acme/providers/openai.yaml"),
            "acme-before",
        );
        write_provider(
            &temp_base.join("tenants/globex/providers/openai.yaml"),
            "globex-before",
        );

        let mut config = runtime_test_config(&temp_base);
        config.tenancy.admin_auth = AdminAuthConfig {
            header: "x-admin-key".to_string(),
            value: "old-admin-secret".to_string(),
            value_file: None,
            value_env: None,
        };

        let handle = Arc::new(TenantStoreHandle::new(build_runtime_store(&config).unwrap()));

        write_provider(
            &temp_base.join("tenants/acme/providers/openai.yaml"),
            "acme-after",
        );

        let mut reloaded_config = config.clone();
        reloaded_config.tenancy.admin_auth.value = "new-admin-secret".to_string();

        let guard = handle.lock_reload_guard();

        let tenant_handle = Arc::clone(&handle);
        let tenant_thread =
            std::thread::spawn(move || tenant_handle.reload_tenant("acme").unwrap());

        let admin_handle = Arc::clone(&handle);
        let admin_thread =
            std::thread::spawn(move || admin_handle.reload_all(&reloaded_config).unwrap());

        drop(guard);

        tenant_thread.join().unwrap();
        admin_thread.join().unwrap();

        let current = handle.current();
        assert_eq!(tenant_provider_name(&current, "acme"), "acme-after");
        assert_eq!(current.admin_auth_secret.as_deref(), Some("new-admin-secret"));

        fs::remove_dir_all(temp_base).unwrap();
    }

    #[test]
    fn tenant_reload_errors_do_not_include_resolved_secret_values() {
        let temp_base = unique_test_dir("tenant_reload_redacts_secret_values");
        write_tenant_metadata(
            &temp_base,
            "acme",
            r#"
id = "acme"

[[keys]]
source = "header"
name = "x-api-key"
value = "secret-acme"
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
value = "secret-globex"
"#,
        );
        write_provider(
            &temp_base.join("tenants/acme/providers/openai.yaml"),
            "acme-before",
        );
        write_provider(
            &temp_base.join("tenants/globex/providers/openai.yaml"),
            "globex-before",
        );

        let config = runtime_test_config(&temp_base);
        let handle = TenantStoreHandle::new(build_runtime_store(&config).unwrap());

        write_tenant_metadata(
            &temp_base,
            "acme",
            r#"
id = "acme"

[[keys]]
source = "header"
name = "x-api-key"
value = "secret-globex"
"#,
        );

        let error = match handle.reload_tenant("acme") {
            Ok(_) => panic!("reload should fail when tenant keys collide"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("x-api-key"));
        assert!(!error.contains("secret-acme"));
        assert!(!error.contains("secret-globex"));

        fs::remove_dir_all(temp_base).unwrap();
    }
}
