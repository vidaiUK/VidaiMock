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
 * Scenario playback: replay a scripted response picked per request.
 *
 * A provider opts in with `scenarios: { dir, key }`. The scenario id comes from
 * the `X-Vidai-Scenario` header, or from rendering `key` (e.g. the model name).
 * Files live in `<config-dir>/scenarios/<dir>/` and are Tera templates:
 *
 *   <id>.sse     full stream, sent event by event (blank-line separated)
 *   <id>.body    non-streaming body (also `.json`)
 *   <id>.status  optional status code
 *
 * Each stream event is its own network write. Two SSE comment directives,
 * stripped before sending, control timing and read boundaries:
 *
 *   :vidai-delay <ms>   wait this long before sending the event
 *   :vidai-split        end the current write here, mid-event
 */

use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use std::convert::Infallible;
use std::time::Duration;

use crate::handlers::{apply_response_headers, render_path, AppState};
use crate::provider::{ProviderConfig, ProviderRegistry};

pub(crate) const HEADER: &str = "x-vidai-scenario";

/// Template names that may hold each part of a scenario, in lookup order.
const STREAM_EXT: &[&str] = &["sse", "sse.j2"];
const BODY_EXT: &[&str] = &["body", "body.j2", "json", "json.j2"];
const STATUS_EXT: &[&str] = &["status", "status.j2"];

/// The scenario id this request asks for, if the provider uses scenarios.
pub(crate) fn requested_id(
    provider: &ProviderConfig,
    registry: &ProviderRegistry,
    headers: &HeaderMap,
    ctx: &tera::Context,
) -> Option<String> {
    let cfg = provider.scenarios.as_ref()?;
    if let Some(id) = headers.get(HEADER).and_then(|v| v.to_str().ok()) {
        let id = id.trim();
        if !id.is_empty() {
            return Some(id.to_string());
        }
    }
    let key = cfg.key.as_deref()?;
    let id = crate::handlers::render_inline(registry, key, ctx);
    let id = id.trim();
    (!id.is_empty()).then(|| id.to_string())
}

fn find(registry: &ProviderRegistry, dir: &str, id: &str, exts: &[&str]) -> Option<String> {
    exts.iter()
        .map(|ext| format!("scenarios/{dir}/{id}.{ext}"))
        .find(|name| registry.tera.templates.contains_key(name))
}

/// Answer the request from scenario `id`.
pub(crate) async fn respond(
    state: &AppState,
    provider: &ProviderConfig,
    ctx: &mut tera::Context,
    id: &str,
    streaming: bool,
    headers: &HeaderMap,
) -> Response {
    let registry = &state.registry;
    let dir = provider.scenarios.as_ref().map(|c| c.dir.as_str()).unwrap_or_default();
    let stream_tpl = find(registry, dir, id, STREAM_EXT);
    let body_tpl = find(registry, dir, id, BODY_EXT);

    if stream_tpl.is_none() && body_tpl.is_none() {
        // A mis-wired test should fail loudly, not get a default answer.
        let msg = format!("Unknown scenario '{id}' for provider '{}' (looked in scenarios/{dir}/)", provider.name);
        tracing::warn!("{msg}");
        let body = serde_json::json!({"error": {"message": msg, "type": "vidaimock_scenario_not_found"}});
        return json_response(StatusCode::INTERNAL_SERVER_ERROR, body.to_string());
    }

    let status = find(registry, dir, id, STATUS_EXT)
        .map(|t| render_path(registry, &t, ctx))
        .and_then(|s| s.trim().parse::<u16>().ok())
        .and_then(|c| StatusCode::from_u16(c).ok())
        .unwrap_or(StatusCode::OK);
    ctx.insert("status_code", &status.as_u16());
    ctx.insert("scenario", id);

    let is_error = status.is_client_error() || status.is_server_error();
    let mut resp = match (streaming && !is_error, &stream_tpl, &body_tpl) {
        (true, Some(tpl), _) => stream_response(state, provider, render_path(registry, tpl, ctx), headers),
        (_, _, Some(tpl)) => json_response(status, render_path(registry, tpl, ctx)),
        // Error status with only a stream file: send it as a plain body.
        (_, Some(tpl), None) => json_response(status, render_path(registry, tpl, ctx)),
        (_, None, None) => unreachable!(),
    };
    *resp.status_mut() = status;
    apply_response_headers(resp.headers_mut(), provider, registry, ctx);
    resp
}

