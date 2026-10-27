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
 * Provider `response_headers` (MG-001): gateways need upstream headers such
 * as Retry-After, rate-limit counters and request ids to test passthrough.
 */

use serde_json::json;
use vidaimock::MockServer;

async fn server() -> MockServer {
    MockServer::builder().bind("127.0.0.1:0").start().await.unwrap()
}

async fn post(server: &MockServer, path: &str, body: serde_json::Value) -> reqwest::Response {
    reqwest::Client::new()
        .post(format!("{}{}", server.base_url(), path))
        .json(&body)
        .send()
        .await
        .unwrap()
}

fn chat() -> serde_json::Value {
    json!({"model": "gpt-4o", "messages": [{"role": "user", "content": "hi"}]})
}

#[tokio::test]
async fn openai_success_carries_request_id_and_rate_limits_but_no_retry_after() {
    let s = server().await;
    let r = post(&s, "/v1/chat/completions", chat()).await;
    assert_eq!(r.status(), 200);
    let h = r.headers();
    assert!(h["x-request-id"].to_str().unwrap().starts_with("req_"));
    assert_eq!(h["x-ratelimit-remaining-requests"], "9999");
    assert!(h.get("retry-after").is_none(), "no Retry-After on success");
    assert_eq!(h["content-type"], "application/json");
}

#[tokio::test]
async fn openai_429_carries_retry_after() {
    let s = server().await;
    let r = post(&s, "/v1/chat/completions?chaos_status=429", chat()).await;
    assert_eq!(r.status(), 429);
    assert_eq!(r.headers()["retry-after"], "2");
    assert_eq!(r.headers()["x-ratelimit-remaining-requests"], "0");
}

#[tokio::test]
async fn openai_stream_carries_headers() {
    let s = server().await;
    let mut body = chat();
    body["stream"] = json!(true);
    let r = post(&s, "/v1/chat/completions", body).await;
    assert_eq!(r.status(), 200);
    assert!(r.headers()["content-type"].to_str().unwrap().starts_with("text/event-stream"));
    assert!(r.headers().contains_key("x-request-id"));
    assert!(r.headers().get("retry-after").is_none());
}

#[tokio::test]
async fn openai_stream_error_carries_retry_after() {
    let s = server().await;
    let mut body = chat();
    body["stream"] = json!(true);
    let r = post(&s, "/v1/chat/completions?chaos_status=503", body).await;
    assert_eq!(r.status(), 503);
    assert_eq!(r.headers()["retry-after"], "2");
}

#[tokio::test]
async fn anthropic_carries_request_id_and_retry_after_on_429() {
    let s = server().await;
    let body = json!({"model": "claude-haiku-4-5", "max_tokens": 16, "messages": [{"role": "user", "content": "hi"}]});
    let ok = post(&s, "/v1/messages", body.clone()).await;
    assert!(ok.headers()["request-id"].to_str().unwrap().starts_with("req_"));
    assert!(ok.headers().get("retry-after").is_none());
    let limited = post(&s, "/v1/messages?chaos_status=429", body).await;
    assert_eq!(limited.status(), 429);
    assert_eq!(limited.headers()["retry-after"], "2");
}

/// A custom provider can set any header, including overriding Content-Type.
#[tokio::test]
async fn custom_provider_headers_and_content_type_override() {
    let dir = std::env::temp_dir().join(format!("vidaimock-headers-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("providers")).unwrap();
    std::fs::write(dir.join("providers/h.yaml"), r#"name: "h"
matcher: "^/h$"
response_body: "<ok/>"
response_headers:
  content-type: "application/xml"
  x-echo-model: "{{ json.model }}"
  x-empty: "{% if false %}never{% endif %}"
"#).unwrap();
    let s = MockServer::builder().bind("127.0.0.1:0").config_dir(&dir).isolated(true).start().await.unwrap();
    let r = post(&s, "/h", json!({"model": "m1"})).await;
    assert_eq!(r.headers()["content-type"], "application/xml");
    assert_eq!(r.headers()["x-echo-model"], "m1");
    assert!(r.headers().get("x-empty").is_none());
    let _ = std::fs::remove_dir_all(&dir);
}
