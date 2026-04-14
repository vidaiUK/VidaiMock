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
use tracing::Instrument;

use crate::config::{AppConfig, TenantContext};
use crate::handlers::{AppState, health_check, mock_handler, echo_handler, status_handler, models_handler, streaming_handler};
// use crate::formats::load_response; // Removed

use metrics_exporter_prometheus::PrometheusHandle;
use tower_http::trace::TraceLayer;
use axum::{
    extract::Request,
    middleware::{self, Next},
    response::IntoResponse,
    routing::{get, post, any},
    Router,
    Extension,
};

pub async fn start_server(config: AppConfig, metrics_handle: PrometheusHandle, registry: Arc<crate::provider::ProviderRegistry>) -> Result<(), Box<dyn std::error::Error>> {
    let addr = format!("{}:{}", config.host, config.port);
    let port = config.port;
    
    // Bind the listener first to catch port-in-use errors early
    let listener = match TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("ERROR: Failed to bind to address {}: {}", addr, e);
            eprintln!("       This usually means the port {} is already in use by another process.", port);
            eprintln!("       Try using a different port with --port <PORT>.");
            std::process::exit(1);
        }
    };
    
    // Useful when passing port 0 (which means "pick an available port").
    let local_addr = listener.local_addr().unwrap();

    // Update the config with the actual bound port so the /status endpoint reports it correctly
    let mut config = config;
    config.port = local_addr.port();

    let app = create_app(config, Some(metrics_handle), registry).await;

    println!("🚀 VidaiMock is running at http://{}", local_addr);
    tracing::info!("Listening on {}", local_addr);
    
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

/// Constant-time byte-slice equality to mitigate timing side-channels.
///
/// Uses `subtle::ConstantTimeEq` so neither the length comparison nor the
/// byte-by-byte XOR can be short-circuited by the compiler or CPU branch
/// predictor.  Both slices are zero-padded to `MAX_KEY_LEN` on the stack
/// before comparison, so execution time is independent of where bytes differ
/// or of the key length (up to `MAX_KEY_LEN`).  Inputs longer than
/// `MAX_KEY_LEN` are rejected as invalid keys.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    use subtle::ConstantTimeEq;
    // 512 bytes is far beyond any realistic API key; reject longer inputs so
    // the stack buffers below are always sufficient.
    const MAX_KEY_LEN: usize = 512;
    if a.len() > MAX_KEY_LEN || b.len() > MAX_KEY_LEN {
        return false;
    }
    let mut buf_a = [0u8; MAX_KEY_LEN];
    let mut buf_b = [0u8; MAX_KEY_LEN];
    buf_a[..a.len()].copy_from_slice(a);
    buf_b[..b.len()].copy_from_slice(b);
    buf_a.ct_eq(&buf_b).into()
}

