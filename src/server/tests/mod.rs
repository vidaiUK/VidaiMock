use super::*;
use crate::tenancy::{
    build_runtime_store, AdminAuthConfig, TenancyConfig, TenancyMode, TenantRequestMetrics,
    TenantStore, TenantStoreHandle,
};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use clap::Parser;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use tower::ServiceExt; // for oneshot

mod tenancy;

fn get_test_config() -> AppConfig {
    AppConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        workers: 1,
        log_level: "debug".to_string(),
        config_dir: PathBuf::from("config"),
        tenancy: crate::tenancy::TenancyConfig::default(),
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
        reload_args: None,
        chaos: crate::config::ChaosConfig {
            enabled: false,
            drop_pct: 0.0,
            malformed_pct: 0.0,
            trickle_ms: 0,
            disconnect_pct: 0.0,
        },
    }
}

fn get_test_store() -> Arc<TenantStore> {
    let mut registry = crate::provider::ProviderRegistry::new();
    registry
        .add_provider(crate::provider::ProviderConfig {
            name: "openai".to_string(),
            matcher: "^/v1/chat/completions$".to_string(),
            request_mapping: HashMap::new(),
            response_template: None,
            response_body: Some(
                r#"{"id": "test-id", "object": "chat.completion", "model": "test-model"}"#
                    .to_string(),
            ),
            stream: None,
            status_code: None,
            error_template: None,
            priority: 0,
        })
        .unwrap();

    Arc::new(TenantStore::new(
        TenancyMode::Single,
        PathBuf::from("config"),
        crate::tenancy::TenancyConfig::default(),
        crate::config::LatencyConfig::default(),
        crate::config::ChaosConfig::default(),
        "x-admin-key".to_string(),
        None,
        "x-tenant".to_string(),
        Arc::new(crate::tenancy::TenantRuntime {
            label: crate::tenancy::DEFAULT_TENANT_ID.to_string(),
            template_metadata: crate::tenancy::TenantTemplateMetadata {
                id: crate::tenancy::DEFAULT_TENANT_ID.to_string(),
                ..crate::tenancy::TenantTemplateMetadata::default()
            },
            registry: Arc::new(registry),
            requires_key: false,
            management_auth_header: "x-tenant-admin-key".to_string(),
            management_auth_secret: None,
            latency: crate::config::LatencyConfig::default(),
            chaos: crate::config::ChaosConfig::default(),
        }),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        std::collections::HashSet::new(),
        std::collections::HashSet::new(),
    ))
}

fn get_test_store_handle() -> Arc<TenantStoreHandle> {
    Arc::new(TenantStoreHandle::new(get_test_store()))
}

/// Loads the bundled embedded providers (OpenAI, Anthropic, Gemini, etc.)
/// into the default tenant runtime for wire-shape regression tests.
fn get_embedded_registry() -> Arc<TenantStoreHandle> {
    let mut registry = crate::provider::ProviderRegistry::new();
    // Pass a non-existent dir so only embedded providers load (filesystem
    // overlay empty). This mirrors the behaviour when a binary is run
    // without --config-dir on a machine without a config/ directory.
    registry
        .load_from_dir(&PathBuf::from("non_existent_dir_regression"))
        .unwrap();
    Arc::new(TenantStoreHandle::new(Arc::new(TenantStore::new(
        TenancyMode::Single,
        PathBuf::from("config"),
        crate::tenancy::TenancyConfig::default(),
        crate::config::LatencyConfig::default(),
        crate::config::ChaosConfig::default(),
        "x-admin-key".to_string(),
        None,
        "x-tenant".to_string(),
        Arc::new(crate::tenancy::TenantRuntime {
            label: crate::tenancy::DEFAULT_TENANT_ID.to_string(),
            template_metadata: crate::tenancy::TenantTemplateMetadata {
                id: crate::tenancy::DEFAULT_TENANT_ID.to_string(),
                ..crate::tenancy::TenantTemplateMetadata::default()
            },
            registry: Arc::new(registry),
            requires_key: false,
            management_auth_header: "x-tenant-admin-key".to_string(),
            management_auth_secret: None,
            latency: crate::config::LatencyConfig::default(),
            chaos: crate::config::ChaosConfig::default(),
        }),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        std::collections::HashSet::new(),
        std::collections::HashSet::new(),
    ))))
}

/// Collects an Axum streaming response body into a single Vec<u8>.
async fn drain_body(resp: axum::http::Response<Body>) -> Vec<u8> {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    bytes.to_vec()
}

