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

use clap::Parser;
use config::{Config, File};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::path::PathBuf;

use crate::tenancy::TenancyConfig;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AppConfig {
    #[serde(default = "default_host")]
    pub host: String,
    pub port: u16,
    pub workers: usize,
    pub log_level: String,
    pub config_dir: PathBuf,
    #[serde(default)]
    pub tenancy: TenancyConfig,
    #[serde(default)]
    pub latency: LatencyConfig,
    #[serde(default)]
    pub chaos: ChaosConfig,
    #[serde(default)]
    pub endpoints: Vec<EndpointConfig>,
    #[serde(skip)]
    pub response_file: Option<PathBuf>,
    #[serde(skip)]
    pub reload_args: Option<Cli>,
}

fn default_host() -> String {
    "0.0.0.0".to_string()
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub struct LatencyConfig {
    pub mode: String, // "benchmark", "realistic", "debug"
    pub base_ms: u64,
    pub jitter_pct: f64,
}

impl Default for LatencyConfig {
    fn default() -> Self {
        Self {
            mode: "benchmark".to_string(),
            base_ms: 0,
            jitter_pct: 0.0,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
pub struct EndpointConfig {
    pub path: String,
    pub format: String, // "openai", "anthropic", "gemini", "openrouter", "echo" or custom
    pub content_type: Option<String>,
}

#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    /// Host address to bind to (default: 0.0.0.0, use 127.0.0.1 for localhost only)
    #[arg(long)]
    pub host: Option<String>,

    /// Port to listen on
    #[arg(short, long)]
    pub port: Option<u16>,

    /// Number of worker threads (default: num cpus)
    #[arg(short, long)]
    pub workers: Option<usize>,

    /// Path to config file
    #[arg(long, default_value = "mock-server.toml")]
    pub config: PathBuf,

    /// Directory containing provider configurations
    #[arg(long)]
    pub config_dir: Option<PathBuf>,

    /// Latency in milliseconds
    #[arg(long)]
    pub latency: Option<u64>,

    /// Operation mode
    #[arg(long, value_parser = ["benchmark", "realistic", "debug"])]
    pub mode: Option<String>,

    /// Path to a custom response file (overrides format for default endpoints)
    #[arg(long)]
    pub response_file: Option<PathBuf>,

    /// Comma-separated list of endpoints to serve (overrides config)
    #[arg(long, value_delimiter = ',')]
    pub endpoints: Option<Vec<String>>,

    /// Response format to use (openai, anthropic, etc.)
    #[arg(long)]
    pub format: Option<String>,

    /// Override Content-Type header (e.g. application/xml)
    #[arg(long)]
    pub content_type: Option<String>,
}

impl AppConfig {
    pub fn load() -> Result<Self, config::ConfigError> {
        let args = Cli::parse();
        Self::build_config(args)
    }

    pub fn build_config(args: Cli) -> Result<Self, config::ConfigError> {
        let reload_args = args.clone();
        let config_base_dir = args
            .config
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let mut settings = Config::builder()
            // Start with defaults
            .set_default("port", 8100_i64)?
            .set_default("workers", num_cpus::get() as i64)?
            .set_default("log_level", "error")?
            .set_default("config_dir", "config")?;
        // Latency & Endpoints defaults handled by serde(default)

        // Load from file if exists
        if args.config.exists() {
            settings = settings.add_source(File::from(args.config.clone()));
        }

        // Environment variables (e.g., VIDAIMOCK_PORT, VIDAIMOCK_WORKERS)
        settings = settings.add_source(config::Environment::with_prefix("VIDAIMOCK"));

        // Build base config
        let mut config: AppConfig = settings.build()?.try_deserialize()?;

        // CLI Overrides
        if let Some(host) = args.host {
            config.host = host;
        }
        if let Some(port) = args.port {
            config.port = port;
        }
        if let Some(workers) = args.workers {
            config.workers = workers;
        }
        if let Some(latency) = args.latency {
            config.latency.base_ms = latency;
        }
        if let Some(mode) = args.mode {
            config.latency.mode = mode;
        }
        if let Some(dir) = args.config_dir {
            config.config_dir = dir;
        }

        config.response_file = args.response_file;

        // If endpoints provided via CLI, we construct a simple config where all endpoints use the specified (or default "openai") format
        if let Some(cli_endpoints) = args.endpoints {
            let default_format = args.format.unwrap_or_else(|| "openai".to_string());
            let content_type = args.content_type.clone();
            config.endpoints = cli_endpoints
                .into_iter()
                .map(|s| {
                    if s.contains(':') {
                        let parts: Vec<&str> = s.splitn(2, ':').collect();
                        EndpointConfig {
                            path: parts[0].to_string(),
                            format: parts[1].to_string(),
                            content_type: content_type.clone(),
                        }
                    } else {
                        EndpointConfig {
                            path: s,
                            format: default_format.clone(),
                            content_type: content_type.clone(),
                        }
                    }
                })
                .collect();
        } else if let Some(format) = args.format {
            // If only format is specified but no endpoints, we might want to override the format of existing default endpoints or add a default one.
            if config.endpoints.is_empty() {
                let path = match format.as_str() {
                    "anthropic" => "/v1/messages",
                    "gemini" => "/v1beta/models/gemini-pro:generateContent",
                    _ => "/v1/chat/completions",
                };
                config.endpoints.push(EndpointConfig {
                    path: path.to_string(),
                    format,
                    content_type: args.content_type,
                });
            }
        }

        config
            .tenancy
            .normalize_secret_paths(&config_base_dir);
        config.validate()?;
        config.reload_args = Some(reload_args);

        Ok(config)
    }

    pub fn reload_from_source(&self) -> Result<Self, config::ConfigError> {
        let Some(args) = self.reload_args.clone() else {
            return Err(config::ConfigError::Message(
                "config reload source is unavailable; restart required".to_string(),
            ));
        };

        Self::build_config(args)
    }

    pub fn validate(&self) -> Result<(), config::ConfigError> {
        if self.workers == 0 {
            return Err(config::ConfigError::Message(
                "workers must be at least 1".to_string(),
            ));
        }
        self.tenancy
            .validate()
            .map_err(config::ConfigError::Message)
    }

    pub fn reload_requires_restart(&self, next: &Self) -> Vec<&'static str> {
        let mut fields = Vec::new();

        if self.host != next.host {
            fields.push("host");
        }
        if self.port != next.port {
            fields.push("port");
        }
        if self.workers != next.workers {
            fields.push("workers");
        }
        if self.log_level != next.log_level {
            fields.push("log_level");
        }
        if self.latency != next.latency {
            fields.push("latency");
        }
        if self.chaos != next.chaos {
            fields.push("chaos");
        }
        if self.endpoints != next.endpoints {
            fields.push("endpoints");
        }
        if self.response_file != next.response_file {
            fields.push("response_file");
        }
        if self.tenancy != next.tenancy {
            fields.push("tenancy");
        }

        fields
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub struct ChaosConfig {
    /// Compatibility flag retained for config stability. Chaos behavior is
    /// driven by the percentages and delay fields below.
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub malformed_pct: f64,
    #[serde(default)]
    pub drop_pct: f64,
    #[serde(default)]
    pub trickle_ms: u64,
    #[serde(default)]
    pub disconnect_pct: f64,
}

impl Default for ChaosConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            malformed_pct: 0.0,
            drop_pct: 0.0,
            trickle_ms: 0,
            disconnect_pct: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
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

    fn write_tenant_metadata(base_dir: &PathBuf, tenant_dir: &str, body: &str) {
        let metadata_path = base_dir
            .join("tenants")
            .join(tenant_dir)
            .join("tenant.toml");
        fs::create_dir_all(metadata_path.parent().unwrap()).unwrap();
        fs::write(metadata_path, body).unwrap();
    }

    #[test]
    fn test_cli_port_override() {
        let args = Cli::parse_from(&["mock-server", "--port", "9999"]);
        let config = AppConfig::build_config(args).unwrap();
        assert_eq!(config.port, 9999);
    }

    #[test]
    fn test_cli_latency_mode_override() {
        let args = Cli::parse_from(&["mock-server", "--mode", "realistic", "--latency", "100"]);
        let config = AppConfig::build_config(args).unwrap();
        assert_eq!(config.latency.mode, "realistic");
        assert_eq!(config.latency.base_ms, 100);
    }

    #[test]
    fn test_cli_endpoints_override() {
        let args = Cli::parse_from(&[
            "mock-server",
            "--endpoints",
            "/custom1,/custom2",
            "--format",
            "echo",
        ]);
        let config = AppConfig::build_config(args).unwrap();
        assert_eq!(config.endpoints.len(), 2);
        assert_eq!(config.endpoints[0].path, "/custom1");
        assert_eq!(config.endpoints[0].format, "echo");
        assert_eq!(config.endpoints[1].path, "/custom2");
        assert_eq!(config.endpoints[1].format, "echo");
    }

    #[test]
    fn test_cli_endpoints_with_formats() {
        let args = Cli::parse_from(&[
            "mock-server",
            "--endpoints",
            "/v1/chat:openai,/v1/test:echo",
        ]);
        let config = AppConfig::build_config(args).unwrap();
        assert_eq!(config.endpoints.len(), 2);
        assert_eq!(config.endpoints[0].path, "/v1/chat");
        assert_eq!(config.endpoints[0].format, "openai");
        assert_eq!(config.endpoints[1].path, "/v1/test");
        assert_eq!(config.endpoints[1].format, "echo");
    }

    #[test]
    fn test_default_tenancy_mode_is_single() {
        let args = Cli::parse_from(&["mock-server"]);
        let config = AppConfig::build_config(args).unwrap();

        assert_eq!(config.tenancy.mode, crate::tenancy::TenancyMode::Single);
        assert_eq!(
            config.tenancy.schema_roots(&config.config_dir).unwrap(),
            vec![crate::tenancy::TenantSchema {
                tenant_id: None,
                root_dir: PathBuf::from("config"),
            }]
        );
    }

    #[test]
    fn test_multi_tenancy_mode_uses_tenants_schema() {
        let temp_base = unique_test_dir("test_multi_tenancy_mode");
        let temp_path = temp_base.join("mock-server.toml");
        let tenants_dir = temp_base.join("tenants");
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

        fs::create_dir_all(temp_path.parent().unwrap()).unwrap();
        fs::write(
            &temp_path,
            format!(
                r#"
port = 8100
workers = 4
log_level = "info"

[tenancy]
mode = "multi"
tenants_dir = "{}"
"#,
                tenants_dir.display()
            ),
        )
        .unwrap();

        let args = Cli::parse_from(&["mock-server", "--config", temp_path.to_str().unwrap()]);
        let config = AppConfig::build_config(args).unwrap();

        assert_eq!(config.tenancy.mode, crate::tenancy::TenancyMode::Multi);
        assert_eq!(
            config.tenancy.schema_roots(&config.config_dir).unwrap(),
            vec![
                crate::tenancy::TenantSchema {
                    tenant_id: Some(crate::tenancy::DEFAULT_TENANT_ID.to_string()),
                    root_dir: tenants_dir.join("default"),
                },
                crate::tenancy::TenantSchema {
                    tenant_id: Some("acme".to_string()),
                    root_dir: tenants_dir.join("acme"),
                },
                crate::tenancy::TenantSchema {
                    tenant_id: Some("globex".to_string()),
                    root_dir: tenants_dir.join("globex"),
                },
            ]
        );

        fs::remove_dir_all(temp_base).unwrap();
    }

    #[test]
    fn test_duplicate_tenant_ids_fail_validation() {
        let temp_base = unique_test_dir("test_duplicate_tenant_ids");
        let temp_path = temp_base.join("mock-server.toml");
        let tenants_dir = temp_base.join("tenants");
        write_tenant_metadata(
            &temp_base,
            "acme",
            r#"
id = "acme"
"#,
        );
        write_tenant_metadata(
            &temp_base,
            "globex-copy",
            r#"
id = "acme"
"#,
        );

        fs::create_dir_all(temp_path.parent().unwrap()).unwrap();
        fs::write(
            &temp_path,
            format!(
                r#"
port = 8100
workers = 4
log_level = "info"

[tenancy]
mode = "multi"
tenants_dir = "{}"
"#,
                tenants_dir.display()
            ),
        )
        .unwrap();

        let args = Cli::parse_from(&["mock-server", "--config", temp_path.to_str().unwrap()]);
        let error = AppConfig::build_config(args).unwrap_err();

        assert!(error.to_string().contains("duplicate tenant id"));

        fs::remove_dir_all(temp_base).unwrap();
    }

    #[test]
    fn test_admin_auth_value_file_is_resolved_relative_to_config_file() {
        let temp_base = unique_test_dir("test_admin_auth_value_file_relative_to_config");
        let temp_path = temp_base.join("mock-server.toml");
        fs::create_dir_all(temp_base.join("secrets")).unwrap();
        fs::write(temp_base.join("secrets/admin.key"), "admin-secret").unwrap();
        fs::write(
            &temp_path,
            r#"
port = 8100
workers = 1
log_level = "debug"
config_dir = "config"

[tenancy]
mode = "single"

[tenancy.admin_auth]
header = "x-admin-key"
value_file = "secrets/admin.key"
"#,
        )
        .unwrap();

        let args = Cli::parse_from(&["mock-server", "--config", temp_path.to_str().unwrap()]);
        let config = AppConfig::build_config(args).unwrap();

        assert_eq!(
            config.tenancy.admin_auth.resolved_value().unwrap(),
            Some("admin-secret".to_string())
        );
        assert_eq!(
            config.tenancy.admin_auth.value_file.as_ref().unwrap(),
            &temp_base.join("secrets/admin.key")
        );

        fs::remove_dir_all(temp_base).unwrap();
    }

    #[test]
    fn test_ambiguous_tenant_key_matches_fail_validation() {
        let temp_base = unique_test_dir("test_ambiguous_tenant_keys");
        let temp_path = temp_base.join("mock-server.toml");
        let tenants_dir = temp_base.join("tenants");
        write_tenant_metadata(
            &temp_base,
            "acme",
            r#"
id = "acme"

[[keys]]
source = "header"
name = "X-Tenant"
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
name = "x-tenant"
value = "shared"
"#,
        );

        fs::create_dir_all(temp_path.parent().unwrap()).unwrap();
        fs::write(
            &temp_path,
            format!(
                r#"
port = 8100
workers = 4
log_level = "info"

[tenancy]
mode = "multi"
tenants_dir = "{}"
"#,
                tenants_dir.display()
            ),
        )
        .unwrap();

        let args = Cli::parse_from(&["mock-server", "--config", temp_path.to_str().unwrap()]);
        let error = AppConfig::build_config(args).unwrap_err();

        assert!(error.to_string().contains("ambiguous tenant key match"));

        fs::remove_dir_all(temp_base).unwrap();
    }

    #[test]
    fn test_app_config_serialization_omits_tenancy_secret_fields() {
        let config: AppConfig = serde_json::from_value(json!({
            "host": "127.0.0.1",
            "port": 8100,
            "workers": 4,
            "log_level": "info",
            "config_dir": "config",
            "tenancy": {
                "mode": "multi",
                "tenants_dir": "tenants",
                "tenant_header": "x-tenant",
                "admin_auth": {
                    "header": "x-admin-key",
                    "value": "admin-secret",
                    "value_file": "/tmp/admin.key",
                    "value_env": "ADMIN_KEY"
                }
            },
            "latency": {
                "mode": "benchmark",
                "base_ms": 0,
                "jitter_pct": 0.0
            },
            "chaos": {
                "enabled": false,
                "malformed_pct": 0.0,
                "drop_pct": 0.0,
                "trickle_ms": 0,
                "disconnect_pct": 0.0
            },
            "endpoints": []
        }))
        .unwrap();

        assert_eq!(config.tenancy.admin_auth.header, "x-admin-key");

        let serialized = serde_json::to_value(&config).unwrap();
        let admin_auth = &serialized["tenancy"]["admin_auth"];

        assert_eq!(admin_auth["header"], "x-admin-key");
        assert!(admin_auth.get("value").is_none());
        assert!(admin_auth.get("value_file").is_none());
        assert!(admin_auth.get("value_env").is_none());
        assert!(serialized["tenancy"].get("tenants").is_none());
    }

    #[test]
    fn test_reload_requires_restart_detects_non_runtime_changes() {
        let mut current = AppConfig::build_config(Cli::parse_from(&["mock-server"])).unwrap();
        current.reload_args = None;

        let mut next = current.clone();
        next.latency.base_ms = 25;
        next.endpoints.push(EndpointConfig {
            path: "/v1/test".to_string(),
            format: "echo".to_string(),
            content_type: None,
        });

        let changed = current.reload_requires_restart(&next);
        assert_eq!(changed, vec!["latency", "endpoints"]);
    }

    /// S-2 regression: workers = 0 must fail config validation with a clear
    /// message instead of panicking inside tokio::runtime::Builder.
    #[test]
    fn test_workers_zero_fails_validation() {
        let mut config = AppConfig::build_config(Cli::parse_from(&["mock-server"])).unwrap();
        config.workers = 0;
        let err = config.validate().unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("workers must be at least 1"),
            "validation error should mention workers; got: {}",
            msg
        );
    }
}
