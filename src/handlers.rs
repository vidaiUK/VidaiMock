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

use axum::{
    body::Bytes,
    extract::{Json, OriginalUri, Query},
    http::{StatusCode, HeaderValue, HeaderMap},
    response::{IntoResponse, Response},
    Extension,
};
use serde_json::Value;
use crate::replacer::Replacer;
use futures::stream::{self};
use futures::StreamExt;
use std::convert::Infallible;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::time::{sleep, Duration};
use rand::Rng;

use crate::config::AppConfig;
// use crate::formats::load_response; // Function removed/unused

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub registry: Arc<crate::provider::ProviderRegistry>,
}

/// Apply configured latency delay for realistic simulation mode
async fn apply_latency(config: &AppConfig, headers: &HeaderMap) {
    let mut base = config.latency.base_ms;
    let mut jitter_pct = config.latency.jitter_pct;

    // Header Overrides
    if let Some(val) = headers.get("x-vidai-latency") {
        if let Ok(ms) = val.to_str().unwrap_or_default().parse::<u64>() {
            base = ms;
        }
    }
    if let Some(val) = headers.get("x-vidai-jitter") {
        if let Ok(pct) = val.to_str().unwrap_or_default().parse::<f64>() {
            jitter_pct = pct;
        }
    }

    if base > 0 {
        let mut final_delay = base;
        if jitter_pct > 0.0 {
            let variance = (base as f64 * jitter_pct) as u64;
            // Use rng for simple jitter
            let jitter = rand::rng().random_range(0..=(variance * 2)) as i64 - variance as i64;
             final_delay = (base as i64 + jitter).max(0) as u64;
        }
        sleep(Duration::from_millis(final_delay)).await;
    }
}

/// Result of rolling for chaos: either force a status override (so downstream
/// rendering picks the error_template), emit a deliberately malformed body,
/// or pass through.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ChaosOutcome {
    None,
    /// Force this status code; downstream will render provider error_template.
    ForceStatus(u16),
    /// Emit a malformed (non-JSON) body with HTTP 200.
    Malformed,
}

/// Rolls the configured chaos dice and tells the caller what to do.
/// Returns ChaosOutcome::None when no chaos fires.
fn roll_chaos(config: &AppConfig, headers: &HeaderMap) -> ChaosOutcome {
    let mut drop_pct = config.chaos.drop_pct;
    let mut malformed_pct = config.chaos.malformed_pct;

    if let Some(val) = headers.get("x-vidai-chaos-drop") {
        if let Ok(val) = val.to_str().unwrap_or_default().parse::<f64>() {
            drop_pct = val.clamp(0.0, 100.0);
        }
    }
    if let Some(val) = headers.get("x-vidai-chaos-malformed") {
        if let Ok(val) = val.to_str().unwrap_or_default().parse::<f64>() {
            malformed_pct = val.clamp(0.0, 100.0);
        }
    }

    drop_pct = drop_pct.clamp(0.0, 100.0);
    malformed_pct = malformed_pct.clamp(0.0, 100.0);

    let mut rng = rand::rng();

    if drop_pct > 0.0 && rng.random_bool(drop_pct / 100.0) {
        return ChaosOutcome::ForceStatus(500);
    }
    if malformed_pct > 0.0 && rng.random_bool(malformed_pct / 100.0) {
        return ChaosOutcome::Malformed;
    }

    ChaosOutcome::None
}

/// Indicates how the response status was resolved.
/// `ChaosOverride` means a failure was explicitly injected and the response body
/// should be rendered from the provider's `error_template` if available.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum StatusSource {
    /// Default 200 or provider-configured non-error status.
    Default,
    /// Status injected by X-Mock-Status header, ?chaos_status query, or
    /// X-Vidai-Chaos-Drop — consumers should render the error template.
    ChaosOverride,
}