#[tokio::test]
async fn test_health_check() {
    let config = get_test_config();
    let app = create_app(config, None, get_test_store_handle()).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_mock_endpoint() {
    let config = get_test_config();
    let app = create_app(config, None, get_test_store_handle()).await;

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
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    assert_eq!(body["id"], "test-id");
}

#[tokio::test]
async fn test_echo_endpoint() {
    let mut config = get_test_config();
    config.endpoints = vec![crate::config::EndpointConfig {
        path: "/echo".to_string(),
        format: "echo".to_string(),
        content_type: None,
    }];

    let app = create_app(config, None, get_test_store_handle()).await;
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

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(&bytes[..], body_content.as_bytes());
}

#[tokio::test]
async fn test_multiprovider_endpoints() {
    let config = get_test_config();

    // Setup registry with multiple providers
    let mut registry = crate::provider::ProviderRegistry::new();

    // OpenAI
    registry
        .add_provider(crate::provider::ProviderConfig {
            name: "openai".to_string(),
            matcher: "^/v1/chat/completions$".to_string(),
            request_mapping: HashMap::new(),
            response_template: None,
            response_body: Some(r#"{"object": "chat.completion"}"#.to_string()),
            stream: None,
            status_code: None,
            error_template: None,
            priority: 0,
        })
        .unwrap();

    // Anthropic
    registry
        .add_provider(crate::provider::ProviderConfig {
            name: "anthropic".to_string(),
            matcher: "^/v1/messages$".to_string(),
            request_mapping: HashMap::new(),
            response_template: None,
            response_body: Some(r#"{"type": "message"}"#.to_string()),
            stream: None,
            status_code: None,
            error_template: None,
            priority: 0,
        })
        .unwrap();

    // Gemini
    registry
        .add_provider(crate::provider::ProviderConfig {
            name: "gemini".to_string(),
            matcher: "^/gemini$".to_string(),
            request_mapping: HashMap::new(),
            response_template: None,
            response_body: Some(r#"{"candidates": []}"#.to_string()),
            stream: None,
            status_code: None,
            error_template: None,
            priority: 0,
        })
        .unwrap();

    let app = create_app(
        config,
        None,
        Arc::new(TenantStoreHandle::new(Arc::new(TenantStore::new(
            TenancyMode::Single,
            PathBuf::from("config"),
            crate::tenancy::TenancyConfig::default(),
            crate::config::LatencyConfig::default(),
            crate::config::ChaosConfig::default(),
            "x-admin-key".to_string(),
            None,
            "x-tenant".to_string(),
            Arc::new(crate::tenancy::TenantRuntime {
                label: crate::tenancy::DEFAULT_TENANT_ID.to_string(),
                template_metadata: crate::tenancy::TenantTemplateMetadata {
                    id: crate::tenancy::DEFAULT_TENANT_ID.to_string(),
                    ..crate::tenancy::TenantTemplateMetadata::default()
                },
                registry: Arc::new(registry),
                requires_key: false,
                management_auth_header: "x-tenant-admin-key".to_string(),
                management_auth_secret: None,
                latency: crate::config::LatencyConfig::default(),
                chaos: crate::config::ChaosConfig::default(),
            }),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            std::collections::HashSet::new(),
            std::collections::HashSet::new(),
        )))),
    )
    .await;

    // Test OpenAI
    let response = app
        .clone()
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
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(body_str.contains("chat.completion"));

    // Test Anthropic
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(body_str.contains("type\": \"message"));

    // Test Gemini
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/gemini")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(body_str.contains("candidates"));
}

#[tokio::test]
async fn test_status_endpoint() {
    let config = get_test_config();
    let app = create_app(config, None, get_test_store_handle()).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    assert_eq!(body["status"], "ok");
    assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(body["port"], 0);
    assert_eq!(body.as_object().unwrap().len(), 3);
    assert!(body.get("tenancy").is_none());
    assert!(body.get("latency").is_none());
    assert!(body.get("endpoints").is_none());
}

async fn response_text(response: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

// SDK compatibility regressions stay with the generic server tests because
// they exercise shared response shapes rather than tenant-specific behavior.
#[tokio::test]
async fn test_vm001_openai_stream_emits_finish_reason() {
    let app = create_app(get_test_config(), None, get_embedded_registry()).await;
    let body = r#"{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hi"}],"max_tokens":5,"stream":true}"#;

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = drain_body(resp).await;
    let text = String::from_utf8(bytes).unwrap();

    assert!(
        text.contains(r#""finish_reason":"stop""#),
        "stream must include a chunk with finish_reason=stop before [DONE]; got:\n{}",
        text
    );
    assert!(
        text.ends_with("data: [DONE]\n\n"),
        "stream must end with 'data: [DONE]\\n\\n'; got tail:\n{}",
        &text[text.len().saturating_sub(100)..]
    );
}

/// VM-002: every SSE event must end with a blank line (\n\n), including
/// the usage chunk that precedes [DONE] when stream_options.include_usage
/// is set. openai-python's SSE parser fails with JSONDecodeError otherwise.
#[tokio::test]
async fn test_vm002_openai_stream_usage_chunk_has_blank_line_terminator() {
    let app = create_app(get_test_config(), None, get_embedded_registry()).await;
    let body = r#"{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hi"}],"max_tokens":5,"stream":true,"stream_options":{"include_usage":true}}"#;

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let text = String::from_utf8(drain_body(resp).await).unwrap();

    // The usage chunk is the one with "usage":{"prompt_tokens":... .
    // It must be followed by "\n\ndata: [DONE]" — not just "\ndata:".
    assert!(
        text.contains("\"usage\":{"),
        "stream must include a usage chunk when include_usage=true"
    );
    let usage_idx = text.find("\"usage\":{").unwrap();
    let tail = &text[usage_idx..];
    // Find the end of the usage JSON object, then assert \n\n follows.
    let done_idx = tail
        .find("data: [DONE]")
        .expect("expected [DONE] frame after usage chunk");
    let between = &tail[..done_idx];
    assert!(
        between.ends_with("\n\n"),
        "usage chunk must be terminated with blank line before [DONE]; got bytes: {:?}",
        between
            .as_bytes()
            .iter()
            .rev()
            .take(6)
            .rev()
            .collect::<Vec<_>>()
    );
}

/// VM-005: ?chaos_status=500 query param produces an OpenAI-shaped JSON
/// error envelope (provider error_template), not plain text or the
/// success body. Composes with the URL so gateways can register one
/// "broken" endpoint and another "healthy" endpoint against the same mock.
#[tokio::test]
async fn test_vm005_chaos_status_query_returns_provider_shape_error() {
    let app = create_app(get_test_config(), None, get_embedded_registry()).await;
    let body = r#"{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hi"}]}"#;

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions?chaos_status=503")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    let content_type = resp.headers().get("content-type").cloned();
    let text = String::from_utf8(drain_body(resp).await).unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&text).expect("chaos response must be valid JSON (not plain text)");

    assert_eq!(content_type.unwrap().to_str().unwrap(), "application/json");
    assert!(
        json.get("error").is_some(),
        "OpenAI error envelope must have top-level 'error' key; got: {}",
        text
    );
}

/// VM-005 (streaming): ?chaos_status=500 on a streaming request must
/// return a non-streaming HTTP error (real providers don't send SSE
/// errors). Assert status + JSON body.
#[tokio::test]
async fn test_vm005_chaos_status_on_streaming_returns_non_streaming_error() {
    let app = create_app(get_test_config(), None, get_embedded_registry()).await;
    let body =
        r#"{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hi"}],"stream":true}"#;

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions?chaos_status=500")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    // Body must be JSON, not SSE (no "data:" prefixes).
    let text = String::from_utf8(drain_body(resp).await).unwrap();
    assert!(
        !text.starts_with("data:"),
        "chaos on streaming must return non-streaming JSON body; got:\n{}",
        text
    );
    let _json: serde_json::Value =
        serde_json::from_str(&text).expect("streaming chaos response body must be valid JSON");
}

/// VM-005: X-Vidai-Chaos-Drop=100 must return a JSON error envelope,
/// not plain text. Previously returned "Simulated Internal Server Error"
/// as text/plain which no SDK can parse.
#[tokio::test]
async fn test_vm005_chaos_drop_header_returns_json_error_envelope() {
    let app = create_app(get_test_config(), None, get_embedded_registry()).await;
    let body = r#"{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hi"}]}"#;

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .header("x-vidai-chaos-drop", "100")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let content_type = resp
        .headers()
        .get("content-type")
        .cloned()
        .expect("missing content-type header");
    assert_eq!(
        content_type.to_str().unwrap(),
        "application/json",
        "chaos response must carry application/json content-type"
    );

    let text = String::from_utf8(drain_body(resp).await).unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&text).expect("chaos response must be valid JSON");
    assert!(
        json.get("error").is_some(),
        "OpenAI error envelope must have top-level 'error' key"
    );
}

/// VM-006: Anthropic streaming events must each be separated by a blank
/// line (`\n\n`) so SDK SSE parsers treat them as distinct events.
/// The symptom was "events out of order" but the root cause was frames
/// merging because blank lines were stripped by the raw-frame renderer.
#[tokio::test]
async fn test_vm006_anthropic_stream_events_have_blank_line_separators() {
    let app = create_app(get_test_config(), None, get_embedded_registry()).await;
    let body = r#"{"model":"claude","max_tokens":30,"stream":true,"messages":[{"role":"user","content":"hi"}]}"#;

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let text = String::from_utf8(drain_body(resp).await).unwrap();

    // First event must be message_start.
    let first_event_line = text
        .lines()
        .find(|l| l.starts_with("event: "))
        .expect("no event: lines found in Anthropic stream");
    assert_eq!(
        first_event_line, "event: message_start",
        "Anthropic streaming must start with message_start; got: {}",
        first_event_line
    );

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
            assert!(
                absolute >= 1 && bytes[absolute - 1] == b'\n',
                "event at byte {} not preceded by blank line; context: {:?}",
                absolute,
                &text[absolute.saturating_sub(20)..(absolute + 20).min(text.len())]
            );
        }
        events_seen += 1;
        pos = absolute + 1;
    }
    assert!(
        events_seen >= 7,
        "expected at least 7 Anthropic event types; found {}",
        events_seen
    );
}

/// VM-006 (order): even with framing fixed, the event order itself
/// must be strict: message_start first, then content_block_start before
/// any content_block_delta.
#[tokio::test]
async fn test_vm006_anthropic_stream_event_order() {
    let app = create_app(get_test_config(), None, get_embedded_registry()).await;
    let body = r#"{"model":"claude","max_tokens":30,"stream":true,"messages":[{"role":"user","content":"hi"}]}"#;

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    let text = String::from_utf8(drain_body(resp).await).unwrap();
    let events: Vec<&str> = text.lines().filter(|l| l.starts_with("event: ")).collect();

    // Find indices of the three ordering-critical events.
    let pos_start = events
        .iter()
        .position(|e| *e == "event: message_start")
        .expect("missing message_start");
    let pos_block_start = events
        .iter()
        .position(|e| *e == "event: content_block_start")
        .expect("missing content_block_start");
    let pos_first_delta = events
        .iter()
        .position(|e| *e == "event: content_block_delta")
        .expect("missing content_block_delta");
    let pos_stop = events
        .iter()
        .position(|e| *e == "event: message_stop")
        .expect("missing message_stop");

    assert!(
        pos_start < pos_block_start,
        "message_start must precede content_block_start"
    );
    assert!(
        pos_block_start < pos_first_delta,
        "content_block_start must precede content_block_delta"
    );
    assert!(
        pos_first_delta < pos_stop,
        "content_block_delta must precede message_stop"
    );
}

/// VM-007: Anthropic `/v1/messages` without `max_tokens` must return
/// HTTP 400 with an Anthropic-shaped `{"type":"error","error":{...}}`
/// envelope, matching real Anthropic validation.
#[tokio::test]
async fn test_vm007_anthropic_missing_max_tokens_returns_400() {
    let app = create_app(get_test_config(), None, get_embedded_registry()).await;
    let body = r#"{"model":"claude","messages":[{"role":"user","content":"hi"}]}"#;

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "missing max_tokens must return HTTP 400"
    );

    let text = String::from_utf8(drain_body(resp).await).unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&text).expect("error body must be valid JSON");
    assert_eq!(
        json.get("type").and_then(|v| v.as_str()),
        Some("error"),
        "Anthropic error envelope must have type=error; got: {}",
        text
    );
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

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
}

