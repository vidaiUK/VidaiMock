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

use axum::http::HeaderMap;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::config::{TenancyMode, TenantKeySource};
use super::runtime::{TenantRuntime, TenantStore};

#[derive(Clone)]
pub struct TenantResolution {
    pub tenant: Arc<TenantRuntime>,
    pub metrics: TenantRequestMetrics,
}

#[derive(Debug, Clone)]
pub enum TenantRequestMetrics {
    Accepted { tenant: String },
    Rejected { reason: &'static str },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TenantResolutionRejection {
    UnknownTenant,
    UnknownKey,
    MissingKey,
    Conflict,
}

#[derive(Debug, Clone)]
pub struct TenantResolutionError {
    pub rejection: TenantResolutionRejection,
}

impl TenantResolutionError {
    pub fn metric_label(&self) -> &'static str {
        self.rejection.metric_label()
    }
}

impl TenantResolutionRejection {
    pub fn metric_label(&self) -> &'static str {
        match self {
            TenantResolutionRejection::UnknownTenant => "unknown_tenant",
            TenantResolutionRejection::UnknownKey => "unknown_key",
            TenantResolutionRejection::MissingKey => "missing_key",
            TenantResolutionRejection::Conflict => "header_key_conflict",
        }
    }
}

impl TenantStore {
    pub fn resolve_request(
        &self,
        headers: &HeaderMap,
        query_params: &HashMap<String, String>,
    ) -> Result<TenantResolution, TenantResolutionError> {
        if self.mode == TenancyMode::Single {
            return Ok(self.accept(self.default_tenant()));
        }

        let header_tenant_id = self.resolve_header_tenant(headers)?;
        let key_tenant_id = self.resolve_key_tenant(headers, query_params)?;

        match (header_tenant_id, key_tenant_id) {
            (Some(header_tenant_id), Some(key_tenant_id)) => {
                if header_tenant_id != key_tenant_id {
                    return Err(TenantResolutionError {
                        rejection: TenantResolutionRejection::Conflict,
                    });
                }

                Ok(self.accept(self.tenant_by_id(&header_tenant_id).unwrap()))
            }
            (Some(header_tenant_id), None) => {
                let tenant = self.tenant_by_id(&header_tenant_id).unwrap();
                if tenant.requires_key {
                    return Err(TenantResolutionError {
                        rejection: TenantResolutionRejection::MissingKey,
                    });
                }

                Ok(self.accept(tenant))
            }
            (None, Some(key_tenant_id)) => {
                let tenant = self.tenant_by_id(&key_tenant_id).unwrap();
                Ok(self.accept(tenant))
            }
            (None, None) => Ok(self.accept(self.default_tenant())),
        }
    }

    pub fn resolve_management_request(
        &self,
        headers: &HeaderMap,
    ) -> Result<TenantResolution, TenantManagementResolutionError> {
        if self.mode == TenancyMode::Single {
            return Ok(self.accept(self.default_tenant()));
        }

        let tenant = self.resolve_management_auth_tenant(headers)?;
        if let Some(signaled_tenant_id) = self.resolve_optional_management_tenant_signal(headers)? {
            if signaled_tenant_id != tenant.label {
                return Err(TenantManagementResolutionError::Conflict);
            }
        }

        Ok(self.accept(tenant))
    }

    fn accept(&self, tenant: Arc<TenantRuntime>) -> TenantResolution {
        TenantResolution {
            metrics: TenantRequestMetrics::Accepted {
                tenant: tenant.label.clone(),
            },
            tenant,
        }
    }

    fn resolve_header_tenant(
        &self,
        headers: &HeaderMap,
    ) -> Result<Option<String>, TenantResolutionError> {
        let Some(header_value) = headers.get(&self.tenant_header_name) else {
            return Ok(None);
        };

        let header_value = header_value
            .to_str()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        if header_value.is_empty() {
            return Err(TenantResolutionError {
                rejection: TenantResolutionRejection::UnknownTenant,
            });
        }

        self.header_lookup
            .get(&header_value)
            .cloned()
            .map(Some)
            .ok_or(TenantResolutionError {
                rejection: TenantResolutionRejection::UnknownTenant,
            })
    }