/// Resolves the HTTP status code for a response.
/// Priority (first match wins):
///   1. Dice-roll chaos trigger (X-Vidai-Chaos-Drop etc.)
///   2. X-Mock-Status header (client-controlled)
///   3. ?chaos_status query param (server-controlled via URL)
///   4. Provider status_code config (static or Tera-rendered)
///   5. Default 200
/// Returns `(status, source)` so callers can distinguish default vs override.
fn resolve_status_code(
    chaos: ChaosOutcome,
    headers: &HeaderMap,
    query_params: &HashMap<String, String>,
    status_code: Option<&str>,
    context: &tera::Context,
    state: &Arc<AppState>,
) -> (StatusCode, StatusSource) {
    // 1. Chaos dice roll (X-Vidai-Chaos-Drop). Highest precedence so
    // randomized failure-injection always wins over deterministic config.
    if let ChaosOutcome::ForceStatus(code) = chaos {
        if let Ok(status) = StatusCode::from_u16(code) {
            return (status, StatusSource::ChaosOverride);
        }
    }

    // 2. Header override — client-controlled.
    if let Some(val) = headers.get("x-mock-status") {
        if let Ok(code) = val.to_str().unwrap_or_default().parse::<u16>() {
            if let Ok(status) = StatusCode::from_u16(code) {
                return (status, StatusSource::ChaosOverride);
            }
        }
    }

    // 3. Query-param override — server-controlled via URL registered in a
    // routing layer's provider-config. Lets one URL be "broken" and another
    // "healthy" without per-request header forwarding.
    if let Some(val) = query_params.get("chaos_status") {
        if let Ok(code) = val.parse::<u16>() {
            if let Ok(status) = StatusCode::from_u16(code) {
                return (status, StatusSource::ChaosOverride);
            }
        }
    }

    // 4. Provider-configured status_code (static or Tera-rendered).
    // A value containing either `{{` (expression) or `{%` (statement) is
    // rendered as Tera; otherwise treated as a literal string.
    match status_code {
        None => (StatusCode::OK, StatusSource::Default),
        Some(raw) => {
            let code_str = if raw.contains("{{") || raw.contains("{%") {
                state.registry.render_str(raw, context).unwrap_or_default()
            } else {
                raw.to_string()
            };
            let status = code_str.trim().parse::<u16>()
                .ok()
                .and_then(|code| StatusCode::from_u16(code).ok())
                .unwrap_or(StatusCode::OK);
            (status, StatusSource::Default)
        }
    }
}

pub async fn health_check() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({"status": "ok"})))
}

pub async fn status_handler(
    Extension(state): Extension<Arc<AppState>>,
) -> Json<AppConfig> {
    Json(state.config.as_ref().clone())
}