/// VM-008: Gemini streamGenerateContent with alt=sse must NOT emit a
/// `data: [DONE]` sentinel. Real Gemini terminates on connection close;
/// a [DONE] frame causes google-genai SDK to raise UnknownApiResponseError.
#[tokio::test]
async fn test_vm008_gemini_stream_no_done_sentinel() {
    let app = create_app(get_test_config(), None, get_embedded_registry()).await;
    let body = r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}]}"#;

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let text = String::from_utf8(drain_body(resp).await).unwrap();

    assert!(
        !text.contains("[DONE]"),
        "Gemini stream must not emit [DONE] sentinel; got:\n{}",
        text
    );
    // Must still contain at least one real data: frame.
    assert!(
        text.contains("data: {"),
        "Gemini stream should contain at least one JSON data frame"
    );
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

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    let text = String::from_utf8(drain_body(resp).await).unwrap();

    // Collect JSON payloads from every `data:` frame.
    let frames: Vec<serde_json::Value> = text
        .lines()
        .filter_map(|l| l.strip_prefix("data: "))
        .filter(|j| j.starts_with('{'))
        .filter_map(|j| serde_json::from_str(j).ok())
        .collect();

    assert!(
        frames.len() >= 2,
        "expected at least 2 frames (>=1 intermediate + 1 final); got {}",
        frames.len()
    );

    // Final chunk must carry finishReason=STOP and usageMetadata.
    let final_frame = frames.last().unwrap();
    let final_finish = final_frame["candidates"][0]["finishReason"].as_str();
    assert_eq!(
        final_finish,
        Some("STOP"),
        "final chunk must carry finishReason=STOP; got frame: {}",
        final_frame
    );
    assert!(
        final_frame.get("usageMetadata").is_some(),
        "final chunk must carry usageMetadata; got: {}",
        final_frame
    );

    // Every intermediate chunk must carry finishReason=null and must NOT
    // carry usageMetadata. This is the specific shape google-genai SDK
    // requires to continue iterating through the stream.
    for (i, frame) in frames.iter().take(frames.len() - 1).enumerate() {
        let finish = &frame["candidates"][0]["finishReason"];
        assert!(
            finish.is_null(),
            "intermediate chunk {} must have finishReason=null; got {}; frame: {}",
            i,
            finish,
            frame
        );
        assert!(
            frame.get("usageMetadata").is_none(),
            "intermediate chunk {} must NOT carry usageMetadata; got frame: {}",
            i,
            frame
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
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
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
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .header("x-mock-status", "429")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    // Body is rendered from error_template (since status is an error).
    let text = String::from_utf8(drain_body(resp).await).unwrap();
    let _: serde_json::Value = serde_json::from_str(&text).expect("body must be valid JSON");
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
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let text = String::from_utf8(drain_body(resp).await).unwrap();
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    let choice = &json["choices"][0];
    let msg = &choice["message"];

    assert_eq!(
        choice["finish_reason"], "stop",
        "finish_reason must be 'stop' after tool result, not 'tool_calls'; got:\n{}",
        text
    );
    assert!(
        msg.get("tool_calls").is_none() || msg["tool_calls"].is_null(),
        "response must NOT include tool_calls after a tool result; got:\n{}",
        text
    );
    assert!(
        msg["content"].as_str().is_some(),
        "response must include text content after a tool result; got:\n{}",
        text
    );
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
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
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
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let text = String::from_utf8(drain_body(resp).await).unwrap();
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();

    assert_eq!(
        json["stop_reason"], "end_turn",
        "stop_reason must be 'end_turn' after tool_result, not 'tool_use'; got:\n{}",
        text
    );
    let first_block_type = json["content"][0]["type"].as_str();
    assert_eq!(
        first_block_type,
        Some("text"),
        "first content block must be 'text' after tool_result, not 'tool_use'; got:\n{}",
        text
    );
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
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
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
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1beta/models/gemini-2.5-flash:generateContent")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let text = String::from_utf8(drain_body(resp).await).unwrap();
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    let first_part = &json["candidates"][0]["content"]["parts"][0];

    assert!(
        first_part.get("text").is_some(),
        "first part must carry text after functionResponse; got:\n{}",
        text
    );
    assert!(
        first_part.get("functionCall").is_none(),
        "first part must NOT be functionCall after functionResponse; got:\n{}",
        text
    );
    // finishMessage is only emitted on the function-call branch; absence here
    // confirms the loop-terminating branch rendered.
    assert!(
        json["candidates"][0].get("finishMessage").is_none(),
        "finishMessage must be absent on the text-response branch; got:\n{}",
        text
    );
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
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1beta/models/gemini-2.5-flash:generateContent")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let text = String::from_utf8(drain_body(resp).await).unwrap();
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert!(json["candidates"][0]["content"]["parts"][0]
        .get("functionCall")
        .is_some());
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
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8(drain_body(resp).await).unwrap()).unwrap();
    let tc = &json["choices"][0]["message"]["tool_calls"][0];
    assert_eq!(
        tc["function"]["name"], "canary_tool_a",
        "OpenAI response must echo caller's tool name; got: {}",
        tc
    );
    assert_eq!(
        tc["function"]["arguments"], "{}",
        "OpenAI args should default to empty object; got: {}",
        tc
    );
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
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let text = String::from_utf8(drain_body(resp).await).unwrap();
    // Find the tool_calls frame and assert it carries the caller's name.
    assert!(
        text.contains("\"name\":\"canary_tool_s\""),
        "OpenAI streaming tool_calls must echo caller's name; got:\n{}",
        text
    );
    assert!(
        !text.contains("\"name\":\"get_weather\""),
        "OpenAI streaming must not emit the hardcoded demo name; got:\n{}",
        text
    );
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
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8(drain_body(resp).await).unwrap()).unwrap();
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
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1beta/models/gemini-2.5-flash:generateContent")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8(drain_body(resp).await).unwrap()).unwrap();
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
        let Some(payload) = line.strip_prefix("data: ") else {
            continue;
        };
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
        assert!(
            !payload.contains('\n'),
            "{label}: data frame must be single-line; got embedded \\n in:\n{payload}"
        );
        frame_count += 1;
    }
    assert!(
        frame_count >= 1,
        "{label}: expected at least one data: frame; got 0. body:\n{body}"
    );
}