fn json_response(status: StatusCode, body: String) -> Response {
    let mut resp = Response::new(axum::body::Body::from(body));
    *resp.status_mut() = status;
    resp.headers_mut().insert(axum::http::header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    resp
}

/// One network write: bytes to send, after waiting `delay`.
#[derive(Debug, PartialEq)]
pub(crate) struct Write {
    pub delay: Option<Duration>,
    pub bytes: Vec<u8>,
}

/// Split a rendered SSE script into writes: one per event, plus extra breaks
/// at `:vidai-split`. `:vidai-delay` lines set the wait before their event.
pub(crate) fn sse_writes(script: &str) -> Vec<Write> {
    let mut writes = Vec::new();
    for event in split_events(script) {
        let mut delay = None;
        let mut current = String::new();
        for line in event.lines() {
            let trimmed = line.trim();
            if let Some(ms) = trimmed.strip_prefix(":vidai-delay") {
                delay = ms.trim().parse::<u64>().ok().map(Duration::from_millis);
            } else if trimmed == ":vidai-split" {
                if !current.is_empty() {
                    writes.push(Write { delay: delay.take(), bytes: std::mem::take(&mut current).into_bytes() });
                }
            } else {
                current.push_str(trimmed);
                current.push('\n');
            }
        }
        current.push('\n'); // terminate the event
        writes.push(Write { delay, bytes: current.into_bytes() });
    }
    writes
}

/// Blank-line separated events, ignoring empty ones.
fn split_events(script: &str) -> Vec<String> {
    let mut events = Vec::new();
    let mut current = String::new();
    for line in script.lines() {
        if line.trim().is_empty() {
            if !current.trim().is_empty() {
                events.push(std::mem::take(&mut current));
            }
            current.clear();
        } else {
            current.push_str(line);
            current.push('\n');
        }
    }
    if !current.trim().is_empty() {
        events.push(current);
    }
    events
}

fn stream_response(state: &AppState, provider: &ProviderConfig, script: String, headers: &HeaderMap) -> Response {
    let aws = provider.stream.as_ref().and_then(|s| s.encoding.as_deref()) == Some("aws-event-stream");
    let writes: Vec<Write> = if aws {
        let mode = crate::aws_event_stream::AwsPayload::from_config(
            provider.stream.as_ref().and_then(|s| s.aws_payload.as_deref()));
        split_events(&script)
            .into_iter()
            .map(|event| Write {
                delay: sse_writes(&event).first().and_then(|w| w.delay),
                bytes: crate::aws_event_stream::AwsEventStreamEncoder::encode_template_output(&event, mode),
            })
            .collect()
    } else {
        sse_writes(&script)
    };

    // Gap between writes when the script sets none: X-Vidai-Chaos-Trickle,
    // else the configured trickle, else none.
    let trickle = headers
        .get("x-vidai-chaos-trickle")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(state.config.chaos.trickle_ms);
    let default_gap = (trickle > 0).then(|| Duration::from_millis(trickle));

    let stream = futures::stream::unfold(writes.into_iter().enumerate(), move |mut it| async move {
        let (i, w) = it.next()?;
        if let Some(d) = w.delay.or(if i > 0 { default_gap } else { None }) {
            tokio::time::sleep(d).await;
        }
        Some((Ok::<_, Infallible>(axum::body::Bytes::from(w.bytes)), it))
    });
    let mut resp = Response::new(axum::body::Body::from_stream(stream));
    let ct = if aws { "application/vnd.amazon.eventstream" } else { "text/event-stream" };
    resp.headers_mut().insert(axum::http::header::CONTENT_TYPE, HeaderValue::from_static(ct));
    resp.headers_mut().insert(axum::http::header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_write_per_event_with_delay_and_split() {
        let script = "event: a\ndata: {\"x\":1}\n\n:vidai-delay 250\nevent: b\ndata: {\"y\":\n:vidai-split\ndata: 2}\n\n\n";
        let w = sse_writes(script);
        assert_eq!(w.len(), 3);
        assert_eq!(w[0], Write { delay: None, bytes: b"event: a\ndata: {\"x\":1}\n\n".to_vec() });
        assert_eq!(w[1], Write { delay: Some(Duration::from_millis(250)), bytes: b"event: b\ndata: {\"y\":\n".to_vec() });
        assert_eq!(w[2], Write { delay: None, bytes: b"data: 2}\n\n".to_vec() });
    }
}
