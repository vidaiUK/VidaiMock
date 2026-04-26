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

mod config;
mod management;
mod resolution;
mod runtime;

pub use config::{
    AdminAuthConfig, TenancyConfig, TenancyMode, TenantSchema, TenantTemplateMetadata,
    DEFAULT_TENANT_ID,
};
pub use management::{list_tenants, tenant_view, ReloadView};
pub use resolution::{TenantRequestMetrics, TenantResolution, TenantResolutionError};
pub(crate) use resolution::constant_time_eq_str;
pub use runtime::{build_runtime_store, TenantRuntime, TenantStore, TenantStoreHandle};