/// VM-011 / Gemini: streaming with tools must not emit multi-line JSON.
#[tokio::test]
async fn test_vm011_gemini_streaming_with_tools_single_line_frames() {
    let app = create_app(get_test_config(), None, get_embedded_registry()).await;
    let body = r#"{
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}],
            "tools": [{"functionDeclarations": [{"name": "canary_t", "parameters": {"type":"OBJECT"}}]}]
        }"#;
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let text = String::from_utf8(drain_body(resp).await).unwrap();
    assert_data_frames_are_single_line_json(&text, "gemini stream+tools");

    // Also assert the functionCall echoes the caller's name (combined
    // VM-010 + VM-011 check for the streaming path).
    assert!(
        text.contains("\"name\":\"canary_t\""),
        "Gemini streaming tool_call must echo caller's name; got:\n{}",
        text
    );
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
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let text = String::from_utf8(drain_body(resp).await).unwrap();

    // content_block_start's content_block must be tool_use, not text.
    assert!(
        text.contains("\"type\":\"tool_use\""),
        "Anthropic streaming with tools must emit tool_use content block; got:\n{}",
        text
    );
    assert!(
        text.contains("\"name\":\"canary_t\""),
        "tool_use block must echo caller's name; got:\n{}",
        text
    );
    // Deltas must be input_json_delta, not text_delta fragments of JSON body.
    assert!(
        text.contains("\"type\":\"input_json_delta\""),
        "Anthropic streaming tool deltas must be input_json_delta; got:\n{}",
        text
    );
    // stop_reason in message_delta must be tool_use.
    assert!(
        text.contains("\"stop_reason\":\"tool_use\""),
        "message_delta must carry stop_reason=tool_use; got:\n{}",
        text
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
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let text = String::from_utf8(drain_body(resp).await).unwrap();
    // OpenAI's stream uses "data: ..." + "data: [DONE]". Skip the [DONE]
    // sentinel which is a valid OpenAI convention but not valid JSON.
    let body_without_done: String = text
        .lines()
        .filter(|l| *l != "data: [DONE]")
        .collect::<Vec<_>>()
        .join("\n");
    assert_data_frames_are_single_line_json(&body_without_done, "openai stream+tools");
}

