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
       .layer(Extension(state))
       .layer(TraceLayer::new_for_http())
       .layer(middleware::from_fn(track_metrics))
}

async fn track_metrics(req: Request, next: Next) -> impl IntoResponse {
    let start = std::time::Instant::now();
    let path = req.uri().path().to_owned();
    let method = req.method().clone();

    let response = next.run(req).await;

    let latency = start.elapsed().as_secs_f64();
    let status = response.status().as_u16().to_string();

    metrics::counter!("http_requests_total", "method" => method.to_string(), "path" => path.clone(), "status" => status).increment(1);
    metrics::histogram!("http_request_duration_seconds", "method" => method.to_string(), "path" => path).record(latency);

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
            isolated: false,
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
            error_template: None,
            priority: 0,
        };
        registry.add_provider(config).unwrap();
        Arc::new(registry)
    }

    /// Loads the bundled embedded providers (OpenAI, Anthropic, Gemini, etc.).
    /// Used for wire-shape regression tests that assert on real template output.
    fn get_embedded_registry() -> Arc<crate::provider::ProviderRegistry> {
        let mut registry = crate::provider::ProviderRegistry::new();
        // Pass a non-existent dir so only embedded providers load (filesystem
        // overlay empty). This mirrors the behaviour when a binary is run
        // without --config-dir on a machine without a config/ directory.
        registry.load_from_dir(&PathBuf::from("non_existent_dir_regression")).unwrap();
        Arc::new(registry)
    }

    /// Collects an Axum streaming response body into a single Vec<u8>.
    async fn drain_body(resp: axum::http::Response<Body>) -> Vec<u8> {
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        bytes.to_vec()
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
            error_template: None,
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
            error_template: None,
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
            error_template: None,
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

    // ─────────────────────────────────────────────────────────────────────────
    // SDK Compat regression suite (BUGS-VIDAIMOCK.md: VM-001 .. VM-008)
    // Each test locks a specific wire-shape contract so regressions that would
    // reintroduce an SDK-level parse failure fail in CI.
    // ─────────────────────────────────────────────────────────────────────────

    /// VM-001: OpenAI streaming must emit a terminal chunk carrying
    /// finish_reason = "stop" before the usage/[DONE] frames.
    /// Real openai-python treats that chunk as the stream's terminal signal.
    #[tokio::test]
    async fn test_vm001_openai_stream_emits_finish_reason() {
        let app = create_app(get_test_config(), None, get_embedded_registry()).await;
        let body = r#"{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hi"}],"max_tokens":5,"stream":true}"#;

        let resp = app.oneshot(
            Request::builder().method("POST").uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(body)).unwrap()
        ).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let bytes = drain_body(resp).await;
        let text = String::from_utf8(bytes).unwrap();

        assert!(text.contains(r#""finish_reason":"stop""#),
            "stream must include a chunk with finish_reason=stop before [DONE]; got:\n{}", text);
        assert!(text.ends_with("data: [DONE]\n\n"),
            "stream must end with 'data: [DONE]\\n\\n'; got tail:\n{}",
            &text[text.len().saturating_sub(100)..]);
    }

    /// VM-002: every SSE event must end with a blank line (\n\n), including
    /// the usage chunk that precedes [DONE] when stream_options.include_usage
    /// is set. openai-python's SSE parser fails with JSONDecodeError otherwise.
    #[tokio::test]
    async fn test_vm002_openai_stream_usage_chunk_has_blank_line_terminator() {
        let app = create_app(get_test_config(), None, get_embedded_registry()).await;
        let body = r#"{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hi"}],"max_tokens":5,"stream":true,"stream_options":{"include_usage":true}}"#;

        let resp = app.oneshot(
            Request::builder().method("POST").uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(body)).unwrap()
        ).await.unwrap();
        let text = String::from_utf8(drain_body(resp).await).unwrap();

        // The usage chunk is the one with "usage":{"prompt_tokens":... .
        // It must be followed by "\n\ndata: [DONE]" — not just "\ndata:".
        assert!(text.contains("\"usage\":{"),
            "stream must include a usage chunk when include_usage=true");
        let usage_idx = text.find("\"usage\":{").unwrap();
        let tail = &text[usage_idx..];
        // Find the end of the usage JSON object, then assert \n\n follows.
        let done_idx = tail.find("data: [DONE]")
            .expect("expected [DONE] frame after usage chunk");
        let between = &tail[..done_idx];
        assert!(between.ends_with("\n\n"),
            "usage chunk must be terminated with blank line before [DONE]; got bytes: {:?}",
            between.as_bytes().iter().rev().take(6).rev().collect::<Vec<_>>());
    }

    /// VM-005: ?chaos_status=500 query param produces an OpenAI-shaped JSON
    /// error envelope (provider error_template), not plain text or the
    /// success body. Composes with the URL so a routing layer can register
    /// one "broken" endpoint and another "healthy" endpoint against the same mock.
    #[tokio::test]
    async fn test_vm005_chaos_status_query_returns_provider_shape_error() {
        let app = create_app(get_test_config(), None, get_embedded_registry()).await;
        let body = r#"{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hi"}]}"#;

        let resp = app.oneshot(
            Request::builder().method("POST").uri("/v1/chat/completions?chaos_status=503")
                .header("content-type", "application/json")
                .body(Body::from(body)).unwrap()
        ).await.unwrap();

        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        let content_type = resp.headers().get("content-type").cloned();
        let text = String::from_utf8(drain_body(resp).await).unwrap();
        let json: serde_json::Value = serde_json::from_str(&text)
            .expect("chaos response must be valid JSON (not plain text)");

        assert_eq!(content_type.unwrap().to_str().unwrap(), "application/json");
        assert!(json.get("error").is_some(),
            "OpenAI error envelope must have top-level 'error' key; got: {}", text);
    }

    /// VM-005 (streaming): ?chaos_status=500 on a streaming request must
    /// return a non-streaming HTTP error (real providers don't send SSE
    /// errors). Assert status + JSON body.
    #[tokio::test]
    async fn test_vm005_chaos_status_on_streaming_returns_non_streaming_error() {
        let app = create_app(get_test_config(), None, get_embedded_registry()).await;
        let body = r#"{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hi"}],"stream":true}"#;

        let resp = app.oneshot(
            Request::builder().method("POST").uri("/v1/chat/completions?chaos_status=500")
                .header("content-type", "application/json")
                .body(Body::from(body)).unwrap()
        ).await.unwrap();

        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        // Body must be JSON, not SSE (no "data:" prefixes).
        let text = String::from_utf8(drain_body(resp).await).unwrap();
        assert!(!text.starts_with("data:"),
            "chaos on streaming must return non-streaming JSON body; got:\n{}", text);
        let _json: serde_json::Value = serde_json::from_str(&text)
            .expect("streaming chaos response body must be valid JSON");
    }

    /// VM-005: X-Vidai-Chaos-Drop=100 must return a JSON error envelope,
    /// not plain text. Previously returned "Simulated Internal Server Error"
    /// as text/plain which no SDK can parse.
    #[tokio::test]
    async fn test_vm005_chaos_drop_header_returns_json_error_envelope() {
        let app = create_app(get_test_config(), None, get_embedded_registry()).await;
        let body = r#"{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hi"}]}"#;

        let resp = app.oneshot(
            Request::builder().method("POST").uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .header("x-vidai-chaos-drop", "100")
                .body(Body::from(body)).unwrap()
        ).await.unwrap();

        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let content_type = resp.headers().get("content-type").cloned()
            .expect("missing content-type header");
        assert_eq!(content_type.to_str().unwrap(), "application/json",
            "chaos response must carry application/json content-type");

        let text = String::from_utf8(drain_body(resp).await).unwrap();
        let json: serde_json::Value = serde_json::from_str(&text)
            .expect("chaos response must be valid JSON");
        assert!(json.get("error").is_some(),
            "OpenAI error envelope must have top-level 'error' key");
    }

    /// VM-006: Anthropic streaming events must each be separated by a blank
    /// line (`\n\n`) so SDK SSE parsers treat them as distinct events.
    /// The symptom was "events out of order" but the root cause was frames
    /// merging because blank lines were stripped by the raw-frame renderer.
    #[tokio::test]
    async fn test_vm006_anthropic_stream_events_have_blank_line_separators() {
        let app = create_app(get_test_config(), None, get_embedded_registry()).await;
        let body = r#"{"model":"claude","max_tokens":30,"stream":true,"messages":[{"role":"user","content":"hi"}]}"#;

        let resp = app.oneshot(
            Request::builder().method("POST").uri("/v1/messages")
                .header("content-type", "application/json")
                .body(Body::from(body)).unwrap()
        ).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let text = String::from_utf8(drain_body(resp).await).unwrap();

        // First event must be message_start.
        let first_event_line = text.lines()
            .find(|l| l.starts_with("event: "))
            .expect("no event: lines found in Anthropic stream");
        assert_eq!(first_event_line, "event: message_start",
            "Anthropic streaming must start with message_start; got: {}", first_event_line);

        // Every inter-event transition must be separated by `\n\n`.
        // We check this by scanning for every occurrence of "\nevent:" and
        // asserting the byte before the leading \n is another \n (i.e. a
        // blank line precedes each event after the first).
        // Skip the very first `event:` at the start of the stream.
        let bytes = text.as_bytes();
        let mut pos = 0usize;
        let mut events_seen = 0usize;
        while let Some(idx) = text[pos..].find("\nevent: ") {
            let absolute = pos + idx;
            // absolute is the \n before "event:". For the 2nd event onward,
            // bytes[absolute - 1] must also be \n (i.e. a blank line).
            if events_seen >= 1 {
                assert!(absolute >= 1 && bytes[absolute - 1] == b'\n',
                    "event at byte {} not preceded by blank line; context: {:?}",
                    absolute,
                    &text[absolute.saturating_sub(20)..(absolute + 20).min(text.len())]);
            }
            events_seen += 1;
            pos = absolute + 1;
        }
        assert!(events_seen >= 7,
            "expected at least 7 Anthropic event types; found {}", events_seen);
    }

    /// VM-006 (order): even with framing fixed, the event order itself
    /// must be strict: message_start first, then content_block_start before
    /// any content_block_delta.
    #[tokio::test]
    async fn test_vm006_anthropic_stream_event_order() {
        let app = create_app(get_test_config(), None, get_embedded_registry()).await;
        let body = r#"{"model":"claude","max_tokens":30,"stream":true,"messages":[{"role":"user","content":"hi"}]}"#;

        let resp = app.oneshot(
            Request::builder().method("POST").uri("/v1/messages")
                .header("content-type", "application/json")
                .body(Body::from(body)).unwrap()
        ).await.unwrap();

        let text = String::from_utf8(drain_body(resp).await).unwrap();
        let events: Vec<&str> = text.lines()
            .filter(|l| l.starts_with("event: "))
            .collect();

        // Find indices of the three ordering-critical events.
        let pos_start = events.iter().position(|e| *e == "event: message_start")
            .expect("missing message_start");
        let pos_block_start = events.iter().position(|e| *e == "event: content_block_start")
            .expect("missing content_block_start");
        let pos_first_delta = events.iter().position(|e| *e == "event: content_block_delta")
            .expect("missing content_block_delta");
        let pos_stop = events.iter().position(|e| *e == "event: message_stop")
            .expect("missing message_stop");

        assert!(pos_start < pos_block_start,
            "message_start must precede content_block_start");
        assert!(pos_block_start < pos_first_delta,
            "content_block_start must precede content_block_delta");
        assert!(pos_first_delta < pos_stop,
            "content_block_delta must precede message_stop");
    }

    /// VM-007: Anthropic `/v1/messages` without `max_tokens` must return
    /// HTTP 400 with an Anthropic-shaped `{"type":"error","error":{...}}`
    /// envelope, matching real Anthropic validation.
    #[tokio::test]
    async fn test_vm007_anthropic_missing_max_tokens_returns_400() {
        let app = create_app(get_test_config(), None, get_embedded_registry()).await;
        let body = r#"{"model":"claude","messages":[{"role":"user","content":"hi"}]}"#;

        let resp = app.oneshot(
            Request::builder().method("POST").uri("/v1/messages")
                .header("content-type", "application/json")
                .body(Body::from(body)).unwrap()
        ).await.unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST,
            "missing max_tokens must return HTTP 400");

        let text = String::from_utf8(drain_body(resp).await).unwrap();
        let json: serde_json::Value = serde_json::from_str(&text)
            .expect("error body must be valid JSON");
        assert_eq!(json.get("type").and_then(|v| v.as_str()), Some("error"),
            "Anthropic error envelope must have type=error; got: {}", text);
        assert_eq!(
            json["error"]["type"].as_str(),
            Some("invalid_request_error"),
            "Anthropic validation errors must carry type=invalid_request_error"
        );
        // Message must name the missing field (real Anthropic: "max_tokens: field required").
        let message = json["error"]["message"].as_str().unwrap_or("");
        assert!(
            message.contains("max_tokens"),
            "error message must name the missing field; got: {}",
            message
        );
    }

    /// VM-007: Request WITH max_tokens continues to succeed (regression guard —
    /// the validation must be targeted, not blanket).
    #[tokio::test]
    async fn test_vm007_anthropic_with_max_tokens_returns_200() {
        let app = create_app(get_test_config(), None, get_embedded_registry()).await;
        let body = r#"{"model":"claude","max_tokens":30,"messages":[{"role":"user","content":"hi"}]}"#;

        let resp = app.oneshot(
            Request::builder().method("POST").uri("/v1/messages")
                .header("content-type", "application/json")
                .body(Body::from(body)).unwrap()
        ).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }

    /// VM-008: Gemini streamGenerateContent with alt=sse must NOT emit a
    /// `data: [DONE]` sentinel. Real Gemini terminates on connection close;
    /// a [DONE] frame causes google-genai SDK to raise UnknownApiResponseError.
    #[tokio::test]
    async fn test_vm008_gemini_stream_no_done_sentinel() {
        let app = create_app(get_test_config(), None, get_embedded_registry()).await;
        let body = r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}]}"#;

        let resp = app.oneshot(
            Request::builder().method("POST")
                .uri("/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse")
                .header("content-type", "application/json")
                .body(Body::from(body)).unwrap()
        ).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let text = String::from_utf8(drain_body(resp).await).unwrap();

        assert!(!text.contains("[DONE]"),
            "Gemini stream must not emit [DONE] sentinel; got:\n{}", text);
        // Must still contain at least one real data: frame.
        assert!(text.contains("data: {"),
            "Gemini stream should contain at least one JSON data frame");
    }

    /// VM-008 follow-up: Gemini streaming chunk shape contract.
    /// Real Gemini puts finishReason + usageMetadata ONLY on the final chunk;
    /// intermediate chunks have finishReason=null and no usageMetadata.
    /// Regression guard against over-emission (which would make google-genai
    /// SDK treat every chunk as a terminal signal and discard the rest).
    #[tokio::test]
    async fn test_vm008_gemini_stream_finishreason_only_on_final_chunk() {
        let app = create_app(get_test_config(), None, get_embedded_registry()).await;
        let body = r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}]}"#;

        let resp = app.oneshot(
            Request::builder().method("POST")
                .uri("/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse")
                .header("content-type", "application/json")
                .body(Body::from(body)).unwrap()
        ).await.unwrap();

        let text = String::from_utf8(drain_body(resp).await).unwrap();

        // Collect JSON payloads from every `data:` frame.
        let frames: Vec<serde_json::Value> = text.lines()
            .filter_map(|l| l.strip_prefix("data: "))
            .filter(|j| j.starts_with('{'))
            .filter_map(|j| serde_json::from_str(j).ok())
            .collect();

        assert!(frames.len() >= 2,
            "expected at least 2 frames (>=1 intermediate + 1 final); got {}",
            frames.len());

        // Final chunk must carry finishReason=STOP and usageMetadata.
        let final_frame = frames.last().unwrap();
        let final_finish = final_frame["candidates"][0]["finishReason"].as_str();
        assert_eq!(final_finish, Some("STOP"),
            "final chunk must carry finishReason=STOP; got frame: {}", final_frame);
        assert!(final_frame.get("usageMetadata").is_some(),
            "final chunk must carry usageMetadata; got: {}", final_frame);

        // Every intermediate chunk must carry finishReason=null and must NOT
        // carry usageMetadata. This is the specific shape google-genai SDK
        // requires to continue iterating through the stream.
        for (i, frame) in frames.iter().take(frames.len() - 1).enumerate() {
            let finish = &frame["candidates"][0]["finishReason"];
            assert!(
                finish.is_null(),
                "intermediate chunk {} must have finishReason=null; got {}; frame: {}",
                i, finish, frame
            );
            assert!(
                frame.get("usageMetadata").is_none(),
                "intermediate chunk {} must NOT carry usageMetadata; got frame: {}",
                i, frame
            );
        }
    }

    /// Regression: OpenAI non-streaming chat continues to return 200 with
    /// a valid chat.completion shape even after the error_template + chaos
    /// rewiring.
    #[tokio::test]
    async fn test_openai_non_streaming_still_works() {
        let app = create_app(get_test_config(), None, get_embedded_registry()).await;
        let body = r#"{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hi"}]}"#;
        let resp = app.oneshot(
            Request::builder().method("POST").uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(body)).unwrap()
        ).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let text = String::from_utf8(drain_body(resp).await).unwrap();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(json["object"], "chat.completion");
        assert!(json["choices"][0]["message"].is_object());
    }

    /// Regression: X-Mock-Status header retains its original behaviour —
    /// forces an HTTP status while keeping the success body shape (the
    /// contract documented for BFF-level passthrough tests).
    #[tokio::test]
    async fn test_xmockstatus_header_still_works() {
        let app = create_app(get_test_config(), None, get_embedded_registry()).await;
        let body = r#"{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hi"}]}"#;
        let resp = app.oneshot(
            Request::builder().method("POST").uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .header("x-mock-status", "429")
                .body(Body::from(body)).unwrap()
        ).await.unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
        // Body is rendered from error_template (since status is an error).
        let text = String::from_utf8(drain_body(resp).await).unwrap();
        let _: serde_json::Value = serde_json::from_str(&text)
            .expect("body must be valid JSON");
    }

    // ─── VM-009: Agentic tool-loop termination (end-to-end) ──────────────
    //
    // These integration tests complement the unit tests in provider.rs by
    // exercising the full HTTP->template->response path. They guarantee that
    // the heuristic is wired into each provider's bundled chat template and
    // that the emitted response actually changes shape when a tool result
    // sits in the history.

    /// VM-009 / OpenAI: with `tools` but a preceding `role:tool` message, the
    /// response must be plain text with finish_reason=stop — NOT tool_calls.
    #[tokio::test]
    async fn test_vm009_openai_tool_loop_terminates_on_tool_result() {
        let app = create_app(get_test_config(), None, get_embedded_registry()).await;
        let body = r#"{
            "model": "gpt-4o-mini",
            "tools": [{"type": "function", "function": {"name": "get_weather", "parameters": {}}}],
            "messages": [
                {"role": "user", "content": "What is the weather in London?"},
                {"role": "assistant", "tool_calls": [
                    {"id": "call_1", "type": "function",
                     "function": {"name": "get_weather", "arguments": "{\"city\":\"London\"}"}}
                ]},
                {"role": "tool", "tool_call_id": "call_1", "content": "15°C cloudy"}
            ]
        }"#;
        let resp = app.oneshot(
            Request::builder().method("POST").uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(body)).unwrap()
        ).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let text = String::from_utf8(drain_body(resp).await).unwrap();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        let choice = &json["choices"][0];
        let msg = &choice["message"];

        assert_eq!(choice["finish_reason"], "stop",
            "finish_reason must be 'stop' after tool result, not 'tool_calls'; got:\n{}", text);
        assert!(msg.get("tool_calls").is_none() || msg["tool_calls"].is_null(),
            "response must NOT include tool_calls after a tool result; got:\n{}", text);
        assert!(msg["content"].as_str().is_some(),
            "response must include text content after a tool result; got:\n{}", text);
    }

    /// VM-009 / OpenAI (regression guard): with `tools` and NO tool result in
    /// the history, the response must still be tool_calls. We must only
    /// terminate when appropriate.
    #[tokio::test]
    async fn test_vm009_openai_tools_without_result_still_returns_tool_calls() {
        let app = create_app(get_test_config(), None, get_embedded_registry()).await;
        let body = r#"{
            "model": "gpt-4o-mini",
            "tools": [{"type": "function", "function": {"name": "get_weather", "parameters": {}}}],
            "messages": [{"role": "user", "content": "Weather?"}]
        }"#;
        let resp = app.oneshot(
            Request::builder().method("POST").uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(body)).unwrap()
        ).await.unwrap();
        let text = String::from_utf8(drain_body(resp).await).unwrap();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(json["choices"][0]["finish_reason"], "tool_calls");
        assert!(json["choices"][0]["message"]["tool_calls"].is_array());
    }

    /// VM-009 / Anthropic: tool_result content block in user message
    /// terminates the loop — stop_reason must be end_turn and content
    /// must be text, not tool_use.
    #[tokio::test]
    async fn test_vm009_anthropic_tool_loop_terminates_on_tool_result() {
        let app = create_app(get_test_config(), None, get_embedded_registry()).await;
        let body = r#"{
            "model": "claude",
            "max_tokens": 200,
            "tools": [{"name": "get_weather", "description": "x", "input_schema": {"type":"object"}}],
            "messages": [
                {"role": "user", "content": "Weather in London?"},
                {"role": "assistant", "content": [
                    {"type": "tool_use", "id": "t1", "name": "get_weather", "input": {"city": "London"}}
                ]},
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "t1", "content": "15°C cloudy"}
                ]}
            ]
        }"#;
        let resp = app.oneshot(
            Request::builder().method("POST").uri("/v1/messages")
                .header("content-type", "application/json")
                .body(Body::from(body)).unwrap()
        ).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let text = String::from_utf8(drain_body(resp).await).unwrap();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();

        assert_eq!(json["stop_reason"], "end_turn",
            "stop_reason must be 'end_turn' after tool_result, not 'tool_use'; got:\n{}", text);
        let first_block_type = json["content"][0]["type"].as_str();
        assert_eq!(first_block_type, Some("text"),
            "first content block must be 'text' after tool_result, not 'tool_use'; got:\n{}", text);
    }

    /// VM-009 / Anthropic (regression guard): with `tools` but only the
    /// initial user message, the response must still be a tool_use block.
    #[tokio::test]
    async fn test_vm009_anthropic_tools_without_result_still_returns_tool_use() {
        let app = create_app(get_test_config(), None, get_embedded_registry()).await;
        let body = r#"{
            "model": "claude",
            "max_tokens": 200,
            "tools": [{"name": "get_weather", "description": "x", "input_schema": {"type":"object"}}],
            "messages": [{"role": "user", "content": "Weather?"}]
        }"#;
        let resp = app.oneshot(
            Request::builder().method("POST").uri("/v1/messages")
                .header("content-type", "application/json")
                .body(Body::from(body)).unwrap()
        ).await.unwrap();
        let text = String::from_utf8(drain_body(resp).await).unwrap();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(json["stop_reason"], "tool_use");
        assert_eq!(json["content"][0]["type"], "tool_use");
    }

    /// VM-009 / Gemini: functionResponse part in user contents terminates
    /// the loop — first part must be text, not functionCall. finishMessage
    /// must NOT say "Model generated function call(s)."
    #[tokio::test]
    async fn test_vm009_gemini_tool_loop_terminates_on_function_response() {
        let app = create_app(get_test_config(), None, get_embedded_registry()).await;
        let body = r#"{
            "contents": [
                {"role": "user", "parts": [{"text": "Weather in London?"}]},
                {"role": "model", "parts": [{"functionCall": {"name": "get_weather", "args": {"city":"London"}}}]},
                {"role": "user", "parts": [{"functionResponse": {"name": "get_weather", "response": {"temp": 15}}}]}
            ],
            "tools": [{"functionDeclarations": [{"name": "get_weather", "parameters": {}}]}]
        }"#;
        let resp = app.oneshot(
            Request::builder().method("POST")
                .uri("/v1beta/models/gemini-2.5-flash:generateContent")
                .header("content-type", "application/json")
                .body(Body::from(body)).unwrap()
        ).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let text = String::from_utf8(drain_body(resp).await).unwrap();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        let first_part = &json["candidates"][0]["content"]["parts"][0];

        assert!(first_part.get("text").is_some(),
            "first part must carry text after functionResponse; got:\n{}", text);
        assert!(first_part.get("functionCall").is_none(),
            "first part must NOT be functionCall after functionResponse; got:\n{}", text);
        // finishMessage is only emitted on the function-call branch; absence here
        // confirms the loop-terminating branch rendered.
        assert!(json["candidates"][0].get("finishMessage").is_none(),
            "finishMessage must be absent on the text-response branch; got:\n{}", text);
    }

    /// VM-009 / Gemini (regression guard): with tools and no functionResponse,
    /// still returns functionCall.
    #[tokio::test]
    async fn test_vm009_gemini_tools_without_response_still_returns_function_call() {
        let app = create_app(get_test_config(), None, get_embedded_registry()).await;
        let body = r#"{
            "contents": [{"role": "user", "parts": [{"text": "Weather?"}]}],
            "tools": [{"functionDeclarations": [{"name": "get_weather", "parameters": {}}]}]
        }"#;
        let resp = app.oneshot(
            Request::builder().method("POST")
                .uri("/v1beta/models/gemini-2.5-flash:generateContent")
                .header("content-type", "application/json")
                .body(Body::from(body)).unwrap()
        ).await.unwrap();
        let text = String::from_utf8(drain_body(resp).await).unwrap();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(json["candidates"][0]["content"]["parts"][0].get("functionCall").is_some());
    }

    // ─── VM-010: Tool-call responses must echo the caller's tool name ──────
    //
    // Previously, the OpenAI chat template hardcoded "get_weather"/"Paris"
    // regardless of what tool the caller declared. Anthropic and Gemini
    // already echoed correctly; we lock that in as regression guards too.

    /// VM-010 / OpenAI non-streaming: declared tool name must appear in response.
    #[tokio::test]
    async fn test_vm010_openai_echoes_caller_tool_name() {
        let app = create_app(get_test_config(), None, get_embedded_registry()).await;
        let body = r#"{
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [{"type": "function", "function": {"name": "canary_tool_a", "parameters": {}}}]
        }"#;
        let resp = app.oneshot(
            Request::builder().method("POST").uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(body)).unwrap()
        ).await.unwrap();
        let json: serde_json::Value = serde_json::from_str(
            &String::from_utf8(drain_body(resp).await).unwrap()
        ).unwrap();
        let tc = &json["choices"][0]["message"]["tool_calls"][0];
        assert_eq!(tc["function"]["name"], "canary_tool_a",
            "OpenAI response must echo caller's tool name; got: {}", tc);
        assert_eq!(tc["function"]["arguments"], "{}",
            "OpenAI args should default to empty object; got: {}", tc);
    }

    /// VM-010 / OpenAI streaming: same — the streaming tool_calls frame
    /// must echo the caller's name, not hardcoded.
    #[tokio::test]
    async fn test_vm010_openai_streaming_echoes_caller_tool_name() {
        let app = create_app(get_test_config(), None, get_embedded_registry()).await;
        let body = r#"{
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hi"}],
            "stream": true,
            "tools": [{"type": "function", "function": {"name": "canary_tool_s", "parameters": {}}}]
        }"#;
        let resp = app.oneshot(
            Request::builder().method("POST").uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(body)).unwrap()
        ).await.unwrap();
        let text = String::from_utf8(drain_body(resp).await).unwrap();
        // Find the tool_calls frame and assert it carries the caller's name.
        assert!(text.contains("\"name\":\"canary_tool_s\""),
            "OpenAI streaming tool_calls must echo caller's name; got:\n{}", text);
        assert!(!text.contains("\"name\":\"get_weather\""),
            "OpenAI streaming must not emit the hardcoded demo name; got:\n{}", text);
    }

    /// VM-010 / Anthropic non-streaming (regression guard).
    #[tokio::test]
    async fn test_vm010_anthropic_echoes_caller_tool_name() {
        let app = create_app(get_test_config(), None, get_embedded_registry()).await;
        let body = r#"{
            "model": "claude", "max_tokens": 200,
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [{"name": "canary_tool_b", "description": "x", "input_schema": {"type": "object"}}]
        }"#;
        let resp = app.oneshot(
            Request::builder().method("POST").uri("/v1/messages")
                .header("content-type", "application/json")
                .body(Body::from(body)).unwrap()
        ).await.unwrap();
        let json: serde_json::Value = serde_json::from_str(
            &String::from_utf8(drain_body(resp).await).unwrap()
        ).unwrap();
        assert_eq!(json["content"][0]["type"], "tool_use");
        assert_eq!(json["content"][0]["name"], "canary_tool_b");
    }

    /// VM-010 / Gemini non-streaming (regression guard).
    #[tokio::test]
    async fn test_vm010_gemini_echoes_caller_tool_name() {
        let app = create_app(get_test_config(), None, get_embedded_registry()).await;
        let body = r#"{
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}],
            "tools": [{"functionDeclarations": [{"name": "canary_tool_c", "parameters": {"type":"OBJECT"}}]}]
        }"#;
        let resp = app.oneshot(
            Request::builder().method("POST")
                .uri("/v1beta/models/gemini-2.5-flash:generateContent")
                .header("content-type", "application/json")
                .body(Body::from(body)).unwrap()
        ).await.unwrap();
        let json: serde_json::Value = serde_json::from_str(
            &String::from_utf8(drain_body(resp).await).unwrap()
        ).unwrap();
        let fc = &json["candidates"][0]["content"]["parts"][0]["functionCall"];
        assert_eq!(fc["name"], "canary_tool_c");
    }

    // ─── VM-011: Streaming-with-tools wire-format linter ───────────────────
    //
    // Every `data:` frame in a streaming SSE response must be a single line
    // of valid JSON. Before the VM-011 fix, Gemini + Anthropic emitted
    // pretty-printed multi-line JSON that bled outside the SSE framing
    // (Gemini's SDK errored; Anthropic's silently mis-parsed). Locks in the
    // single-line contract across all three providers.
    //
    // Reusable: easy to extend when new providers add streaming-with-tools.

    /// Assert every `data: {...}` line in an SSE body is a single-line
    /// JSON object that parses cleanly and contains no embedded newlines.
    fn assert_data_frames_are_single_line_json(body: &str, label: &str) {
        let mut frame_count = 0;
        for line in body.lines() {
            let Some(payload) = line.strip_prefix("data: ") else { continue };
            if payload.is_empty() || payload == "[DONE]" {
                // Empty data: is a malformed frame (the original Gemini bug).
                panic!("{label}: empty 'data:' line found — indicates broken multi-line frame");
            }
            // Must parse as JSON.
            serde_json::from_str::<serde_json::Value>(payload)
                .unwrap_or_else(|e| panic!(
                    "{label}: data frame must be valid JSON; parse error: {e}\nframe: {payload}\nfull body:\n{body}"
                ));
            // Must contain no embedded newlines (single-line per SSE spec).
            assert!(!payload.contains('\n'),
                "{label}: data frame must be single-line; got embedded \\n in:\n{payload}");
            frame_count += 1;
        }
        assert!(frame_count >= 1,
            "{label}: expected at least one data: frame; got 0. body:\n{body}");
    }

    /// VM-011 / Gemini: streaming with tools must not emit multi-line JSON.
    #[tokio::test]
    async fn test_vm011_gemini_streaming_with_tools_single_line_frames() {
        let app = create_app(get_test_config(), None, get_embedded_registry()).await;
        let body = r#"{
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}],
            "tools": [{"functionDeclarations": [{"name": "canary_t", "parameters": {"type":"OBJECT"}}]}]
        }"#;
        let resp = app.oneshot(
            Request::builder().method("POST")
                .uri("/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse")
                .header("content-type", "application/json")
                .body(Body::from(body)).unwrap()
        ).await.unwrap();
        let text = String::from_utf8(drain_body(resp).await).unwrap();
        assert_data_frames_are_single_line_json(&text, "gemini stream+tools");

        // Also assert the functionCall echoes the caller's name (combined
        // VM-010 + VM-011 check for the streaming path).
        assert!(text.contains("\"name\":\"canary_t\""),
            "Gemini streaming tool_call must echo caller's name; got:\n{}", text);
    }

    /// VM-011 / Anthropic: streaming with tools must emit typed tool_use
    /// events, not word-chunked JSON text deltas. Before the fix, the stream
    /// parsed as "content_block_delta with text_delta" containing fragments
    /// of the tool_use JSON body.
    #[tokio::test]
    async fn test_vm011_anthropic_streaming_with_tools_emits_tool_use_block() {
        let app = create_app(get_test_config(), None, get_embedded_registry()).await;
        let body = r#"{
            "model": "claude", "max_tokens": 50, "stream": true,
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [{"name": "canary_t", "description": "x", "input_schema": {"type": "object"}}]
        }"#;
        let resp = app.oneshot(
            Request::builder().method("POST").uri("/v1/messages")
                .header("content-type", "application/json")
                .body(Body::from(body)).unwrap()
        ).await.unwrap();
        let text = String::from_utf8(drain_body(resp).await).unwrap();

        // content_block_start's content_block must be tool_use, not text.
        assert!(
            text.contains("\"type\":\"tool_use\""),
            "Anthropic streaming with tools must emit tool_use content block; got:\n{}", text
        );
        assert!(
            text.contains("\"name\":\"canary_t\""),
            "tool_use block must echo caller's name; got:\n{}", text
        );
        // Deltas must be input_json_delta, not text_delta fragments of JSON body.
        assert!(
            text.contains("\"type\":\"input_json_delta\""),
            "Anthropic streaming tool deltas must be input_json_delta; got:\n{}", text
        );
        // stop_reason in message_delta must be tool_use.
        assert!(
            text.contains("\"stop_reason\":\"tool_use\""),
            "message_delta must carry stop_reason=tool_use; got:\n{}", text
        );
    }

    /// VM-011 / OpenAI (regression guard): streaming-with-tools single-line
    /// framing was already correct before the VM-011 fix; assert it stays that
    /// way after the shared content extractor changes.
    #[tokio::test]
    async fn test_vm011_openai_streaming_with_tools_single_line_frames() {
        let app = create_app(get_test_config(), None, get_embedded_registry()).await;
        let body = r#"{
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hi"}],
            "stream": true,
            "tools": [{"type": "function", "function": {"name": "canary_t", "parameters": {}}}]
        }"#;
        let resp = app.oneshot(
            Request::builder().method("POST").uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(body)).unwrap()
        ).await.unwrap();
        let text = String::from_utf8(drain_body(resp).await).unwrap();
        // OpenAI's stream uses "data: ..." + "data: [DONE]". Skip the [DONE]
        // sentinel which is a valid OpenAI convention but not valid JSON.
        let body_without_done: String = text.lines()
            .filter(|l| *l != "data: [DONE]")
            .collect::<Vec<_>>()
            .join("\n");
        assert_data_frames_are_single_line_json(&body_without_done, "openai stream+tools");
    }

    // ─── --isolated mode (issue #6) — end-to-end via HTTP ─────────────────
    //
    // These exercise the full request → match → render path with the binary
    // configured in isolated mode. Complements the registry-level tests in
    // provider.rs which prove the loader gating.

    /// Build an isolated-mode AppState from a temp config-dir + a custom
    /// provider with an inline `response_body` (no template lookup needed).
    /// Returns the configured Router ready for `.oneshot()`.
    async fn build_isolated_app(
        test_dir_name: &str,
        provider_yamls: &[(&str, &str)],  // (file_name, yaml_body)
    ) -> (axum::Router, std::path::PathBuf) {
        let dir = std::env::current_dir().unwrap()
            .join(format!("target/test_isolated_e2e/{}", test_dir_name));
        if dir.exists() { std::fs::remove_dir_all(&dir).unwrap(); }
        std::fs::create_dir_all(dir.join("providers")).unwrap();
        for (name, body) in provider_yamls {
            std::fs::write(dir.join("providers").join(name), body).unwrap();
        }

        let mut config = get_test_config();
        config.isolated = true;
        config.config_dir = dir.clone();

        let registry = crate::provider::init_registry_with_options(&dir, true);
        let app = create_app(config, None, registry).await;
        (app, dir)
    }

    /// /v1/models in isolated mode reflects ONLY user-loaded providers —
    /// not the bundled ~20, and not a canned `gpt-4` fallback either.
    #[tokio::test]
    async fn test_isolated_models_endpoint_lists_only_user_providers() {
        let (app, dir) = build_isolated_app(
            "models_only_user",
            &[("solo.yaml", "name: \"solo\"\nmatcher: \"^/solo$\"\nresponse_body: \"{}\"\n")],
        ).await;

        let resp = app.oneshot(
            Request::builder().uri("/v1/models").body(Body::empty()).unwrap()
        ).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap()
        ).unwrap();

        let ids: Vec<&str> = body["data"].as_array().unwrap()
            .iter()
            .map(|m| m["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, vec!["solo"],
            "isolated /v1/models must list only user providers; got {:?}", ids);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Isolated + empty dir: /v1/models returns an empty list, NOT the
    /// hard-coded gpt-4 fallback (which would mislead users who deliberately
    /// locked the surface down).
    #[tokio::test]
    async fn test_isolated_models_endpoint_empty_dir_returns_empty_list() {
        let (app, dir) = build_isolated_app("models_empty", &[]).await;

        let resp = app.oneshot(
            Request::builder().uri("/v1/models").body(Body::empty()).unwrap()
        ).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap()
        ).unwrap();

        let list = body["data"].as_array().expect("data must be an array");
        assert_eq!(list.len(), 0,
            "isolated + empty dir must return an empty model list, not a fake gpt-4; got {:?}", list);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Bundled provider routes that previously worked (because of the
    /// embedded openai.yaml) now 404 in isolated mode — and the 404 body
    /// includes the isolated-mode hint so users can diagnose quickly.
    #[tokio::test]
    async fn test_isolated_bundled_routes_404_with_helpful_message() {
        let (app, dir) = build_isolated_app("bundled_404",
            &[("solo.yaml", "name: \"solo\"\nmatcher: \"^/solo$\"\nresponse_body: \"{}\"\n")],
        ).await;

        // /v1/chat/completions is a registered Axum route, but no provider
        // matches it in isolated mode (we only registered /solo).
        let resp = app.oneshot(
            Request::builder().method("POST").uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from("{}")).unwrap()
        ).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let text = String::from_utf8(
            axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap().to_vec()
        ).unwrap();
        assert!(text.contains("isolated mode"),
            "404 body should mention isolated mode to help users diagnose; got: {}", text);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Sanity: a user provider that doesn't reference any bundled template
    /// (uses inline `response_body`) still serves successfully in isolated mode.
    #[tokio::test]
    async fn test_isolated_user_provider_serves_normally() {
        let (app, dir) = build_isolated_app("user_serves",
            &[("solo.yaml",
               "name: \"solo\"\nmatcher: \"^/solo$\"\nresponse_body: '{\"hello\":\"world\"}'\n")],
        ).await;

        let resp = app.oneshot(
            Request::builder().method("POST").uri("/solo")
                .header("content-type", "application/json")
                .body(Body::from("{}")).unwrap()
        ).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap()
        ).unwrap();
        assert_eq!(body["hello"], "world");
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Isolated mode's 404 hint differs from non-isolated; in default mode
    /// the 404 says "no bundled or disk provider matches" (different hint).
    /// Regression guard: non-isolated 404 still works and mentions /status.
    #[tokio::test]
    async fn test_non_isolated_404_message_unchanged() {
        let app = create_app(get_test_config(), None, get_test_registry()).await;
        // /unknown is not matched by the tiny test registry.
        let resp = app.oneshot(
            Request::builder().method("POST").uri("/totally/unknown/path")
                .header("content-type", "application/json")
                .body(Body::from("{}")).unwrap()
        ).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let text = String::from_utf8(
            axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap().to_vec()
        ).unwrap();
        // Default-mode 404 should NOT say "isolated mode".
        assert!(!text.contains("isolated mode"),
            "default-mode 404 must not mention isolated mode; got: {}", text);
        assert!(text.contains("/status"),
            "404 should suggest /status for diagnosis; got: {}", text);
    }

    /// /status JSON exposes the isolated flag so users can self-diagnose.
    #[tokio::test]
    async fn test_isolated_status_endpoint_exposes_flag() {
        let (app, dir) = build_isolated_app("status_flag",
            &[("solo.yaml", "name: \"solo\"\nmatcher: \"^/solo$\"\nresponse_body: \"{}\"\n")],
        ).await;

        let resp = app.oneshot(
            Request::builder().uri("/status").body(Body::empty()).unwrap()
        ).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap()
        ).unwrap();
        assert_eq!(body["isolated"], serde_json::json!(true),
            "/status must expose isolated=true so users can confirm runtime mode");
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Default mode /status still works and reports isolated=false.
    #[tokio::test]
    async fn test_default_mode_status_reports_isolated_false() {
        let config = get_test_config();
        let app = create_app(config, None, get_test_registry()).await;
        let resp = app.oneshot(
            Request::builder().uri("/status").body(Body::empty()).unwrap()
        ).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap()
        ).unwrap();
        assert_eq!(body["isolated"], serde_json::json!(false));
    }

    // ---------------------------------------------------------------------
    // VM-012 — OpenAI Responses (/v1/responses) tool calling.
    //
    // Issue #8: the streaming path rendered a separate template set that had
    // no tool branch at all, so a request carrying `tools` streamed generic
    // filler text while the non-streaming path correctly returned a
    // function_call. The structured chunk reached the template and Tera
    // stringified it, emitting the literal "[[object]]" as the text delta.
    //
    // VM-011 covered streaming+tools for Gemini, Anthropic and OpenAI chat but
    // omitted Responses, which is how this shipped. These tests close that gap
    // for BOTH modes and BOTH turns of a tool loop.
    // ---------------------------------------------------------------------

    /// Sends a /v1/responses request and returns the raw body.
    async fn post_responses(body: &'static str) -> String {
        let app = create_app(get_test_config(), None, get_embedded_registry()).await;
        let resp = app.oneshot(
            Request::builder().method("POST").uri("/v1/responses")
                .header("content-type", "application/json")
                .body(Body::from(body)).unwrap()
        ).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        String::from_utf8(drain_body(resp).await).unwrap()
    }

    const RESPONSES_WITH_TOOLS_STREAM: &str = r#"{
        "model": "gpt-4o-mini",
        "stream": true,
        "input": [{"role": "user", "content": "Call the get_weather tool now."}],
        "tools": [{"type": "function", "name": "get_weather",
                   "parameters": {"type": "object", "properties": {}}}]
    }"#;

    /// The reported bug: streaming with `tools` must emit a function_call
    /// output item, not text deltas.
    #[tokio::test]
    async fn test_vm012_responses_streaming_with_tools_emits_function_call() {
        let text = post_responses(RESPONSES_WITH_TOOLS_STREAM).await;

        assert!(!text.contains("[[object]]"),
            "streaming must not stringify the structured tool chunk; got:\n{}", text);
        assert!(text.contains(r#""type":"function_call""#),
            "stream must contain a function_call output item; got:\n{}", text);
        assert!(text.contains("response.function_call_arguments.delta"),
            "stream must emit function_call_arguments.delta; got:\n{}", text);
        assert!(text.contains("response.function_call_arguments.done"),
            "stream must emit function_call_arguments.done; got:\n{}", text);
        assert!(!text.contains("response.output_text.delta"),
            "tool-mode stream must not emit text deltas; got:\n{}", text);
        assert!(text.contains(r#""name":"get_weather""#),
            "the caller's tool name must be echoed back; got:\n{}", text);
    }

    /// A client building the final item from lifecycle events needs
    /// output_item.done before response.completed — skipping it for function
    /// calls is a known interoperability failure in other mocks.
    #[tokio::test]
    async fn test_vm012_responses_streaming_tool_call_emits_output_item_done() {
        let text = post_responses(RESPONSES_WITH_TOOLS_STREAM).await;

        let done_at = text.find("response.output_item.done")
            .unwrap_or_else(|| panic!("missing output_item.done; got:\n{}", text));
        let completed_at = text.find("response.completed")
            .unwrap_or_else(|| panic!("missing response.completed; got:\n{}", text));
        assert!(done_at < completed_at,
            "output_item.done must precede response.completed; got:\n{}", text);

        // function_call_arguments.done must carry `name`: without it a client
        // cannot tell which function to invoke.
        let done_line = text.lines()
            .find(|l| l.contains("response.function_call_arguments.done") && l.starts_with("data:"))
            .unwrap_or_else(|| panic!("no data line for arguments.done; got:\n{}", text));
        assert!(done_line.contains(r#""name":"get_weather""#),
            "function_call_arguments.done must include the function name; got:\n{}", done_line);
    }

    /// Regression guard: without `tools`, streaming must still be plain text.
    /// The fix must not turn every stream into a tool call.
    #[tokio::test]
    async fn test_vm012_responses_streaming_without_tools_still_streams_text() {
        let text = post_responses(r#"{
            "model": "gpt-4o-mini",
            "stream": true,
            "input": [{"role": "user", "content": "hello"}]
        }"#).await;

        assert!(text.contains("response.output_text.delta"),
            "text mode must emit output_text deltas; got:\n{}", text);
        assert!(!text.contains(r#""type":"function_call""#),
            "no tools in request => no function_call; got:\n{}", text);
        assert!(text.contains("response.content_part.added"),
            "text mode must still open a content part; got:\n{}", text);
    }

    /// Agentic loop termination, streaming: a `function_call_output` item in
    /// `input` means the tool already answered, so the mock must synthesize
    /// text. Returning another tool call loops the client forever.
    #[tokio::test]
    async fn test_vm012_responses_streaming_tool_loop_terminates_on_result() {
        let text = post_responses(r#"{
            "model": "gpt-4o-mini",
            "stream": true,
            "input": [
                {"role": "user", "content": "What is the weather?"},
                {"type": "function_call", "call_id": "c1", "name": "get_weather", "arguments": "{}"},
                {"type": "function_call_output", "call_id": "c1", "output": "15C cloudy"}
            ],
            "tools": [{"type": "function", "name": "get_weather", "parameters": {}}]
        }"#).await;

        assert!(!text.contains(r#""type":"function_call""#),
            "must not call the tool again after a tool result; got:\n{}", text);
        assert!(text.contains("response.output_text.delta"),
            "must synthesize text after a tool result; got:\n{}", text);
        assert!(!text.contains("[[object]]"),
            "termination must be real text, not a stringified chunk; got:\n{}", text);
    }

    /// Agentic loop termination, non-streaming. This path branched on the
    /// presence of `tools` alone, so it looped too — the bug was not
    /// streaming-only.
    #[tokio::test]
    async fn test_vm012_responses_tool_loop_terminates_on_result() {
        let text = post_responses(r#"{
            "model": "gpt-4o-mini",
            "input": [
                {"role": "user", "content": "What is the weather?"},
                {"type": "function_call", "call_id": "c1", "name": "get_weather", "arguments": "{}"},
                {"type": "function_call_output", "call_id": "c1", "output": "15C cloudy"}
            ],
            "tools": [{"type": "function", "name": "get_weather", "parameters": {}}]
        }"#).await;

        let json: serde_json::Value = serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("invalid JSON ({}); got:\n{}", e, text));
        let kinds: Vec<&str> = json["output"].as_array().unwrap().iter()
            .filter_map(|o| o["type"].as_str()).collect();
        assert!(!kinds.contains(&"function_call"),
            "must not call the tool again after a tool result; got:\n{}", text);
        assert!(kinds.contains(&"message"),
            "must return a message after a tool result; got:\n{}", text);
    }

    /// Regression guard: with `tools` and no tool result, non-streaming must
    /// still return a function_call.
    #[tokio::test]
    async fn test_vm012_responses_tools_without_result_still_returns_function_call() {
        let text = post_responses(r#"{
            "model": "gpt-4o-mini",
            "input": [{"role": "user", "content": "What is the weather?"}],
            "tools": [{"type": "function", "name": "get_weather", "parameters": {}}]
        }"#).await;

        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(json["output"][0]["type"], "function_call",
            "tools without a result must still produce a function_call; got:\n{}", text);
    }

    /// The Responses API accepts `input` as a bare string as well as an array.
    /// Tool detection filters `input`, and Tera's filter() raises on a
    /// non-array — which turned this valid request into a 500. Both modes are
    /// covered because all four templates do the same detection.
    #[tokio::test]
    async fn test_vm012_responses_accepts_string_input_with_tools() {
        let text = post_responses(r#"{
            "model": "gpt-4o-mini",
            "input": "What is the weather?",
            "tools": [{"type": "function", "name": "get_weather", "parameters": {}}]
        }"#).await;
        let json: serde_json::Value = serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("string input must render valid JSON ({}); got:\n{}", e, text));
        assert_eq!(json["output"][0]["type"], "function_call",
            "string input with tools must still produce a function_call; got:\n{}", text);

        let stream = post_responses(r#"{
            "model": "gpt-4o-mini",
            "stream": true,
            "input": "What is the weather?",
            "tools": [{"type": "function", "name": "get_weather", "parameters": {}}]
        }"#).await;
        assert!(stream.contains("response.function_call_arguments.done"),
            "string input must stream a function call, not error; got:\n{}", stream);
    }

    /// An empty `tools` array is not a tool request. Guards against treating
    /// `"tools": []` as truthy and emitting a spurious function call.
    #[tokio::test]
    async fn test_vm012_responses_empty_tools_array_streams_text() {
        let text = post_responses(r#"{
            "model": "gpt-4o-mini",
            "stream": true,
            "input": [{"role": "user", "content": "hello"}],
            "tools": []
        }"#).await;
        assert!(!text.contains(r#""type":"function_call""#),
            "empty tools array must not produce a function_call; got:\n{}", text);
        assert!(text.contains("response.output_text.delta"),
            "empty tools array must stream text; got:\n{}", text);
    }

    // ---------------------------------------------------------------------
    // VM-013 — tool calling for the OpenAI-compatible providers.
    //
    // azure-openai, mistral, openai-compatible, groq and openrouter had no
    // tool branching in EITHER mode: they rendered templates that echoed the
    // request back regardless of `tools`. Unlike VM-012 this was never a
    // regression — it had simply never been built — but it left the mock
    // unusable for testing tool-calling clients against those endpoints.
    //
    // Each provider keeps its own envelope (groq's timing fields,
    // openrouter's provider/total_cost), so the templates stay separate and
    // these tests assert both the tool call AND the provider-specific fields.
    // ---------------------------------------------------------------------

    /// POSTs to an arbitrary path and returns the raw body.
    async fn post_path(uri: &'static str, body: &'static str) -> String {
        let app = create_app(get_test_config(), None, get_embedded_registry()).await;
        let resp = app.oneshot(
            Request::builder().method("POST").uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body)).unwrap()
        ).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "{} should return 200", uri);
        String::from_utf8(drain_body(resp).await).unwrap()
    }

    const COMPAT_TOOLS_BODY: &str = r#"{
        "model": "gpt-4",
        "messages": [{"role": "user", "content": "What is the weather?"}],
        "tools": [{"type": "function", "function": {
            "name": "get_weather", "parameters": {"type": "object", "properties": {}}}}]
    }"#;

    const COMPAT_TOOL_RESULT_BODY: &str = r#"{
        "model": "gpt-4",
        "messages": [
            {"role": "user", "content": "What is the weather?"},
            {"role": "assistant", "tool_calls": [{"id": "call_1", "type": "function",
             "function": {"name": "get_weather", "arguments": "{}"}}]},
            {"role": "tool", "tool_call_id": "call_1", "content": "15C cloudy"}
        ],
        "tools": [{"type": "function", "function": {
            "name": "get_weather", "parameters": {"type": "object", "properties": {}}}}]
    }"#;

    /// Asserts a chat.completion body carries a tool call echoing the caller's
    /// tool name, with finish_reason=tool_calls.
    fn assert_tool_call(text: &str, who: &str) {
        let json: serde_json::Value = serde_json::from_str(text)
            .unwrap_or_else(|e| panic!("{}: invalid JSON ({}); got:\n{}", who, e, text));
        let choice = &json["choices"][0];
        assert_eq!(choice["finish_reason"], "tool_calls",
            "{}: finish_reason must be tool_calls; got:\n{}", who, text);
        assert_eq!(choice["message"]["tool_calls"][0]["function"]["name"], "get_weather",
            "{}: must echo the caller's tool name; got:\n{}", who, text);
    }

    /// Asserts a chat.completion body terminated the loop with plain text.
    fn assert_terminated(text: &str, who: &str) {
        let json: serde_json::Value = serde_json::from_str(text)
            .unwrap_or_else(|e| panic!("{}: invalid JSON ({}); got:\n{}", who, e, text));
        let choice = &json["choices"][0];
        assert_eq!(choice["finish_reason"], "stop",
            "{}: must stop after a tool result; got:\n{}", who, text);
        assert!(choice["message"].get("tool_calls").is_none()
                || choice["message"]["tool_calls"].is_null(),
            "{}: must not call the tool again after a result; got:\n{}", who, text);
        assert!(choice["message"]["content"].as_str().is_some(),
            "{}: must synthesize text after a tool result; got:\n{}", who, text);
    }

    #[tokio::test]
    async fn test_vm013_azure_returns_tool_calls() {
        let uri = "/openai/deployments/gpt-4/chat/completions";
        assert_tool_call(&post_path(uri, COMPAT_TOOLS_BODY).await, "azure");
        assert_terminated(&post_path(uri, COMPAT_TOOL_RESULT_BODY).await, "azure");
    }

    #[tokio::test]
    async fn test_vm013_mistral_returns_tool_calls() {
        let uri = "/v1/mistral/chat/completions";
        assert_tool_call(&post_path(uri, COMPAT_TOOLS_BODY).await, "mistral");
        assert_terminated(&post_path(uri, COMPAT_TOOL_RESULT_BODY).await, "mistral");
    }

    /// Groq keeps its own usage block; the tool branch must not drop it.
    #[tokio::test]
    async fn test_vm013_groq_returns_tool_calls_and_keeps_usage_shape() {
        let uri = "/groq/v1/chat/completions";
        let text = post_path(uri, COMPAT_TOOLS_BODY).await;
        assert_tool_call(&text, "groq");

        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(json["usage"].get("queue_time").is_some(),
            "groq tool response must keep its usage shape; got:\n{}", text);
        assert_eq!(json["system_fingerprint"], "fp_mock_groq");

        assert_terminated(&post_path(uri, COMPAT_TOOL_RESULT_BODY).await, "groq");
    }

    /// OpenRouter adds `provider` and usage.total_cost; the tool branch must
    /// not drop them.
    #[tokio::test]
    async fn test_vm013_openrouter_returns_tool_calls_and_keeps_envelope() {
        let uri = "/api/v1/chat/completions";
        let text = post_path(uri, COMPAT_TOOLS_BODY).await;
        assert_tool_call(&text, "openrouter");

        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(json["provider"], "MockProvider",
            "openrouter tool response must keep `provider`; got:\n{}", text);
        assert!(json["usage"].get("total_cost").is_some(),
            "openrouter tool response must keep total_cost; got:\n{}", text);

        assert_terminated(&post_path(uri, COMPAT_TOOL_RESULT_BODY).await, "openrouter");
    }

    /// The shared OpenAI-compatible stream chunk template must emit a
    /// tool_calls delta for a structured chunk. Without the branch the chunk
    /// is stringified as "[[object]]" — the same defect as issue #8.
    #[tokio::test]
    async fn test_vm013_compat_streaming_emits_tool_calls_delta() {
        let text = post_path("/openai/deployments/gpt-4/chat/completions", r#"{
            "model": "gpt-4",
            "stream": true,
            "messages": [{"role": "user", "content": "What is the weather?"}],
            "tools": [{"type": "function", "function": {
                "name": "get_weather", "parameters": {}}}]
        }"#).await;

        assert!(!text.contains("[[object]]"),
            "structured chunk must not be stringified; got:\n{}", text);
        assert!(text.contains(r#""tool_calls""#),
            "stream must carry a tool_calls delta; got:\n{}", text);
        assert!(text.contains(r#""name":"get_weather""#),
            "stream must echo the caller's tool name; got:\n{}", text);
    }

    /// Regression guard: without tools these providers must still echo text.
    #[tokio::test]
    async fn test_vm013_compat_without_tools_still_returns_text() {
        let text = post_path("/v1/mistral/chat/completions", r#"{
            "model": "mistral-medium",
            "messages": [{"role": "user", "content": "hello"}]
        }"#).await;

        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(json["choices"][0]["finish_reason"], "stop");
        assert!(json["choices"][0]["message"].get("tool_calls").is_none()
                || json["choices"][0]["message"]["tool_calls"].is_null(),
            "no tools in request => no tool_calls; got:\n{}", text);
    }

    /// Real OpenAI keeps the response id stable for the life of a stream, and
    /// the item id stable across that item's events. The templates previously
    /// called uuid() per interpolation, so every event carried a different id
    /// and strict clients correlating by id would break.
    #[tokio::test]
    async fn test_vm012_responses_stream_ids_are_stable_across_events() {
        let text = post_responses(RESPONSES_WITH_TOOLS_STREAM).await;

        let ids: std::collections::HashSet<&str> = text.match_indices("\"id\":\"resp_")
            .map(|(i, _)| {
                let start = i + "\"id\":\"".len();
                let rest = &text[start..];
                &rest[..rest.find('"').unwrap()]
            })
            .collect();
        assert_eq!(ids.len(), 1,
            "response id must be identical in every event of one stream, found {:?}; got:\n{}",
            ids, text);

        let item_ids: std::collections::HashSet<&str> = text.match_indices("\"item_id\":\"")
            .map(|(i, _)| {
                let start = i + "\"item_id\":\"".len();
                let rest = &text[start..];
                &rest[..rest.find('"').unwrap()]
            })
            .collect();
        assert!(item_ids.len() <= 1,
            "function call item_id must be stable, found {:?}; got:\n{}", item_ids, text);
    }
}
