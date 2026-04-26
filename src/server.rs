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
 * Core server logic. Sets up the Axum router, middleware (tracing, metrics),
 * and manages the server lifecycle.
 */

use std::sync::Arc;
use tokio::net::TcpListener;

use crate::config::AppConfig;
use crate::handlers::{
    admin_reload_handler, admin_tenant_handler, admin_tenants_handler, echo_handler, health_check,
    mock_handler, models_handler, status_handler, streaming_handler, tenant_handler,
    tenant_reload_handler, AppState,
};
use crate::tenancy::{TenantRequestMetrics, TenantStoreHandle};
// use crate::formats::load_response; // Removed

use axum::{
    extract::Request,
    middleware::{self, Next},
    response::IntoResponse,
    routing::{any, get, post},
    Extension, Router,
};
use metrics_exporter_prometheus::PrometheusHandle;
use tower_http::trace::TraceLayer;

pub async fn start_server(
    config: AppConfig,
    metrics_handle: PrometheusHandle,
    tenants: Arc<TenantStoreHandle>,
) -> Result<(), Box<dyn std::error::Error>> {
    let addr = format!("{}:{}", config.host, config.port);
    let port = config.port;

    // Bind the listener first to catch port-in-use errors early
    let listener = match TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("ERROR: Failed to bind to address {}: {}", addr, e);
            eprintln!(
                "       This usually means the port {} is already in use by another process.",
                port
            );
            eprintln!("       Try using a different port with --port <PORT>.");
            std::process::exit(1);
        }
    };

    // Useful when passing port 0 (which means "pick an available port").
    let local_addr = listener.local_addr().unwrap();

    // Update the config with the actual bound port so the /status endpoint reports it correctly
    let mut config = config;
    config.port = local_addr.port();

    let app = create_app(config, Some(metrics_handle), tenants).await;

    println!("🚀 VidaiMock is running at http://{}", local_addr);
    tracing::info!("Listening on {}", local_addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

pub async fn create_app(
    config: AppConfig,
    metrics_handle: Option<PrometheusHandle>,
    tenants: Arc<TenantStoreHandle>,
) -> Router {
    let state = Arc::new(AppState {
        config: Arc::new(config.clone()),
        tenants,
    });

    let mut app = Router::new()
        .route("/health", get(health_check))
        .route("/status", get(status_handler))
        .route("/admin/reload", post(admin_reload_handler))
        .route("/admin/tenants", get(admin_tenants_handler))
        .route("/admin/tenants/{id}", get(admin_tenant_handler))
        .route("/tenant", get(tenant_handler))
        .route("/tenant/reload", post(tenant_reload_handler));

    // Register endpoints
    let mut registered_paths = std::collections::HashSet::new();

    for endpoint in &config.endpoints {
        if endpoint.format == "echo" {
            app = app.route(&endpoint.path, any(echo_handler));
        } else if endpoint.path.contains("stream") {
            app = app.route(&endpoint.path, post(streaming_handler));
        } else {
            app = app.route(&endpoint.path, post(mock_handler));
        }
        registered_paths.insert(endpoint.path.clone());
    }

    // Default routes for common AI APIs (only if not already registered)
    macro_rules! register_default {
        ($path:expr, $method:ident, $handler:ident) => {
            if !registered_paths.contains($path) {
                app = app.route($path, $method($handler));
                registered_paths.insert($path.to_string());
            }
        };
    }

    register_default!("/v1/chat/completions", post, mock_handler);
    register_default!("/v1/chat/completions/stream", post, mock_handler);
    register_default!("/v1/models", get, models_handler);
    register_default!("/v1/embeddings", post, mock_handler);
    register_default!("/v1/images/generations", post, mock_handler);
    register_default!("/v1/responses", post, mock_handler);
    register_default!("/v1/moderations", post, mock_handler);

    // Error simulator
    register_default!("/error/{code}", post, mock_handler);

    register_default!("/v1/engines/{engine}/embeddings", post, mock_handler);
    register_default!("/v1beta/models/{model_action}", post, mock_handler);
    register_default!("/v1beta/models", get, models_handler);
    register_default!("/v1beta/openai/models", get, models_handler);
    register_default!("/v1beta/openai/embeddings", post, mock_handler);
    // Gemini AI Studio /v1/models paths - POST for generateContent
    register_default!("/v1/models/{model_action}", post, mock_handler);

    // Azure OpenAI paths
    register_default!(
        "/openai/deployments/{deployment}/chat/completions",
        post,
        mock_handler
    );
    register_default!(
        "/openai/deployments/{deployment}/embeddings",
        post,
        mock_handler
    );

    // Anthropic models
    if !registered_paths.contains("/v1/models/{model_action}") {
        app = app.route("/v1/models/{model_action}", get(models_handler));
    }
    register_default!("/v1/messages/stream", post, mock_handler);

    // Bedrock paths
    register_default!("/model/{model_id}/invoke", post, mock_handler);
    register_default!("/model/{model_id}/converse", post, mock_handler);
    register_default!(
        "/model/{model_id}/invoke-with-response-stream",
        post,
        mock_handler
    );
    register_default!("/model/{model_id}/converse-stream", post, mock_handler);

    // Vertex AI paths
    register_default!(
        "/v1/projects/{project}/locations/{location}/publishers/google/models/{model_action}",
        post,
        mock_handler
    );

    // Register metrics endpoint if handle is provided
    if let Some(handle) = metrics_handle {
        app = app.route("/metrics", get(move || std::future::ready(handle.render())));
    }

    app.fallback(post(mock_handler))
        .layer(Extension(state))
        .layer(TraceLayer::new_for_http())
        .layer(middleware::from_fn(track_metrics))
}

async fn track_metrics(req: Request, next: Next) -> impl IntoResponse {
    let start = std::time::Instant::now();
    let path = req.uri().path().to_owned();
    let method = req.method().clone();

    let response = next.run(req).await;

    // This measures handler time until the response object is returned.
    // For streaming responses it is closer to setup/TTFB than full body drain.
    let latency = start.elapsed().as_secs_f64();
    let status = response.status().as_u16().to_string();

    if let Some(metrics) = response.extensions().get::<TenantRequestMetrics>() {
        match metrics {
            TenantRequestMetrics::Accepted { tenant } => {
                metrics::counter!(
                    "http_requests_total",
                    "method" => method.to_string(),
                    "path" => path.clone(),
                    "status" => status,
                    "tenant" => tenant.clone()
                )
                .increment(1);
                metrics::histogram!(
                    "http_request_duration_seconds",
                    "method" => method.to_string(),
                    "path" => path,
                    "tenant" => tenant.clone()
                )
                .record(latency);
            }
            TenantRequestMetrics::Rejected { reason } => {
                metrics::counter!(
                    "tenant_request_rejections_total",
                    "method" => method.to_string(),
                    "path" => path.clone(),
                    "status" => status,
                    "reason" => (*reason).to_string()
                )
                .increment(1);
                metrics::histogram!(
                    "tenant_request_rejection_duration_seconds",
                    "method" => method.to_string(),
                    "path" => path,
                    "reason" => (*reason).to_string()
                )
                .record(latency);
            }
        }
    } else {
        metrics::counter!(
            "http_requests_total",
            "method" => method.to_string(),
            "path" => path.clone(),
            "status" => status
        )
        .increment(1);
        metrics::histogram!(
            "http_request_duration_seconds",
            "method" => method.to_string(),
            "path" => path
        )
        .record(latency);
    }

    response
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("Signal received, starting graceful shutdown...");
}

#[cfg(test)]
mod tests;