/// M-2 regression: a streaming provider whose response_body contains an
/// invalid Tera template must return HTTP 500 — not HTTP 200 with an empty
/// body. Previously render failure was silenced with unwrap_or_default() so
/// callers received a well-formed SSE envelope with zero content chunks.
#[tokio::test]
async fn test_streaming_body_render_failure_returns_500() {
    let mut registry = crate::provider::ProviderRegistry::new();
    registry
        .add_provider(crate::provider::ProviderConfig {
            name: "openai".to_string(),
            matcher: "^/v1/chat/completions$".to_string(),
            request_mapping: HashMap::new(),
            response_template: None,
            // Intentionally broken Tera syntax — unclosed variable tag.
            response_body: Some("{{ broken_template".to_string()),
            stream: Some(crate::provider::ProviderStreamConfig {
                enabled: true,
                format: None,
                encoding: None,
                frame_format: None,
                lifecycle: None,
            }),
            status_code: None,
            error_template: None,
            priority: 100,
        })
        .unwrap();

    let store = Arc::new(TenantStoreHandle::new(Arc::new(TenantStore::new(
        TenancyMode::Single,
        PathBuf::from("config"),
        crate::tenancy::TenancyConfig::default(),
        crate::config::LatencyConfig::default(),
        crate::config::ChaosConfig::default(),
        "x-admin-key".to_string(),
        None,
        "x-tenant".to_string(),
        Arc::new(crate::tenancy::TenantRuntime {
            label: crate::tenancy::DEFAULT_TENANT_ID.to_string(),
            template_metadata: crate::tenancy::TenantTemplateMetadata {
                id: crate::tenancy::DEFAULT_TENANT_ID.to_string(),
                ..crate::tenancy::TenantTemplateMetadata::default()
            },
            registry: Arc::new(registry),
            requires_key: false,
            management_auth_header: "x-tenant-admin-key".to_string(),
            management_auth_secret: None,
            latency: crate::config::LatencyConfig::default(),
            chaos: crate::config::ChaosConfig::default(),
        }),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        std::collections::HashSet::new(),
        std::collections::HashSet::new(),
    ))));

    let app = create_app(get_test_config(), None, store).await;

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"stream":true}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "a broken streaming template must return 500, not 200 with empty body"
    );
    // Confirm the response is not an SSE stream (real error, not silent empty stream).
    let body_text = String::from_utf8(drain_body(resp).await).unwrap();
    assert!(
        !body_text.starts_with("data:"),
        "error response must not be an SSE stream; got:\n{}",
        body_text
    );
}