pub async fn mock_handler(
    Extension(state): Extension<Arc<AppState>>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    Query(query_params): Query<HashMap<String, String>>,
    Json(request_json): Json<Value>,
) -> impl IntoResponse {
    // Apply latency simulation (with header override support)
    apply_latency(&state.config, &headers).await;

    // Roll chaos dice. Result becomes an injected status_code override that
    // flows through the normal rendering pipeline, so the body gets the
    // provider's error_template (OpenAI-shape, Anthropic-shape, etc.) instead
    // of a plain-text 500 that no SDK can parse.
    let chaos = roll_chaos(&state.config, &headers);
    if let ChaosOutcome::Malformed = chaos {
        let mut resp = Response::new(axum::body::Body::from(
            "This is not valid JSON { missing_brace"));
        *resp.status_mut() = StatusCode::OK;
        return resp.into_response();
    }

    // Check for streaming request
    let is_streaming_path = uri.path().contains(":streamGenerateContent")
        || uri.path().contains("/converse-stream")
        || uri.path().contains("/invoke-with-response-stream")
        || uri.path().ends_with("/stream");

    if is_streaming_path || request_json.get("stream").and_then(|v| v.as_bool()).unwrap_or(false) {
        return streaming_handler(
            Extension(state),
            OriginalUri(uri),
            headers,
            Query(query_params),
            Json(request_json),
        ).await.into_response();
    }

    let path = uri.path();
    
    // 1. Try to find a matching provider in the registry
    if let Some(provider) = state.registry.find_provider(path) {
        let mut header_map = HashMap::new();
        for (k, v) in headers.iter() {
            header_map.insert(k.to_string(), v.to_str().unwrap_or_default().to_string());
        }
        let path_segments: Vec<String> = path.split('/').filter(|s| !s.is_empty()).map(|s| s.to_string()).collect();

        // Build context
        let mut context = Replacer::build_context(
            Some(&request_json),
            &header_map,
            &query_params,
            &path_segments,
            &provider.name
        );

        // Process request_mapping: Extract variables using Tera (e.g. prompt extraction)
        for (key, expr) in &provider.request_mapping {
            // Render the expression string using the current context
            // Render the expression string using the registry's Tera (so filters are available)
            let val = match state.registry.render_str(expr, &context) {
                Ok(s) => s,
                Err(e) => {
                     tracing::warn!("Failed to evaluate mapping '{}': {}", key, e);
                     continue;
                }
            };
            context.insert(key, &val);
        }

        // Resolve status first so we know whether to render error_template or response_template.
        let (status, source) = resolve_status_code(
            chaos, &headers, &query_params, provider.status_code.as_deref(), &context, &state);

        // Insert status into context so error templates can self-describe
        // (e.g. `{"error": {"code": {{ status_code }}}}`).
        context.insert("status_code", &status.as_u16());

        // When a chaos override triggers an error, render the provider's error_template
        // if configured; otherwise fall through to the normal response_template so
        // behaviour is unchanged for providers that haven't opted into error shapes.
        // Route to error_template whenever status is >= 400, regardless of
        // whether the trigger was a chaos override or the provider's own
        // status_code expression (e.g. VM-007: Anthropic evaluates
        // `status_code: "{% if json.max_tokens... %}400{% else %}200{% endif %}"`).
        // This lets providers emit correctly-shaped error envelopes for
        // their own validation rules and chaos injection through the same
        // template pipeline.
        let is_error_status = status.is_client_error() || status.is_server_error();
        let _ = source; // kept for future behaviour splits; currently unused
        let error_template = if is_error_status {
            provider.error_template.as_deref()
        } else {
            None
        };

        let chosen_template_path = error_template.or(provider.response_template.as_deref());
        let rendered = if let Some(template_path) = chosen_template_path {
             match state.registry.tera.render(template_path, &context) {
                 Ok(s) => s,
                 Err(e) => {
                     tracing::error!("Template render failed: {}", e);
                     return (StatusCode::INTERNAL_SERVER_ERROR, format!("Template Error: {}", e)).into_response();
                 }
             }
        } else if let Some(body) = &provider.response_body {
             // Use registry.render_str for ad-hoc strings to ensure filters are available
             match state.registry.render_str(body, &context) {
                 Ok(s) => s,
                 Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("Template Error: {}", e)).into_response()
             }
        } else {
             return (StatusCode::NOT_FOUND, "No response template defined").into_response();
        };

        let mut response = Response::new(axum::body::Body::from(rendered));
        *response.status_mut() = status;
        response.headers_mut().insert(
            axum::http::header::CONTENT_TYPE,
             HeaderValue::from_static("application/json"),
        );
        return response.into_response();
    }

    // No matching provider found. Give the user a useful hint based on
    // runtime state: in isolated mode the most likely cause is "the user
    // forgot to put a provider in their config-dir"; in default mode it's
    // usually "this path isn't a known endpoint".
    let hint = if state.config.isolated {
        " (isolated mode — only providers from --config-dir are loaded; check /status)"
    } else {
        " (no bundled or disk provider matches; check /status for the loaded set)"
    };
    (
        StatusCode::NOT_FOUND,
        format!("No provider matched this request route.{}", hint),
    ).into_response()
}

pub async fn models_handler(
    Extension(state): Extension<Arc<AppState>>,
    OriginalUri(_uri): OriginalUri,
    headers: HeaderMap,
) -> impl IntoResponse {
    apply_latency(&state.config, &headers).await;

    let mut model_list = Vec::new();
    
    // Add models from loaded providers
    for provider in &state.registry.providers {
        model_list.push(serde_json::json!({
            "id": provider.name,
            "object": "model",
            "created": 1700000000,
            "owned_by": "vidai"
        }));
    }

    // Note: if the registry is empty (only possible in --isolated mode with
    // a config-dir that has no providers), we return an empty list rather
    // than a canned `gpt-4` entry — a hard-coded fake would mislead users
    // who deliberately locked the surface down.

    let response_json = serde_json::json!({
        "object": "list",
        "data": model_list
    });
    
    Response::builder()
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(axum::body::Body::from(serde_json::to_string(&response_json).unwrap()))
        .unwrap()
}