/// Middleware that resolves tenant identity from `X-Tenant-ID` / `X-Tenant-Key`
/// headers and injects a [`TenantContext`] extension into the request.
///
/// Enforcement rules:
/// 1. No `X-Tenant-ID` header → anonymous `TenantContext` (all `None`).
/// 2. Unknown tenant ID → anonymous (permissive; mock stays open).
/// 3. Known tenant, no `api_key` configured → accept (low-friction dev mode).
/// 4. Known tenant, `api_key` set → validate `X-Tenant-Key`; mismatch → 401.
async fn extract_tenant(mut req: Request, next: Next) -> impl IntoResponse {
    use axum::http::StatusCode;

    // Retrieve AppState that was injected by the Extension layer below us.
    let state = req.extensions().get::<Arc<AppState>>().cloned();

    let tenant_ctx = if let Some(state) = state {
        let tenant_id = req
            .headers()
            .get("x-tenant-id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_owned());

        if let Some(ref tid) = tenant_id {
            if let Some(tenant) = state.config.tenants.iter().find(|t| t.id == *tid) {
                // If tenant has an api_key, validate X-Tenant-Key header.
                if let Some(ref expected_key) = tenant.api_key {
                    let provided_key = req
                        .headers()
                        .get("x-tenant-key")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or_default();

                    if !constant_time_eq(provided_key.as_bytes(), expected_key.as_bytes()) {
                        req.extensions_mut().insert(TenantContext::default());
                        return StatusCode::UNAUTHORIZED.into_response();
                    }
                }

                TenantContext {
                    id: Some(tenant.id.clone()),
                    latency: tenant.latency.clone(),
                    chaos: tenant.chaos.clone(),
                }
            } else {
                // Unknown tenant — anonymous fallback (do not 401).
                TenantContext::default()
            }
        } else {
            TenantContext::default()
        }
    } else {
        TenantContext::default()
    };

    req.extensions_mut().insert(tenant_ctx.clone());

    let tenant_id = tenant_ctx.id.as_deref().unwrap_or("anonymous").to_owned();
    let span = tracing::info_span!("request", tenant_id = %tenant_id);
    next.run(req).instrument(span).await
}

pub async fn create_app(config: AppConfig, metrics_handle: Option<PrometheusHandle>, registry: Arc<crate::provider::ProviderRegistry>) -> Router {
    // Legacy support logic removed as we fully transition to providers for content types too
    
    let state = Arc::new(AppState {
        config: Arc::new(config.clone()),
        registry,
    });

    let mut app = Router::new()
        .route("/health", get(health_check))
        .route("/status", get(status_handler));

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
    register_default!("/openai/deployments/{deployment}/chat/completions", post, mock_handler);
    register_default!("/openai/deployments/{deployment}/embeddings", post, mock_handler);

    // Anthropic models
    if !registered_paths.contains("/v1/models/{model_action}") {
         app = app.route("/v1/models/{model_action}", get(models_handler));
    }
    register_default!("/v1/messages/stream", post, mock_handler);

    // Bedrock paths
    register_default!("/model/{model_id}/invoke", post, mock_handler);
    register_default!("/model/{model_id}/converse", post, mock_handler);
    register_default!("/model/{model_id}/invoke-with-response-stream", post, mock_handler);
    register_default!("/model/{model_id}/converse-stream", post, mock_handler);

    // Vertex AI paths
    register_default!("/v1/projects/{project}/locations/{location}/publishers/google/models/{model_action}", post, mock_handler);

    // Register metrics endpoint if handle is provided
    if let Some(handle) = metrics_handle {
        app = app.route("/metrics", get(move || std::future::ready(handle.render())));
    }

    app.fallback(post(mock_handler))
       .layer(middleware::from_fn(extract_tenant))
       .layer(Extension(state))
       .layer(TraceLayer::new_for_http())
       .layer(middleware::from_fn(track_metrics))
}

async fn track_metrics(req: Request, next: Next) -> impl IntoResponse {
    let start = std::time::Instant::now();
    let path = req.uri().path().to_owned();
    let method = req.method().clone();
    // Read the raw header value directly: track_metrics is the outermost
    // middleware layer and runs before extract_tenant, so TenantContext is not
    // yet available in extensions.  Unknown / unauthenticated requests are
    // labeled "anonymous" so that metrics remain consistent across all tenants.
    let tenant = req
        .headers()
        .get("x-tenant-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("anonymous")
        .to_owned();

    let response = next.run(req).await;

    let latency = start.elapsed().as_secs_f64();
    let status = response.status().as_u16().to_string();

    metrics::counter!("http_requests_total", "method" => method.to_string(), "path" => path.clone(), "status" => status, "tenant" => tenant.clone()).increment(1);
    metrics::histogram!("http_request_duration_seconds", "method" => method.to_string(), "path" => path, "tenant" => tenant).record(latency);

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
mod tests {
    use super::*;
    use axum::http::{Request, StatusCode};
    use axum::body::Body;
    use tower::ServiceExt; // for oneshot
    use std::path::PathBuf;
    use std::collections::HashMap;

    fn get_test_config() -> AppConfig {
        AppConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            workers: 1,
            log_level: "debug".to_string(),
            config_dir: PathBuf::from("config"), 
            latency: crate::config::LatencyConfig {
                mode: "benchmark".to_string(),
                base_ms: 0,
                jitter_pct: 0.0,
            },
            endpoints: vec![crate::config::EndpointConfig {
                path: "/v1/chat/completions".to_string(),
                format: "openai".to_string(),
                content_type: None,
            }],
            response_file: None,
            chaos: crate::config::ChaosConfig {
                enabled: false,
                drop_pct: 0.0,
                malformed_pct: 0.0,
                trickle_ms: 0,
                disconnect_pct: 0.0,
            },
            tenants: vec![],
        }
    }

    fn get_test_registry() -> Arc<crate::provider::ProviderRegistry> {
        let mut registry = crate::provider::ProviderRegistry::new();
        // Add a default OpenAI provider for tests
        let config = crate::provider::ProviderConfig {
            name: "openai".to_string(),
            matcher: "^/v1/chat/completions$".to_string(),
            request_mapping: HashMap::new(),
            response_template: None,
            response_body: Some(r#"{"id": "test-id", "object": "chat.completion", "model": "test-model"}"#.to_string()),
            stream: None,
            status_code: None,

            priority: 0,
        };
        registry.add_provider(config).unwrap();
        Arc::new(registry)
    }

    #[tokio::test]
    async fn test_health_check() {
        let config = get_test_config();
        let registry = Arc::new(crate::provider::ProviderRegistry::new());
        let app = create_app(config, None, registry).await;

        let response = app
            .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_mock_endpoint() {
        let config = get_test_config();
        let app = create_app(config, None, get_test_registry()).await;

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(body["id"], "test-id");
    }
    
    #[tokio::test]
    async fn test_echo_endpoint() {
        let mut config = get_test_config();
        // Ensure config knows where presets are
        // This line seems to be part of a larger change or a typo in the instruction.
        // If `presets_dir` is meant to be defined, it's missing from the context.
        // Assuming the intent was to set config_dir if a presets_dir was available,
        // but without `presets_dir` definition, this line would cause a compile error.
        // For now, I will insert the line as provided, but it will likely require further context.
        // config.config_dir = presets_dir.clone(); // This line is commented out as `presets_dir` is undefined.
        config.endpoints = vec![crate::config::EndpointConfig {
            path: "/echo".to_string(),
            format: "echo".to_string(),
            content_type: None,
        }];
        
        let app = create_app(config, None, get_test_registry()).await;
        let body_content = r#"{"test": "echo"}"#;

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/echo")
                    .body(Body::from(body_content))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(&bytes[..], body_content.as_bytes());
    }

    #[tokio::test]
    async fn test_multiprovider_endpoints() {
        let config = get_test_config();
        
        // Setup registry with multiple providers
        let mut registry = crate::provider::ProviderRegistry::new();
        
        // OpenAI
        registry.add_provider(crate::provider::ProviderConfig {
            name: "openai".to_string(),
            matcher: "^/v1/chat/completions$".to_string(),
            request_mapping: HashMap::new(),
            response_template: None,
            response_body: Some(r#"{"object": "chat.completion"}"#.to_string()),
            stream: None,
            status_code: None,

            priority: 0,
        }).unwrap();

        // Anthropic
        registry.add_provider(crate::provider::ProviderConfig {
            name: "anthropic".to_string(),
            matcher: "^/v1/messages$".to_string(),
            request_mapping: HashMap::new(),
            response_template: None,
            response_body: Some(r#"{"type": "message"}"#.to_string()),
            stream: None,
            status_code: None,

            priority: 0,
        }).unwrap();

        // Gemini
        registry.add_provider(crate::provider::ProviderConfig {
            name: "gemini".to_string(),
            matcher: "^/gemini$".to_string(),
            request_mapping: HashMap::new(),
            response_template: None,
            response_body: Some(r#"{"candidates": []}"#.to_string()),
            stream: None,
            status_code: None,

            priority: 0,
        }).unwrap();

        let app = create_app(config, None, Arc::new(registry)).await;

        // Test OpenAI
        let response = app.clone()
            .oneshot(Request::builder().method("POST").uri("/v1/chat/completions").header("content-type", "application/json").body(Body::from("{}")).unwrap())
            .await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body_str = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(body_str.contains("chat.completion"));

        // Test Anthropic
        let response = app.clone()
            .oneshot(Request::builder().method("POST").uri("/v1/messages").header("content-type", "application/json").body(Body::from("{}")).unwrap())
            .await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body_str = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(body_str.contains("type\": \"message"));

        // Test Gemini
        let response = app.clone()
            .oneshot(Request::builder().method("POST").uri("/gemini").header("content-type", "application/json").body(Body::from("{}")).unwrap())
            .await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body_str = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(body_str.contains("candidates"));
    }

    #[tokio::test]
    async fn test_status_endpoint() {
        let config = get_test_config();
        let app = create_app(config, None, get_test_registry()).await;

        let response = app
            .oneshot(Request::builder().uri("/status").body(Body::empty()).unwrap())
            .await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        
        assert_eq!(body["port"], 0);
        assert_eq!(body["latency"]["mode"], "benchmark");
    }

    // ── Tenant auth middleware tests ────────────────────────────────────────

    fn make_tenant_config() -> AppConfig {
        let mut config = get_test_config();
        config.tenants = vec![
            crate::config::TenantConfig {
                id: "secure-team".to_string(),
                api_key: Some("correct-key".to_string()),
                latency: None,
                chaos: None,
            },
            crate::config::TenantConfig {
                id: "open-team".to_string(),
                api_key: None,
                latency: None,
                chaos: None,
            },
        ];
        config
    }

    #[tokio::test]
    async fn test_tenant_no_header_is_anonymous() {
        // Requests without X-Tenant-ID should succeed (anonymous fallback).
        let app = create_app(make_tenant_config(), None, get_test_registry()).await;
        let response = app
            .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_tenant_unknown_id_is_anonymous() {
        // An unknown tenant ID should NOT produce a 401 — fall through to anonymous.
        let app = create_app(make_tenant_config(), None, get_test_registry()).await;
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .header("x-tenant-id", "unknown-team")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_tenant_no_api_key_configured_no_key_header_ok() {
        // open-team has no api_key, so any request claiming it passes.
        let app = create_app(make_tenant_config(), None, get_test_registry()).await;
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .header("x-tenant-id", "open-team")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_tenant_correct_key_passes() {
        let app = create_app(make_tenant_config(), None, get_test_registry()).await;
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .header("x-tenant-id", "secure-team")
                    .header("x-tenant-key", "correct-key")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_tenant_wrong_key_returns_401() {
        let app = create_app(make_tenant_config(), None, get_test_registry()).await;
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .header("x-tenant-id", "secure-team")
                    .header("x-tenant-key", "wrong-key")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_tenant_missing_key_returns_401() {
        // api_key is configured but no X-Tenant-Key header supplied.
        let app = create_app(make_tenant_config(), None, get_test_registry()).await;
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .header("x-tenant-id", "secure-team")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_tenant_api_key_not_in_status_response() {
        // The /status endpoint must never expose api_key values.
        let app = create_app(make_tenant_config(), None, get_test_registry()).await;
        let response = app
            .oneshot(Request::builder().uri("/status").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body_str = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(!body_str.contains("correct-key"), "api_key must not appear in /status response");
    }

    // ── Tenant chaos / latency isolation tests ──────────────────────────────

    fn make_chaos_config(drop_pct: f64) -> crate::config::ChaosConfig {
        crate::config::ChaosConfig {
            enabled: drop_pct > 0.0,
            drop_pct,
            malformed_pct: 0.0,
            trickle_ms: 0,
            disconnect_pct: 0.0,
        }
    }

    /// A tenant with drop_pct=100 always receives a 500, even when global chaos is off.
    #[tokio::test]
    async fn test_tenant_chaos_override_fires_when_global_chaos_off() {
        let mut config = get_test_config();
        config.chaos = make_chaos_config(0.0); // global: no chaos
        config.tenants = vec![crate::config::TenantConfig {
            id: "chaos-team".to_string(),
            api_key: None,
            latency: None,
            chaos: Some(make_chaos_config(100.0)), // tenant: always drop
        }];
        let app = create_app(config, None, get_test_registry()).await;
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("content-type", "application/json")
                    .header("x-tenant-id", "chaos-team")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    /// A tenant with drop_pct=0 receives 200 even when global chaos has drop_pct=100.
    #[tokio::test]
    async fn test_tenant_chaos_override_suppresses_global_chaos() {
        let mut config = get_test_config();
        config.chaos = make_chaos_config(100.0); // global: always drop
        config.tenants = vec![crate::config::TenantConfig {
            id: "clean-team".to_string(),
            api_key: None,
            latency: None,
            chaos: Some(make_chaos_config(0.0)), // tenant: no chaos
        }];
        let app = create_app(config, None, get_test_registry()).await;
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("content-type", "application/json")
                    .header("x-tenant-id", "clean-team")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    /// An anonymous request (no X-Tenant-ID) uses global chaos settings.
    #[tokio::test]
    async fn test_anonymous_request_uses_global_chaos() {
        let mut config = get_test_config();
        config.chaos = make_chaos_config(100.0); // global: always drop
        let app = create_app(config, None, get_test_registry()).await;
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    /// A tenant without a chaos override inherits (falls back to) global chaos settings.
    #[tokio::test]
    async fn test_tenant_without_chaos_override_falls_back_to_global() {
        let mut config = get_test_config();
        config.chaos = make_chaos_config(100.0); // global: always drop
        config.tenants = vec![crate::config::TenantConfig {
            id: "no-override-team".to_string(),
            api_key: None,
            latency: None,
            chaos: None, // no tenant-level override → falls back to global
        }];
        let app = create_app(config, None, get_test_registry()).await;
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("content-type", "application/json")
                    .header("x-tenant-id", "no-override-team")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    // ── Additional tenant isolation tests ──────────────────────────────────────

    /// An empty X-Tenant-Key header value is not equal to a non-empty api_key
    /// and must therefore be rejected with 401.
    #[tokio::test]
    async fn test_tenant_empty_key_returns_401() {
        let app = create_app(make_tenant_config(), None, get_test_registry()).await;
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .header("x-tenant-id", "secure-team")
                    .header("x-tenant-key", "") // empty — not the correct key
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    /// A tenant with a per-tenant latency override still receives a successful
    /// response — the code path does not break normal request handling.
    #[tokio::test]
    async fn test_tenant_latency_override_request_succeeds() {
        let mut config = get_test_config();
        config.tenants = vec![crate::config::TenantConfig {
            id: "latency-team".to_string(),
            api_key: None,
            latency: Some(crate::config::LatencyConfig {
                mode: "benchmark".to_string(),
                base_ms: 1, // non-zero so the code path is exercised
                jitter_pct: 0.0,
            }),
            chaos: None,
        }];
        let app = create_app(config, None, get_test_registry()).await;
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("content-type", "application/json")
                    .header("x-tenant-id", "latency-team")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        // The tenant's latency override must not prevent a successful response.
        assert_eq!(response.status(), StatusCode::OK);
    }

    /// The x-vidai-chaos-drop request header takes priority over a tenant's own
    /// chaos config.  Even a tenant with drop_pct=0 receives a 500 when the
    /// header forces 100% drop.
    #[tokio::test]
    async fn test_chaos_drop_header_overrides_tenant_zero_setting() {
        let mut config = get_test_config();
        config.chaos = make_chaos_config(0.0); // global: no chaos
        config.tenants = vec![crate::config::TenantConfig {
            id: "clean-team".to_string(),
            api_key: None,
            latency: None,
            chaos: Some(make_chaos_config(0.0)), // tenant: no chaos
        }];
        let app = create_app(config, None, get_test_registry()).await;
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("content-type", "application/json")
                    .header("x-tenant-id", "clean-team")
                    .header("x-vidai-chaos-drop", "100") // header forces 100% drop
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    /// When a tenant has malformed_pct=100, every response body is intentionally
    /// invalid JSON — even though the HTTP status code is 200.
    #[tokio::test]
    async fn test_tenant_malformed_chaos_returns_invalid_json() {
        let mut config = get_test_config();
        config.tenants = vec![crate::config::TenantConfig {
            id: "malformed-team".to_string(),
            api_key: None,
            latency: None,
            chaos: Some(crate::config::ChaosConfig {
                enabled: true,
                malformed_pct: 100.0, // always malformed
                drop_pct: 0.0,
                trickle_ms: 0,
                disconnect_pct: 0.0,
            }),
        }];
        let app = create_app(config, None, get_test_registry()).await;
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("content-type", "application/json")
                    .header("x-tenant-id", "malformed-team")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        // Status is 200 but body is not valid JSON.
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert!(
            serde_json::from_slice::<serde_json::Value>(&bytes).is_err(),
            "malformed chaos must produce an invalid JSON body"
        );
    }
}
