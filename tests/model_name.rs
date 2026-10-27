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
 * Responses report the model the client asked for, streaming or not.
 * Gateways price, route and log by this field, so a provider name such as
 * "openai" in its place breaks cost and alias tests.
 */

use serde_json::{json, Value};
use vidaimock::MockServer;

async fn post(s: &MockServer, path: &str, body: Value) -> String {
    reqwest::Client::new().post(format!("{}{}", s.base_url(), path)).json(&body).send().await.unwrap().text().await.unwrap()
}

/// Every model value found on the wire (JSON body or SSE data frames).
fn models(wire: &str) -> Vec<String> {
    let docs: Vec<Value> = match serde_json::from_str::<Value>(wire) {
        Ok(v) => vec![v],
        Err(_) => wire.lines().filter_map(|l| l.strip_prefix("data:")).filter_map(|d| serde_json::from_str(d.trim()).ok()).collect(),
    };
    let mut out = Vec::new();
    for d in docs {
        for ptr in ["/model", "/modelVersion", "/message/model", "/response/model"] {
            if let Some(m) = d.pointer(ptr).and_then(Value::as_str) {
                out.push(m.to_string());
            }
        }
    }
    out
}

fn assert_reports(wire: &str, expected: &str, what: &str) {
    let found = models(wire);
    assert!(!found.is_empty(), "{what}: no model on the wire:\n{wire}");
    assert!(found.iter().all(|m| m == expected), "{what}: expected {expected}, got {found:?}");
}

#[tokio::test]
async fn reports_requested_model_everywhere() {
    let s = MockServer::builder().bind("127.0.0.1:0").start().await.unwrap();
    let msgs = json!([{"role": "user", "content": "hi"}]);
    for stream in [false, true] {
        let tag = if stream { "stream" } else { "non-stream" };
        let w = post(&s, "/v1/chat/completions", json!({"model": "gpt-4o-mini", "messages": msgs, "stream": stream})).await;
        assert_reports(&w, "gpt-4o-mini", &format!("openai {tag}"));
        let w = post(&s, "/v1/chat/completions", json!({"model": "gpt-4o-mini", "messages": msgs, "stream": stream, "response_format": {"type": "json_object"}})).await;
        assert_reports(&w, "gpt-4o-mini", &format!("openai json-mode {tag}"));
        let w = post(&s, "/v1/messages", json!({"model": "claude-sonnet-4-5", "max_tokens": 16, "messages": msgs, "stream": stream})).await;
        assert_reports(&w, "claude-sonnet-4-5", &format!("anthropic {tag}"));
        let w = post(&s, "/v1/responses", json!({"model": "gpt-4.1", "input": "hi", "stream": stream})).await;
        assert_reports(&w, "gpt-4.1", &format!("responses {tag}"));
    }
    let gem = json!({"contents": [{"parts": [{"text": "hi"}]}]});
    assert_reports(&post(&s, "/v1beta/models/gemini-2.0-flash:generateContent", gem.clone()).await, "gemini-2.0-flash", "gemini");
    assert_reports(&post(&s, "/v1beta/models/gemini-2.0-flash:streamGenerateContent?alt=sse", gem).await, "gemini-2.0-flash", "gemini stream");
    assert_reports(&post(&s, "/openai/deployments/prod/chat/completions?api-version=1", json!({"messages": msgs})).await,
        "gpt-4o-2024-08-06", "azure without a model reports the deployment model");
    assert_reports(&post(&s, "/model/anthropic.claude-3-haiku-20240307-v1:0/invoke", json!({"max_tokens": 9, "messages": msgs})).await,
        "anthropic.claude-3-haiku-20240307-v1:0", "bedrock invoke");
}

/// Bodies without `messages` (Bedrock Titan-style, empty JSON) still get a
/// well-formed answer instead of a template error.
#[tokio::test]
async fn bodies_without_messages_still_render() {
    let s = MockServer::builder().bind("127.0.0.1:0").start().await.unwrap();
    for (path, body) in [
        ("/model/amazon.titan-text-express-v1/invoke", json!({"prompt": "Hello"})),
        ("/v1/messages", json!({"max_tokens": 5})),
        ("/v1/chat/completions", json!({})),
        ("/openai/deployments/d/chat/completions?api-version=1", json!({})),
    ] {
        let r = reqwest::Client::new().post(format!("{}{}", s.base_url(), path)).json(&body).send().await.unwrap();
        let status = r.status();
        let text = r.text().await.unwrap();
        assert_eq!(status, 200, "{path}: {text}");
        serde_json::from_str::<Value>(&text).unwrap_or_else(|e| panic!("{path}: {e}: {text}"));
    }
}