fn make_streaming_store_with_body(response_body: &str) -> Arc<TenantStoreHandle> {
    make_streaming_store_with_status_and_body(response_body, None)
}

fn make_streaming_store_with_status_and_body(
    response_body: &str,
    status_code: Option<&str>,
) -> Arc<TenantStoreHandle> {
    let mut registry = crate::provider::ProviderRegistry::new();
    registry
        .add_provider(crate::provider::ProviderConfig {
            name: "openai".to_string(),
            matcher: "^/v1/chat/completions$".to_string(),
            request_mapping: HashMap::new(),
            response_template: None,
            response_body: Some(response_body.to_string()),
            stream: Some(crate::provider::ProviderStreamConfig {
                enabled: true,
                format: None,
                encoding: None,
                frame_format: None,
                lifecycle: None,
            }),
            status_code: status_code.map(str::to_string),
            error_template: None,
            priority: 100,
        })
        .unwrap();

    Arc::new(TenantStoreHandle::new(Arc::new(TenantStore::new(
        TenancyMode::Single,
        PathBuf::from("config"),
        crate::tenancy::TenancyConfig::default(),
        crate::config::LatencyConfig::default(),
        crate::config::ChaosConfig::default(),
        "x-admin-key".to_string(),
        None,
        "x-tenant".to_string(),
        Arc::new(crate::tenancy::TenantRuntime {
            label: crate::tenancy::DEFAULT_TENANT_ID.to_string(),
            template_metadata: crate::tenancy::TenantTemplateMetadata {
                id: crate::tenancy::DEFAULT_TENANT_ID.to_string(),
                ..crate::tenancy::TenantTemplateMetadata::default()
            },
            registry: Arc::new(registry),
            requires_key: false,
            management_auth_header: "x-tenant-admin-key".to_string(),
            management_auth_secret: None,
            latency: crate::config::LatencyConfig::default(),
            chaos: crate::config::ChaosConfig::default(),
        }),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        std::collections::HashSet::new(),
        std::collections::HashSet::new(),
    ))))
}