    fn resolve_management_auth_tenant(
        &self,
        headers: &HeaderMap,
    ) -> Result<Arc<TenantRuntime>, TenantManagementResolutionError> {
        let mut matched_tenant = None;

        // Startup/reload validation keeps tenant-admin identities unique, so
        // this remains a straightforward constant-time comparison pass instead
        // of a second trust boundary that has to resolve ambiguity on the fly.
        for tenant in self.management_auth_candidates() {
            let Some(expected_secret) = tenant.management_auth_secret.as_deref() else {
                continue;
            };

            let Some(provided) = headers
                .get(tenant.management_auth_header.as_str())
                .and_then(|value| value.to_str().ok())
            else {
                continue;
            };

            if management_secret_matches(
                tenant.management_auth_header.as_str(),
                provided.trim(),
                expected_secret,
            ) {
                if matched_tenant
                    .as_ref()
                    .is_some_and(|existing: &Arc<TenantRuntime>| existing.label != tenant.label)
                {
                    return Err(TenantManagementResolutionError::Conflict);
                }

                matched_tenant = Some(tenant);
            }
        }

        matched_tenant.ok_or(TenantManagementResolutionError::Unauthorized)
    }

    fn resolve_optional_management_tenant_signal(
        &self,
        headers: &HeaderMap,
    ) -> Result<Option<String>, TenantManagementResolutionError> {
        if headers.get(&self.tenant_header_name).is_none() {
            return Ok(None);
        }

        self.resolve_header_tenant(headers)
            .map_err(|_| TenantManagementResolutionError::Unauthorized)
    }

    fn management_auth_candidates(&self) -> Vec<Arc<TenantRuntime>> {
        let mut candidates = vec![self.default_tenant()];
        candidates.extend(self.tenants_by_id.values().cloned());
        candidates
    }

    fn resolve_key_tenant(
        &self,
        headers: &HeaderMap,
        query_params: &HashMap<String, String>,
    ) -> Result<Option<String>, TenantResolutionError> {
        let provided_keys = self.collect_provided_keys(headers, query_params);
        if provided_keys.is_empty() {
            return Ok(None);
        }

        let mut matched_tenants = HashSet::new();
        for provided_key in provided_keys {
            let Some(tenant_id) = self.key_lookup.get(&provided_key).cloned() else {
                return Err(TenantResolutionError {
                    rejection: TenantResolutionRejection::UnknownKey,
                });
            };
            matched_tenants.insert(tenant_id);
        }

        if matched_tenants.len() > 1 {
            return Err(TenantResolutionError {
                rejection: TenantResolutionRejection::Conflict,
            });
        }

        Ok(matched_tenants.into_iter().next())
    }

