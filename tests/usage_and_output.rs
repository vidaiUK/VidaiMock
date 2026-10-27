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
 * Gateway billing tests: X-Vidai-Usage sets exact usage numbers in each
 * provider's native shape; X-Vidai-Output-Tokens scales the output; the
 * request body limit is configurable (MG-009).
 */

use serde_json::{json, Value};
use vidaimock::MockServer;

async fn server() -> MockServer {
    MockServer::builder().bind("127.0.0.1:0").start().await.unwrap()
}

async fn post(s: &MockServer, path: &str, body: Value, headers: &[(&str, &str)]) -> Value {
    let mut req = reqwest::Client::new().post(format!("{}{}", s.base_url(), path)).json(&body);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let r = req.send().await.unwrap();
    let text = r.text().await.unwrap();
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}: {text}"))
}

async fn post_text(s: &MockServer, path: &str, body: Value, headers: &[(&str, &str)]) -> String {
    let mut req = reqwest::Client::new().post(format!("{}{}", s.base_url(), path)).json(&body);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    req.send().await.unwrap().text().await.unwrap()
}

const CACHE_HIT: &str = "input=22740,output=12,cached=22515,reasoning=41";

#[tokio::test]
async fn openai_chat_usage_override() {
    let s = server().await;
    let v = post(&s, "/v1/chat/completions", json!({"model": "gpt-4o", "messages": [{"role": "user", "content": "hi"}]}),
        &[("X-Vidai-Usage", CACHE_HIT)]).await;
    let u = &v["usage"];
    assert_eq!(u["prompt_tokens"], 22740);
    assert_eq!(u["prompt_tokens_details"]["cached_tokens"], 22515);
    // OpenAI counts reasoning inside completion tokens.
    assert_eq!(u["completion_tokens"], 41);
    assert_eq!(u["completion_tokens_details"]["reasoning_tokens"], 41);
    assert_eq!(u["total_tokens"], 22740 + 41);
}

#[tokio::test]
async fn openai_stream_usage_chunk_override() {
    let s = server().await;
    let wire = post_text(&s, "/v1/chat/completions",
        json!({"model": "gpt-4o", "stream": true, "stream_options": {"include_usage": true}, "messages": [{"role": "user", "content": "hi"}]}),
        &[("X-Vidai-Usage", "input=100,output=20,cached=64")]).await;
    let usage = wire.lines().filter_map(|l| l.strip_prefix("data: ")).filter(|d| *d != "[DONE]")
        .map(|d| serde_json::from_str::<Value>(d).unwrap()).find(|e| e["usage"].is_object()).expect("usage chunk");
    assert_eq!(usage["usage"]["prompt_tokens"], 100);
    assert_eq!(usage["usage"]["prompt_tokens_details"]["cached_tokens"], 64);
    assert_eq!(usage["usage"]["total_tokens"], 120);
}

#[tokio::test]
async fn responses_usage_override() {
    let s = server().await;
    let v = post(&s, "/v1/responses", json!({"model": "gpt-4o-mini", "input": "hi"}), &[("X-Vidai-Usage", "input=500,output=30,cached=256,reasoning=10")]).await;
    let u = &v["usage"];
    assert_eq!(u["input_tokens"], 500);
    assert_eq!(u["input_tokens_details"]["cached_tokens"], 256);
    assert_eq!(u["output_tokens"], 30);
    assert_eq!(u["output_tokens_details"]["reasoning_tokens"], 10);
    assert_eq!(u["total_tokens"], 530);
}

#[tokio::test]
async fn anthropic_usage_override_message_and_stream() {
    let s = server().await;
    let body = json!({"model": "claude-sonnet-4-5", "max_tokens": 64, "messages": [{"role": "user", "content": "hi"}]});
    let h = [("X-Vidai-Usage", "input=12,output=40,cached=9000,cache_write=1200")];
    let v = post(&s, "/v1/messages", body.clone(), &h).await;
    let u = &v["usage"];
    assert_eq!(u["input_tokens"], 12, "Anthropic input excludes cache reads/writes");
    assert_eq!(u["cache_read_input_tokens"], 9000);
    assert_eq!(u["cache_creation_input_tokens"], 1200);
    assert_eq!(u["cache_creation"]["ephemeral_5m_input_tokens"], 1200);
    assert_eq!(u["output_tokens"], 40);

    let mut stream = body;
    stream["stream"] = json!(true);
    let wire = post_text(&s, "/v1/messages", stream, &h).await;
    let events: Vec<Value> = wire.lines().filter_map(|l| l.strip_prefix("data: ")).map(|d| serde_json::from_str(d).unwrap()).collect();
    let start = events.iter().find(|e| e["type"] == "message_start").unwrap();
    assert_eq!(start["message"]["usage"]["cache_read_input_tokens"], 9000);
    let delta = events.iter().find(|e| e["type"] == "message_delta").unwrap();
    assert_eq!(delta["usage"]["output_tokens"], 40);
    assert_eq!(delta["usage"]["cache_creation_input_tokens"], 1200);
}