/// Regression: when a streaming request triggers an error-status path and the
/// response_body contains an invalid Tera template, the handler must return
/// HTTP 500 with an error message, not HTTP <status> with a silently empty body
/// (which was the previous unwrap_or_default() behaviour).
#[tokio::test]
async fn test_streaming_error_status_render_failure_returns_500() {
    // Provider is configured to always return 422 and render the response_body
    // as the error body — but the body is an intentionally broken Tera template.
    let store = make_streaming_store_with_status_and_body(
        "{{ broken_template", // invalid Tera — unclosed tag
        Some("422"),
    );

    let app = create_app(get_test_config(), None, store).await;

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"stream":true}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "a broken error-status body template inside the streaming path must return 500, \
         not the error status with a silently empty body"
    );
    let body_text = String::from_utf8(drain_body(resp).await).unwrap();
    assert!(
        !body_text.is_empty(),
        "error response body must not be empty"
    );
    assert!(
        !body_text.starts_with("data:"),
        "error response must not be an SSE stream; got:\n{}",
        body_text
    );
}

/// Regression: a valid streaming error-status path (where the response_body
/// renders successfully) must return the configured error status with a JSON
/// body, not start an SSE stream.
#[tokio::test]
async fn test_streaming_error_status_with_valid_template_returns_json_error() {
    // Provider configured to return 422. response_body is a plain JSON string
    // with no Tera variables so it renders cleanly in both the pre-stream phase
    // and the error-body phase. The handler must return the 422 status with
    // the body as JSON, not as an SSE stream.
    let store = make_streaming_store_with_status_and_body(
        r#"{"error":{"code":422,"message":"unprocessable"}}"#,
        Some("422"),
    );

    let app = create_app(get_test_config(), None, store).await;

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"stream":true}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "streaming error-status with valid body template must return the configured status code"
    );
    let body_text = String::from_utf8(drain_body(resp).await).unwrap();
    let body: serde_json::Value = serde_json::from_str(&body_text)
        .expect("error response body must be valid JSON");
    assert_eq!(body["error"]["code"], 422);
    assert!(
        !body_text.starts_with("data:"),
        "error body must not be SSE-framed"
    );
}