    fn collect_provided_keys(
        &self,
        headers: &HeaderMap,
        query_params: &HashMap<String, String>,
    ) -> Vec<ResolvedRequestKey> {
        let mut provided = Vec::new();

        for key_name in &self.known_header_key_names {
            if let Some(value) = headers.get(key_name).and_then(|value| value.to_str().ok()) {
                for candidate in request_key_variants(TenantKeySource::Header, key_name, value) {
                    provided.push(candidate);
                }
            }
        }

        for key_name in &self.known_query_key_names {
            if let Some(value) = query_params.get(key_name) {
                for candidate in request_key_variants(TenantKeySource::Query, key_name, value) {
                    provided.push(candidate);
                }
            }
        }

        provided
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TenantManagementResolutionError {
    Unauthorized,
    Conflict,
}

fn management_secret_matches(header_name: &str, provided: &str, expected_secret: &str) -> bool {
    constant_time_eq_str(provided, expected_secret)
        || (header_name.eq_ignore_ascii_case("authorization")
            && provided
                .strip_prefix("Bearer ")
                .or_else(|| provided.strip_prefix("bearer "))
                .is_some_and(|value| constant_time_eq_str(value.trim(), expected_secret)))
}

pub(crate) fn constant_time_eq_str(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    let max_len = left.len().max(right.len());
    let mut diff = left.len() ^ right.len();

    for index in 0..max_len {
        let left_byte = left.get(index).copied().unwrap_or_default();
        let right_byte = right.get(index).copied().unwrap_or_default();
        diff |= usize::from(left_byte ^ right_byte);
    }

    diff == 0
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResolvedRequestKey {
    pub source: TenantKeySource,
    pub name: String,
    pub value: String,
}

fn request_key_variants(
    source: TenantKeySource,
    name: &str,
    value: &str,
) -> Vec<ResolvedRequestKey> {
    let mut variants = vec![ResolvedRequestKey {
        source: source.clone(),
        name: name.to_ascii_lowercase(),
        value: value.trim().to_string(),
    }];

    if matches!(source, TenantKeySource::Header) && name.eq_ignore_ascii_case("authorization") {
        if let Some(stripped) = value
            .trim()
            .strip_prefix("Bearer ")
            .or_else(|| value.trim().strip_prefix("bearer "))
        {
            variants.push(ResolvedRequestKey {
                source,
                name: name.to_ascii_lowercase(),
                value: stripped.trim().to_string(),
            });
        }
    }

    variants
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ChaosConfig, LatencyConfig};
    use crate::provider::ProviderRegistry;
    use crate::tenancy::config::{
        AdminAuthConfig, TenancyConfig, TenantConfig, TenantKeyConfig, TenantTemplateMetadata,
    };
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn build_test_store(tenants: Vec<TenantConfig>) -> TenantStore {
        let default_runtime = Arc::new(TenantRuntime {
            label: "default".to_string(),
            template_metadata: TenantTemplateMetadata {
                id: "default".to_string(),
                ..TenantTemplateMetadata::default()
            },
            registry: Arc::new(ProviderRegistry::new()),
            requires_key: false,
            management_auth_header: "x-tenant-admin-key".to_string(),
            management_auth_secret: None,
            latency: LatencyConfig::default(),
            chaos: ChaosConfig::default(),
        });

        let tenancy = TenancyConfig {
            mode: TenancyMode::Multi,
            tenants_dir: PathBuf::from("tenants"),
            tenant_header: "x-tenant".to_string(),
            admin_auth: AdminAuthConfig::default(),
        };

        let mut tenants_by_id = HashMap::new();
        let mut header_lookup = HashMap::new();
        let mut key_lookup = HashMap::new();
        let mut known_header_key_names = HashSet::new();
        let mut known_query_key_names = HashSet::new();

        for tenant in &tenants {
            let requires_key = tenant.requires_key(&tenancy.normalized_tenant_header());
            let runtime = Arc::new(TenantRuntime {
                label: tenant.id.clone(),
                template_metadata: tenant.template_metadata(),
                registry: Arc::new(ProviderRegistry::new()),
                requires_key,
                management_auth_header: tenant
                    .management_auth
                    .as_ref()
                    .map(|config| config.header.to_ascii_lowercase())
                    .unwrap_or_else(|| "x-tenant-admin-key".to_string()),
                management_auth_secret: tenant.management_auth.as_ref().and_then(|config| {
                    (!config.value.trim().is_empty()).then(|| config.value.clone())
                }),
                latency: LatencyConfig::default(),
                chaos: ChaosConfig::default(),
            });

            tenants_by_id.insert(tenant.id.clone(), runtime);
            header_lookup.insert(tenant.id.clone(), tenant.id.clone());
            for value in tenant
                .explicit_header_values(&tenancy.normalized_tenant_header())
                .unwrap()
            {
                header_lookup.insert(value.to_ascii_lowercase(), tenant.id.clone());
            }

            for api_key in tenant
                .api_keys(&tenancy.normalized_tenant_header())
                .unwrap()
            {
                match api_key.source {
                    TenantKeySource::Header => {
                        known_header_key_names.insert(api_key.name.clone());
                    }
                    TenantKeySource::Query => {
                        known_query_key_names.insert(api_key.name.clone());
                    }
                    _ => {}
                }

                key_lookup.insert(
                    ResolvedRequestKey {
                        source: api_key.source,
                        name: api_key.name,
                        value: api_key.value,
                    },
                    tenant.id.clone(),
                );
            }
        }

        TenantStore::new(
            TenancyMode::Multi,
            PathBuf::from("config"),
            tenancy.clone(),
            LatencyConfig::default(),
            ChaosConfig::default(),
            tenancy.admin_auth.header.clone(),
            None,
            tenancy.normalized_tenant_header(),
            default_runtime,
            tenants_by_id,
            header_lookup,
            key_lookup,
            known_header_key_names,
            known_query_key_names,
        )
    }

    #[test]
    fn header_only_tenant_resolution() {
        let store = build_test_store(vec![TenantConfig {
            id: "acme".to_string(),
            keys: Vec::new(),
            ..Default::default()
        }]);

        let mut headers = HeaderMap::new();
        headers.insert("x-tenant", "acme".parse().unwrap());

        let resolved = store.resolve_request(&headers, &HashMap::new()).unwrap();
        assert_eq!(resolved.tenant.label, "acme");
    }

    #[test]
    fn key_only_tenant_resolution() {
        let store = build_test_store(vec![TenantConfig {
            id: "acme".to_string(),
            keys: vec![TenantKeyConfig {
                source: TenantKeySource::Header,
                name: "x-api-key".to_string(),
                value: "secret-acme".to_string(),
                value_file: None,
                value_env: None,
            }],
            ..Default::default()
        }]);

        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", "secret-acme".parse().unwrap());

        let resolved = store.resolve_request(&headers, &HashMap::new()).unwrap();
        assert_eq!(resolved.tenant.label, "acme");
    }

    #[test]
    fn header_and_key_agreement() {
        let store = build_test_store(vec![TenantConfig {
            id: "acme".to_string(),
            keys: vec![TenantKeyConfig {
                source: TenantKeySource::Header,
                name: "x-api-key".to_string(),
                value: "secret-acme".to_string(),
                value_file: None,
                value_env: None,
            }],
            ..Default::default()
        }]);

        let mut headers = HeaderMap::new();
        headers.insert("x-tenant", "acme".parse().unwrap());
        headers.insert("x-api-key", "secret-acme".parse().unwrap());

        let resolved = store.resolve_request(&headers, &HashMap::new()).unwrap();
        assert_eq!(resolved.tenant.label, "acme");
    }

    #[test]
    fn header_and_key_conflict_is_rejected() {
        let store = build_test_store(vec![
            TenantConfig {
                id: "acme".to_string(),
                keys: vec![TenantKeyConfig {
                    source: TenantKeySource::Header,
                    name: "x-api-key".to_string(),
                    value: "secret-acme".to_string(),
                    value_file: None,
                    value_env: None,
                }],
                ..Default::default()
            },
            TenantConfig {
                id: "globex".to_string(),
                keys: vec![TenantKeyConfig {
                    source: TenantKeySource::Header,
                    name: "x-api-key".to_string(),
                    value: "secret-globex".to_string(),
                    value_file: None,
                    value_env: None,
                }],
                ..Default::default()
            },
        ]);

        let mut headers = HeaderMap::new();
        headers.insert("x-tenant", "acme".parse().unwrap());
        headers.insert("x-api-key", "secret-globex".parse().unwrap());

        let error = store
            .resolve_request(&headers, &HashMap::new())
            .err()
            .unwrap();
        assert_eq!(error.rejection, TenantResolutionRejection::Conflict);
    }

    #[test]
    fn unknown_tenant_is_rejected() {
        let store = build_test_store(vec![TenantConfig {
            id: "acme".to_string(),
            keys: Vec::new(),
            ..Default::default()
        }]);

        let mut headers = HeaderMap::new();
        headers.insert("x-tenant", "missing".parse().unwrap());

        let error = store
            .resolve_request(&headers, &HashMap::new())
            .err()
            .unwrap();
        assert_eq!(error.rejection, TenantResolutionRejection::UnknownTenant);
    }

    #[test]
    fn unknown_key_is_rejected() {
        let store = build_test_store(vec![TenantConfig {
            id: "acme".to_string(),
            keys: vec![TenantKeyConfig {
                source: TenantKeySource::Header,
                name: "x-api-key".to_string(),
                value: "secret-acme".to_string(),
                value_file: None,
                value_env: None,
            }],
            ..Default::default()
        }]);

        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", "unknown".parse().unwrap());

        let error = store
            .resolve_request(&headers, &HashMap::new())
            .err()
            .unwrap();
        assert_eq!(error.rejection, TenantResolutionRejection::UnknownKey);
    }

    #[test]
    fn default_tenant_fallback() {
        let store = build_test_store(vec![TenantConfig {
            id: "acme".to_string(),
            keys: Vec::new(),
            ..Default::default()
        }]);

        let resolved = store
            .resolve_request(&HeaderMap::new(), &HashMap::new())
            .unwrap();
        assert_eq!(resolved.tenant.label, "default");
    }

    #[test]
    fn header_only_is_rejected_when_tenant_requires_key() {
        let store = build_test_store(vec![TenantConfig {
            id: "acme".to_string(),
            keys: vec![TenantKeyConfig {
                source: TenantKeySource::Header,
                name: "x-api-key".to_string(),
                value: "secret-acme".to_string(),
                value_file: None,
                value_env: None,
            }],
            ..Default::default()
        }]);

        let mut headers = HeaderMap::new();
        headers.insert("x-tenant", "acme".parse().unwrap());

        let error = store
            .resolve_request(&headers, &HashMap::new())
            .err()
            .unwrap();
        assert_eq!(error.rejection, TenantResolutionRejection::MissingKey);
    }
}