pub async fn streaming_handler(
    Extension(state): Extension<Arc<AppState>>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    Query(query_params): Query<HashMap<String, String>>,
    Json(request_json): Json<Value>,
) -> Response {
    let path = uri.path();

    // Streams don't have a natural "streaming error response" — real providers
    // respond with a non-streaming HTTP error when the upstream fails.
    // Roll chaos up front; if malformed, return a plain malformed body.
    let stream_chaos = roll_chaos(&state.config, &headers);
    if let ChaosOutcome::Malformed = stream_chaos {
        let mut resp = Response::new(axum::body::Body::from(
            "This is not valid JSON { missing_brace"));
        *resp.status_mut() = StatusCode::OK;
        return resp;
    }

    // 1. Try to find provider
    if let Some(provider_ref) = state.registry.find_provider(path) {
         let provider = provider_ref.clone();
         if let Some(stream_config) = provider.stream.clone() {
             let mut header_map = HashMap::new();
             for (k, v) in headers.iter() {
                 header_map.insert(k.to_string(), v.to_str().unwrap_or_default().to_string());
             }
             let path_segments: Vec<String> = path.split('/').filter(|s| !s.is_empty()).map(|s| s.to_string()).collect();
             
             let state_clone = state.clone(); // Clone Arc<AppState> for the stream
             
             // Extract context variables
             let mut base_context = Replacer::build_context(
                Some(&request_json),
                &header_map,
                &query_params,
                &path_segments,
                &provider.name
            );

            // Process request_mapping
            for (key, expr) in &provider.request_mapping {
                let val = match state.registry.render_str(expr, &base_context) {
                    Ok(s) => s,
                    Err(e) => {
                         tracing::warn!("Failed to evaluate mapping '{}': {}", key, e);
                         continue;
                    }
                };
                base_context.insert(key, &val);
            }

            // Simulate chunks (for now, still split by whitespace until we implement better tokenization/generation)
            let full_response = if let Some(template_path) = &provider.response_template {
                 state.registry.tera.render(template_path, &base_context).unwrap_or_default()
            } else if let Some(body) = &provider.response_body {
                 state.registry.render_str(body, &base_context).unwrap_or_default()
            } else {
                 String::new()
            };

            // Extract content to stream
            // Support Tool Calls: extract_content returns Value
            let (content_val, has_tool_calls) = extract_content_value(&full_response);
            
            let chunks: Vec<serde_json::Value> =
                if has_tool_calls {
                    // If tool calls exist, send as a single chunk.
                    vec![content_val]
                } else if let Some(s) = content_val.as_str() {
                    // Otherwise, split the string content into chunks by whitespace (per-word chunked streaming).
                    s.split_whitespace()
                        .map(|w| serde_json::Value::String(format!("{} ", w)))
                        .collect()
                } else {
                    // Fallback: single chunk if not string or not split-able.
                    vec![content_val]
                };
            
            let lifecycle = stream_config.lifecycle.clone();
            let stream_fmt = stream_config.format.clone();
            let encoding = stream_config.encoding.clone().unwrap_or_default(); // "aws-event-stream" or empty for SSE
            let is_raw_frame = stream_config.frame_format.as_deref() == Some("raw");
            
            let mut trickle_ms = state.config.chaos.trickle_ms;
            let mut disconnect_pct = state.config.chaos.disconnect_pct;

            // Header Overrides for Streaming Chaos
            if let Some(val) = headers.get("x-vidai-chaos-trickle") {
                if let Ok(ms) = val.to_str().unwrap_or_default().parse::<u64>() {
                    trickle_ms = ms;
                }
            }
            if let Some(val) = headers.get("x-vidai-chaos-disconnect") {
                if let Ok(pct) = val.to_str().unwrap_or_default().parse::<f64>() {
                    disconnect_pct = pct;
                }
            }
            
            // Use 20ms default if no trickle configured, to ensure *some* streaming effect
            if trickle_ms == 0 { trickle_ms = 20; }

            let state_inner = state_clone.clone();
            let (stream_status, stream_source) = resolve_status_code(
                stream_chaos, &headers, &query_params,
                provider.status_code.as_deref(), &base_context, &state);

            // If the resolved status is an error, do NOT stream — render the
            // error template (or response template) non-streamingly, matching
            // real providers that return HTTP 4xx/5xx with JSON body instead
            // of an SSE stream when requests are invalid or chaos fires.
            // Applies both to chaos overrides (VM-005) and provider-configured
            // validation (VM-007: Anthropic max_tokens missing -> 400).
            let is_error_status = stream_status.is_client_error() || stream_status.is_server_error();
            let _ = stream_source; // currently unused; kept for future splits
            if is_error_status {
                base_context.insert("status_code", &stream_status.as_u16());
                let chosen_template = provider.error_template.as_deref()
                    .or(provider.response_template.as_deref());
                let body = if let Some(tpl) = chosen_template {
                    state.registry.tera.render(tpl, &base_context).unwrap_or_default()
                } else if let Some(b) = &provider.response_body {
                    state.registry.render_str(b, &base_context).unwrap_or_default()
                } else {
                    String::new()
                };
                let mut resp = Response::new(axum::body::Body::from(body));
                *resp.status_mut() = stream_status;
                resp.headers_mut().insert(
                    axum::http::header::CONTENT_TYPE,
                    HeaderValue::from_static("application/json"),
                );
                return resp;
            }

            let stream = stream::unfold((0, chunks, full_response, base_context, lifecycle, state_inner, trickle_ms, disconnect_pct, stream_fmt, encoding.clone(), is_raw_frame),
                move |(idx, chunks, full_response, mut ctx, lifecycle, state, trickle_ms, disconnect_pct, stream_fmt, encoding, is_raw_frame)| async move {
                
                // Chaos: Early Disconnect
                if disconnect_pct > 0.0 {
                     if rand::rng().random_bool(disconnect_pct / 100.0) {
                         return None;
                     }
                }
                // + 1 is necessary here in order to return DONE
                if idx > chunks.len() + 1 { return None; } 
                                
                let raw_data: Option<String> = if idx == 0 {
                    // Start Event
                    if let Some(lc) = &lifecycle {
                        if let Some(on_start) = &lc.on_start {
                             // Context setup?
                             if let Some(tmpl) = &on_start.template_body {
                                 Some(tera::Tera::one_off(tmpl, &ctx, false).unwrap_or_default())
                             } else if let Some(path) = &on_start.template_path {
                                 Some(state.registry.tera.render(path, &ctx).unwrap_or_default())
                             } else {
                                 None 
                             }
                        } else { None }
                    } else { None }
                } else if idx <= chunks.len() {
                    // Chunk Event
                    let chunk = &chunks[idx-1];
                    ctx.insert("chunk", chunk);
                    
                    if let Some(lc) = &lifecycle {
                        if let Some(on_chunk) = &lc.on_chunk {
                             // Context already has "chunk"
                             if let Some(tmpl) = &on_chunk.template_body {
                                 Some(tera::Tera::one_off(tmpl, &ctx, false).unwrap_or_default())
                             } else if let Some(path) = &on_chunk.template_path {
                                 Some(state.registry.tera.render(path, &ctx).unwrap_or_default())
                             } else {
                                 Some(chunk.to_string())
                             }
                        } else {
                             // Default format from config or raw
                             if let Some(fmt) = &stream_fmt {
                                 let mut chunk_ctx = ctx.clone();
                                 chunk_ctx.insert("chunk", &chunk);
                                 Some(tera::Tera::one_off(fmt, &chunk_ctx, false).unwrap_or_else(|_| chunk.to_string()))
                             } else {
                                 Some(chunk.to_string())
                             }
                        }
                    } else {
                         Some(chunk.to_string())
                    }
                } else {
                    // Stop Event
                    if let Some(lc) = &lifecycle {
                         if let Some(on_stop) = &lc.on_stop {
                             if let Some(tmpl) = &on_stop.template_body {
                                  Some(tera::Tera::one_off(tmpl, &ctx, false).unwrap_or_default())
                             } else if let Some(path) = &on_stop.template_path {
                                 Some(state.registry.tera.render(path, &ctx).unwrap_or_default())
                             } else { None }
                         } else { None }
                    } else { 
                        if encoding == "aws-event-stream" { None } else { Some("[DONE]".to_string()) }
                    }
                };

                let next_idx = idx + 1;
                sleep(Duration::from_millis(trickle_ms)).await;
                
                if let Some(data) = raw_data {
                    // Format output based on encoding
                    if encoding == "aws-event-stream" {
                        let bytes = crate::aws_event_stream::AwsEventStreamEncoder::encode_chunk(&data);
                        Some((Ok::<_, Infallible>(axum::body::Bytes::from(bytes)), (next_idx, chunks, full_response, ctx, lifecycle, state, trickle_ms, disconnect_pct, stream_fmt, encoding, is_raw_frame)))
                    } else if is_raw_frame {
                        // Raw frame mode: template controls SSE framing.
                        //
                        // SSE spec (HTML Living Standard §9.2) requires each event to end
                        // with a blank line. Templates can emit multiple events by separating
                        // them with blank lines — we preserve those so each event is properly
                        // terminated on the wire. A single trailing `\n\n` is appended to
                        // guarantee the final event is terminated even if the template omits it.
                        let mut buf = Vec::new();
                        let mut prev_blank = false;
                        for line in data.lines() {
                            let trimmed = line.trim();
                            if trimmed.is_empty() {
                                if !prev_blank {
                                    buf.extend_from_slice(b"\n");
                                    prev_blank = true;
                                }
                            } else {
                                buf.extend_from_slice(trimmed.as_bytes());
                                buf.extend_from_slice(b"\n");
                                prev_blank = false;
                            }
                        }
                        // Ensure final event is terminated with a blank line if not already.
                        if !prev_blank {
                            buf.extend_from_slice(b"\n");
                        }
                        Some((Ok::<_, Infallible>(axum::body::Bytes::from(buf)), (next_idx, chunks, full_response, ctx, lifecycle, state, trickle_ms, disconnect_pct, stream_fmt, encoding, is_raw_frame)))
                    } else {
                        // SSE Format: auto-wrap with data: prefix
                        let minified = minify_json(data);
                        let mut buf = Vec::new();

                        // Determine event name
                        let mut event_name = None;
                        if idx == 0 {
                             if let Some(lc) = &lifecycle { if let Some(s) = &lc.on_start { event_name = s.event_name.clone(); } }
                        } else if idx <= chunks.len() {
                             if let Some(lc) = &lifecycle { if let Some(s) = &lc.on_chunk { event_name = s.event_name.clone(); } }
                        } else {
                             if let Some(lc) = &lifecycle { if let Some(s) = &lc.on_stop { event_name = s.event_name.clone(); } }
                        }

                        if let Some(name) = event_name {
                             buf.extend_from_slice(format!("event: {}\n", name).as_bytes());
                        }
                        buf.extend_from_slice(format!("data: {}\n", minified).as_bytes());
                        buf.extend_from_slice(b"\n");
                        Some((Ok::<_, Infallible>(axum::body::Bytes::from(buf)), (next_idx, chunks, full_response, ctx, lifecycle, state, trickle_ms, disconnect_pct, stream_fmt, encoding, is_raw_frame)))
                    }
                } else {
                     // Skip this tick (e.g. if start/stop event produced no data)
                     Some((Ok::<_, Infallible>(axum::body::Bytes::new()), (next_idx, chunks, full_response, ctx, lifecycle, state, trickle_ms, disconnect_pct, stream_fmt, encoding, is_raw_frame)))
                }
            });
            
            // Return Response
            let body = axum::body::Body::from_stream(stream);
            let mut response = Response::new(body);

            *response.status_mut() = stream_status;

            if provider.stream.as_ref().unwrap().encoding.as_deref() == Some("aws-event-stream") {
                response.headers_mut().insert(axum::http::header::CONTENT_TYPE, HeaderValue::from_static("application/vnd.amazon.eventstream"));
            } else {
                response.headers_mut().insert(axum::http::header::CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));
                response.headers_mut().insert(axum::http::header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
                response.headers_mut().insert(axum::http::header::CONNECTION, HeaderValue::from_static("keep-alive"));
            }
             
            return response;
         }
    }

    // No provider or stream config found
    (StatusCode::NOT_FOUND, "No streaming configuration found for this provider.").into_response()
}

