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

use crate::config::AppConfig;
use crate::server::start_server;
use crate::tenancy::{build_runtime_store, TenantStoreHandle};
use metrics_exporter_prometheus::PrometheusBuilder;
use mimalloc::MiMalloc;
use tracing::info;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::FmtSubscriber;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

mod config;
mod tenancy;
// mod formats; // Removed
mod aws_event_stream;
mod handlers;
mod provider;
mod replacer;
mod server; // Added for Bedrock streaming

/// Maps a `log_level` string to a `LevelFilter`.
///
/// `"off"` maps to `LevelFilter::OFF` to actually disable all tracing output.
/// Unknown values fall back to `LevelFilter::INFO`.
pub(crate) fn level_filter_from_str(log_level: &str) -> LevelFilter {
    match log_level.to_lowercase().as_str() {
        "off" => LevelFilter::OFF,
        "error" => LevelFilter::ERROR,
        "warn" => LevelFilter::WARN,
        "debug" => LevelFilter::DEBUG,
        _ => LevelFilter::INFO,
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = AppConfig::load()?;
    let workers = config.workers;

    // Initialize Prometheus Metrics
    let builder = PrometheusBuilder::new();
    let handle = match builder.install_recorder() {
        Ok(h) => h,
        Err(e) => {
            eprintln!("ERROR: Failed to initialize Prometheus metrics: {}", e);
            eprintln!("       This may happen if the metrics port is already in use.");
            eprintln!("       Try stopping other VidaiMock instances or check port conflicts.");
            std::process::exit(1);
        }
    };

    // Initialize Logging.
    // LevelFilter::OFF completely disables tracing so "off" behaves honestly.
    let log_filter = level_filter_from_str(&config.log_level);

    let subscriber = FmtSubscriber::builder().with_max_level(log_filter).finish();

    if let Err(e) = tracing::subscriber::set_global_default(subscriber) {
        eprintln!("ERROR: Failed to initialize logging: {}", e);
        std::process::exit(1);
    }

    if log_filter != LevelFilter::OFF {
        tracing::info!(
            "VidaiMock Initialization (Workers: {}, Latency: {}ms, Mode: {})",
            workers,
            config.latency.base_ms,
            config.latency.mode
        );

        // Diagnostic: List embedded assets
        for file in crate::provider::Asset::iter() {
            tracing::debug!("Embedded Asset: {}", file);
        }

        let endpoints: Vec<String> = config.endpoints.iter().map(|e| e.path.clone()).collect();
        info!(endpoints = ?endpoints, "Registered Endpoints");
    }

    let tenants = std::sync::Arc::new(TenantStoreHandle::new(build_runtime_store(&config)?));

    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers as usize)
        .enable_all()
        .build()?
        .block_on(start_server(config, handle, tenants))
}
