/*
 * Copyright (c) 2026 Vidai UK.
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
use std::path::PathBuf;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AppConfig {
    #[serde(default = "default_host")]
    pub host: String,
    pub port: u16,
    pub workers: usize,
    pub log_level: String,
    pub config_dir: PathBuf,
    #[serde(default)]
    pub latency: LatencyConfig,
    #[serde(default)]
    pub chaos: ChaosConfig,
    #[serde(default)]
    pub endpoints: Vec<EndpointConfig>,
    #[serde(skip)]
    pub response_file: Option<PathBuf>,
    /// When true, the binary's embedded provider configs and templates are
    /// ignored entirely; only `config_dir` is loaded. Lets users lock down the
    /// surface to exactly what they declare. Default false preserves the
    /// batteries-included experience.
    #[serde(default)]
    pub isolated: bool,
}

fn default_host() -> String {
    "0.0.0.0".to_string()
}

#[derive(Debug, Deserialize, Serialize, Clone)]
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

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct EndpointConfig {
    pub path: String,
    pub format: String, // "openai", "anthropic", "gemini", "openrouter", "echo" or custom
    pub content_type: Option<String>,
}

#[derive(Parser, Debug)]
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

    /// Ignore the binary's embedded provider configs and templates.
    /// Only `--config-dir` is loaded — lets you lock the surface down to
    /// exactly what you declare. Equivalent env var: `VIDAIMOCK_ISOLATED=true`.
    #[arg(long)]
    pub isolated: bool,
}

impl AppConfig {
    pub fn load() -> Result<Self, config::ConfigError> {
        let args = Cli::parse();
        Self::build_config(args)
    }

    /// Build a config for embedded (library) use.
    ///
    /// Unlike [`load`](Self::load) this never calls `Cli::parse()`, which
    /// would try to parse the *host* process's argv — in a test binary that
    /// means cargo's own arguments. Everything else (config file, environment
    /// variables, defaults) behaves exactly as it does for the CLI, so an
    /// embedded server honours the same `mock-server.toml` and `VIDAIMOCK_*`
    /// variables.
    pub fn for_embedded(config_dir: &std::path::Path, isolated: bool) -> Result<Self, config::ConfigError> {
        let args = Cli {
            host: None,
            port: None,
            workers: None,
            config: PathBuf::from("mock-server.toml"),
            config_dir: Some(config_dir.to_path_buf()),
            latency: None,
            mode: None,
            response_file: None,
            endpoints: None,
            format: None,
            content_type: None,
            isolated,
        };
        Self::build_config(args)
    }

    pub fn build_config(args: Cli) -> Result<Self, config::ConfigError> {
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
        // Bool flag: can only enable from CLI. To disable, omit the flag.
        // Env var (VIDAIMOCK_ISOLATED) and TOML (`isolated = true`) work via
        // serde defaults above.
        if args.isolated {
            config.isolated = true;
        }

        config.response_file = args.response_file;

        // If endpoints provided via CLI, we construct a simple config where all endpoints use the specified (or default "openai") format
        if let Some(cli_endpoints) = args.endpoints {
            let default_format = args.format.unwrap_or_else(|| "openai".to_string());
            let content_type = args.content_type.clone();
            config.endpoints = cli_endpoints.into_iter().map(|s| {
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
            }).collect();
        } else if let Some(format) = args.format {
             // If only format is specified but no endpoints, we might want to override the format of existing default endpoints or add a default one.
             if config.endpoints.is_empty() {
                 let path = match format.as_str() {
                     "anthropic" => "/v1/messages",
                     "gemini" => "/v1beta/models/gemini-pro:generateContent",
                     _ => "/v1/chat/completions",
                 };
                 config.endpoints.push(EndpointConfig { path: path.to_string(), format, content_type: args.content_type });
             }
        }
        
        Ok(config)
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ChaosConfig {
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
        let args = Cli::parse_from(&["mock-server", "--endpoints", "/custom1,/custom2", "--format", "echo"]);
        let config = AppConfig::build_config(args).unwrap();
        assert_eq!(config.endpoints.len(), 2);
        assert_eq!(config.endpoints[0].path, "/custom1");
        assert_eq!(config.endpoints[0].format, "echo");
        assert_eq!(config.endpoints[1].path, "/custom2");
        assert_eq!(config.endpoints[1].format, "echo");
    }

    #[test]
    fn test_cli_endpoints_with_formats() {
        let args = Cli::parse_from(&["mock-server", "--endpoints", "/v1/chat:openai,/v1/test:echo"]);
        let config = AppConfig::build_config(args).unwrap();
        assert_eq!(config.endpoints.len(), 2);
        assert_eq!(config.endpoints[0].path, "/v1/chat");
        assert_eq!(config.endpoints[0].format, "openai");
        assert_eq!(config.endpoints[1].path, "/v1/test");
        assert_eq!(config.endpoints[1].format, "echo");
    }

    /// --isolated absent → field defaults to false (no behaviour change for
    /// any existing user).
    #[test]
    fn test_isolated_defaults_to_false() {
        let args = Cli::parse_from(&["mock-server"]);
        let config = AppConfig::build_config(args).unwrap();
        assert!(!config.isolated, "isolated must default to false");
    }

    /// --isolated present → field becomes true (overrides any file/env value
    /// that was false; CLI flag wins).
    #[test]
    fn test_isolated_cli_flag_enables() {
        let args = Cli::parse_from(&["mock-server", "--isolated"]);
        let config = AppConfig::build_config(args).unwrap();
        assert!(config.isolated, "--isolated must enable the flag");
    }
}