fn minify_json(s: String) -> String {
    let trimmed = s.trim();
    if (trimmed.starts_with('{') && trimmed.ends_with('}')) || (trimmed.starts_with('[') && trimmed.ends_with(']')) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(trimmed) {
            return serde_json::to_string(&json).unwrap_or(s);
        }
    }
    s
}

fn extract_content_value(json_str: &str) -> (serde_json::Value, bool) {
    // Try to parse as JSON
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(json_str) {
        tracing::debug!("Extracting content from JSON: {:?}", json);
        // OpenAI format: choices[0].message.content or tool_calls
        if let Some(choices) = json.get("choices").and_then(|c| c.as_array()) {
            if let Some(first_choice) = choices.get(0) {
                if let Some(message) = first_choice.get("message") {
                    // Check for tool_calls first
                     if let Some(tool_calls) = message.get("tool_calls") {
                         return (tool_calls.clone(), true);
                     }
                    if let Some(content) = message.get("content") {
                        return (content.clone(), false);
                    }
                }
            }
        }
        
        // OpenAI Responses API: output[].type=="message" -> content[0].text
        if let Some(output_arr) = json.get("output").and_then(|o| o.as_array()) {
            for item in output_arr {
                if item.get("type").and_then(|t| t.as_str()) == Some("function_call") {
                    return (serde_json::json!([item]), true);
                }
                if item.get("type").and_then(|t| t.as_str()) == Some("message") {
                    if let Some(content_arr) = item.get("content").and_then(|c| c.as_array()) {
                        if let Some(first_block) = content_arr.get(0) {
                            if let Some(text) = first_block.get("text") {
                                return (text.clone(), false);
                            }
                        }
                    }
                }
            }
        }
        // Bedrock Converse: output.message.content[0].text
        if let Some(output) = json.get("output") {
            if let Some(message) = output.get("message") {
                if let Some(content_arr) = message.get("content").and_then(|c| c.as_array()) {
                    if let Some(first_block) = content_arr.get(0) {
                        if let Some(text) = first_block.get("text") {
                            return (text.clone(), false);
                        }
                    }
                }
            }
        }
        // Anthropic format: content[0].text OR content[0].type == "tool_use".
        // A tool_use-first content block is the Anthropic analogue of OpenAI
        // tool_calls — emit as a single structured chunk so the streaming
        // template can render it as a typed content_block_start/delta rather
        // than word-chunking the entire JSON body as text.
        if let Some(content) = json.get("content").and_then(|c| c.as_array()) {
            if let Some(first_block) = content.get(0) {
                if first_block.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                    return (first_block.clone(), true);
                }
                if let Some(text) = first_block.get("text") {
                    return (text.clone(), false);
                }
            }
        }
        // Gemini/Vertex format: candidates[0].content.parts[0] is either
        // `{text: ...}` (text response) or `{functionCall: ...}` (tool call).
        // functionCall goes down the same "single structured chunk" path as
        // OpenAI tool_calls / Anthropic tool_use so streaming doesn't try to
        // word-chunk the JSON body.
        if let Some(candidates) = json.get("candidates").and_then(|c| c.as_array()) {
            if let Some(first_candidate) = candidates.get(0) {
                if let Some(content) = first_candidate.get("content") {
                    if let Some(parts) = content.get("parts").and_then(|p| p.as_array()) {
                        if let Some(first_part) = parts.get(0) {
                            if first_part.get("functionCall").is_some() {
                                return (first_part.clone(), true);
                            }
                            if let Some(text) = first_part.get("text") {
                                return (text.clone(), false);
                            }
                        }
                    }
                }
            }
        }
    }
    // Fallback: raw string
    (serde_json::Value::String(json_str.to_string()), false)
}



pub async fn echo_handler(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes
) -> Response {
    // Apply latency simulation (consistent with mock_handler)
    apply_latency(&state.config, &headers).await;
     
    let mut response = Response::new(axum::body::Body::from(body));
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    response
}