/// Regression: reload_requires_restart must detect changes to tenancy config
/// (mode, tenant_header, admin_auth) so tenancy drift cannot silently pass
/// through a live reload.
#[test]
fn test_reload_requires_restart_detects_tenancy_change() {
    use crate::config::AppConfig;
    use crate::tenancy::{AdminAuthConfig, TenancyConfig, TenancyMode};
    use std::path::PathBuf;

    let base = AppConfig {
        host: "0.0.0.0".to_string(),
        port: 8100,
        workers: 1,
        log_level: "info".to_string(),
        config_dir: PathBuf::from("config"),
        tenancy: TenancyConfig {
            mode: TenancyMode::Single,
            tenants_dir: PathBuf::from("tenants"),
            tenant_header: "x-tenant".to_string(),
            admin_auth: AdminAuthConfig::default(),
        },
        latency: crate::config::LatencyConfig::default(),
        chaos: crate::config::ChaosConfig::default(),
        endpoints: vec![],
        response_file: None,
        reload_args: None,
    };

    // No change — no restart required.
    assert!(
        base.reload_requires_restart(&base).is_empty(),
        "identical config must not require restart"
    );

    // Tenant header renamed.
    let mut changed_header = base.clone();
    changed_header.tenancy.tenant_header = "x-org".to_string();
    let fields = base.reload_requires_restart(&changed_header);
    assert!(
        fields.contains(&"tenancy"),
        "changing tenant_header must require restart; got {:?}",
        fields
    );

    // Tenancy mode switched.
    let mut changed_mode = base.clone();
    changed_mode.tenancy.mode = TenancyMode::Multi;
    let fields = base.reload_requires_restart(&changed_mode);
    assert!(
        fields.contains(&"tenancy"),
        "changing tenancy mode must require restart; got {:?}",
        fields
    );

    // Admin auth header renamed.
    let mut changed_auth = base.clone();
    changed_auth.tenancy.admin_auth.header = "x-root".to_string();
    let fields = base.reload_requires_restart(&changed_auth);
    assert!(
        fields.contains(&"tenancy"),
        "changing admin_auth header must require restart; got {:?}",
        fields
    );
}

/// Regression: log_level = "off" must map to LevelFilter::OFF.
/// Tests the actual `level_filter_from_str` function used in main.rs so the
/// test is not brittle to mapping changes.
#[test]
fn test_log_level_off_maps_to_level_filter_off() {
    use tracing_subscriber::filter::LevelFilter;

    let cases: &[(&str, LevelFilter)] = &[
        ("off", LevelFilter::OFF),
        ("error", LevelFilter::ERROR),
        ("warn", LevelFilter::WARN),
        ("info", LevelFilter::INFO),
        ("debug", LevelFilter::DEBUG),
        ("unknown", LevelFilter::INFO), // unknown falls back to INFO
        ("OFF", LevelFilter::OFF),      // case-insensitive
    ];

    for (input, expected) in cases {
        let actual = crate::level_filter_from_str(input);
        assert_eq!(
            actual, *expected,
            "log_level '{}' must map to {:?}, got {:?}",
            input, expected, actual
        );
    }
}