#[tokio::test]
async fn gemini_usage_override_with_cache_and_thinking() {
    let s = server().await;
    let v = post(&s, "/v1beta/models/gemini-2.5-flash-lite:generateContent",
        json!({"contents": [{"role": "user", "parts": [{"text": "hi"}]}]}), &[("X-Vidai-Usage", CACHE_HIT)]).await;
    let u = &v["usageMetadata"];
    assert_eq!(u["promptTokenCount"], 22740);
    assert_eq!(u["candidatesTokenCount"], 12);
    assert_eq!(u["cachedContentTokenCount"], 22515);
    assert_eq!(u["cacheTokensDetails"][0]["tokenCount"], 22515);
    assert_eq!(u["thoughtsTokenCount"], 41);
    assert_eq!(u["totalTokenCount"], 22793, "Gemini total adds thoughts");

    // Without the header the defaults are unchanged.
    let v = post(&s, "/v1beta/models/gemini-2.5-flash:generateContent", json!({"contents": [{"role": "user", "parts": [{"text": "hi"}]}]}), &[]).await;
    assert_eq!(v["usageMetadata"]["totalTokenCount"], 40);
    assert!(v["usageMetadata"].get("cachedContentTokenCount").is_none());
}

#[tokio::test]
async fn output_tokens_header_scales_text_and_usage() {
    let s = server().await;
    let v = post(&s, "/v1/chat/completions", json!({"model": "gpt-4o", "messages": [{"role": "user", "content": "hi"}], "max_tokens": 300}),
        &[("X-Vidai-Output-Tokens", "max")]).await;
    let text = v["choices"][0]["message"]["content"].as_str().unwrap();
    assert_eq!(text.split_whitespace().count(), 300);
    assert_eq!(v["usage"]["completion_tokens"], 300);

    let wire = post_text(&s, "/v1/messages",
        json!({"model": "claude-haiku-4-5", "max_tokens": 1000, "stream": true, "messages": [{"role": "user", "content": "hi"}]}),
        &[("X-Vidai-Output-Tokens", "50"), ("X-Vidai-Chaos-Trickle", "1")]).await;
    let deltas = wire.matches("\"text_delta\"").count();
    assert_eq!(deltas, 50, "one delta per word");

    let v = post(&s, "/v1beta/models/gemini-2.5-flash:generateContent",
        json!({"contents": [{"role": "user", "parts": [{"text": "hi"}]}], "generationConfig": {"maxOutputTokens": 25}}),
        &[("X-Vidai-Output-Tokens", "max")]).await;
    assert_eq!(v["candidates"][0]["content"]["parts"][0]["text"].as_str().unwrap().split_whitespace().count(), 25);
    assert_eq!(v["usageMetadata"]["candidatesTokenCount"], 25);
}

/// Azure/Mistral echo the prompt; quotes in it must not break the JSON.
#[tokio::test]
async fn echoed_prompt_is_escaped() {
    let s = server().await;
    let v = post(&s, "/openai/deployments/d/chat/completions?api-version=1", json!({"messages": [{"role": "user", "content": "say \"hi\""}]}), &[]).await;
    assert_eq!(v["choices"][0]["message"]["content"], "say \"hi\"");
}

/// MG-009: the body limit is configurable; the default takes multi-MB bodies.
#[tokio::test]
async fn body_limit_default_and_configured() {
    let big = "x".repeat(5 * 1024 * 1024);
    let body = json!({"model": "gpt-4o", "messages": [{"role": "user", "content": big}]});

    let s = server().await;
    let r = reqwest::Client::new().post(format!("{}/v1/chat/completions", s.base_url())).json(&body).send().await.unwrap();
    assert_eq!(r.status(), 200, "5 MiB fits the 32 MiB default");

    let small = MockServer::builder().bind("127.0.0.1:0").max_body_mb(1).start().await.unwrap();
    // The server answers 413 and closes; a client still uploading may see the
    // reset before the status. Either way the request must not succeed.
    match reqwest::Client::new().post(format!("{}/v1/chat/completions", small.base_url())).json(&body).send().await {
        Ok(r) => assert_eq!(r.status(), 413),
        Err(e) => assert!(!e.is_status() && !e.is_decode(), "unexpected error: {e}"),
    }
}
