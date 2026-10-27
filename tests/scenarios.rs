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
 * Scenario playback: scripted streams and bodies picked per request, sent
 * event by event with controllable timing and read boundaries (MG-008).
 */

use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use vidaimock::MockServer;

fn write(dir: &std::path::Path, rel: &str, content: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

fn config(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vidaimock-scn-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    write(&dir, "providers/scn.yaml", r#"name: "scn-openai"
matcher: "^/scn/v1/chat/completions$"
priority: 50
response_body: '{"choices":[{"message":{"role":"assistant","content":"normal answer"}}]}'
response_headers:
  x-scenario: "{{ scenario | default(value='none') }}"
scenarios:
  dir: "openai"
  key: "{% if json.model is starting_with('scn-') %}{{ json.model | replace(from='scn-', to='') }}{% endif %}"
stream:
  enabled: true
  frame_format: raw
  lifecycle:
    on_chunk:
      template_body: 'data: {"choices":[{"delta":{"content":{{ chunk | json_encode() }}}}]}'
"#);
    write(&dir, "scenarios/openai/text.sse", r#"data: {"choices":[{"delta":{"role":"assistant"}}],"model":"{{ json.model }}"}

:vidai-delay 200
data: {"choices":[{"delta":{"content":"Hel"}}]}

data: {"choices":[{"delta":{"content":"lo"}}]}
:vidai-split
data: {"choices":[{"delta":{},"finish_reason":"stop"}]}

data: [DONE]
"#);
    write(&dir, "scenarios/openai/text.body", r#"{"id":"chatcmpl-scn","object":"chat.completion","model":"{{ json.model }}","choices":[{"index":0,"message":{"role":"assistant","content":"Hello"},"finish_reason":"stop"}]}"#);
    write(&dir, "scenarios/openai/err-429.status", "429");
    write(&dir, "scenarios/openai/err-429.body", r#"{"error":{"message":"Rate limit reached","type":"requests","code":"rate_limit_exceeded"}}"#);
    dir
}

async fn start(dir: &PathBuf) -> MockServer {
    MockServer::builder().bind("127.0.0.1:0").config_dir(dir).start().await.unwrap()
}

fn chat(model: &str, stream: bool) -> Value {
    json!({"model": model, "stream": stream, "messages": [{"role": "user", "content": "hi"}]})
}

async fn post(s: &MockServer, body: Value, scenario_header: Option<&str>) -> reqwest::Response {
    let mut req = reqwest::Client::new().post(format!("{}/scn/v1/chat/completions", s.base_url())).json(&body);
    if let Some(h) = scenario_header {
        req = req.header("X-Vidai-Scenario", h);
    }
    req.send().await.unwrap()
}

#[tokio::test]
async fn stream_scenario_is_replayed_event_by_event_with_timing() {
    let dir = config("stream");
    let s = start(&dir).await;
    let r = post(&s, chat("scn-text", true), None).await;
    assert_eq!(r.status(), 200);
    assert!(r.headers()["content-type"].to_str().unwrap().starts_with("text/event-stream"));
    assert_eq!(r.headers()["x-scenario"], "text");

    // Collect each network read with its arrival time.
    let t0 = Instant::now();
    let mut reads: Vec<(Duration, String)> = Vec::new();
    let mut r = r;
    while let Some(chunk) = r.chunk().await.unwrap() {
        reads.push((t0.elapsed(), String::from_utf8(chunk.to_vec()).unwrap()));
    }
    let wire: String = reads.iter().map(|(_, s)| s.as_str()).collect();
    assert_eq!(wire,
        "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}],\"model\":\"scn-text\"}\n\n\
         data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n\
         data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\
         data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n\
         data: [DONE]\n\n",
        "directives are stripped, template rendered");

    // 1 + 1 + 2 (split) + 1 writes; they must not be coalesced into fewer reads.
    assert!(reads.len() >= 5, "expected separate reads, got {}: {reads:?}", reads.len());
    let hel = reads.iter().position(|(_, s)| s.contains("Hel")).unwrap();
    assert!(reads[hel].0 - reads[hel - 1].0 >= Duration::from_millis(180), "delay applied: {reads:?}");
    let split_tail = reads.iter().position(|(_, s)| s.starts_with("data: {\"choices\":[{\"delta\":{},")).unwrap();
    assert!(reads[split_tail - 1].1.ends_with("\"lo\"}}]}\n"), "split ends the read mid-event: {reads:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn body_scenario_status_header_and_fallback() {
    let dir = config("body");
    let s = start(&dir).await;

    let v: Value = post(&s, chat("scn-text", false), None).await.json().await.unwrap();
    assert_eq!(v["id"], "chatcmpl-scn");
    assert_eq!(v["model"], "scn-text");

    let r = post(&s, chat("scn-err-429", true), None).await;
    assert_eq!(r.status(), 429, "error scenarios answer with a plain body even for stream requests");
    assert_eq!(r.headers()["content-type"], "application/json");
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["error"]["code"], "rate_limit_exceeded");

    // The header picks a scenario regardless of the model.
    let v: Value = post(&s, chat("gpt-4o", false), Some("text")).await.json().await.unwrap();
    assert_eq!(v["id"], "chatcmpl-scn");

    // No scenario requested -> the provider's normal response.
    let r = post(&s, chat("gpt-4o", false), None).await;
    assert_eq!(r.headers()["x-scenario"], "none");
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["choices"][0]["message"]["content"], "normal answer");

    // Unknown scenario fails loudly and says which one.
    let r = post(&s, chat("scn-nope", false), None).await;
    assert_eq!(r.status(), 500);
    assert!(r.text().await.unwrap().contains("Unknown scenario 'nope'"));
    let _ = std::fs::remove_dir_all(&dir);
}
