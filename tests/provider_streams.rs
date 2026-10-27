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
 * Wire contracts for the built-in Azure, Vertex and Bedrock streams, as real
 * clients see them. Each of these used to be wrong in a way that made
 * gateway translation tests xfail (VidaiTestSuit MOCK_GAPS MG-005/006/010/011).
 */

use base64::prelude::*;
use serde_json::{json, Value};
use vidaimock::MockServer;

async fn server() -> MockServer {
    MockServer::builder().bind("127.0.0.1:0").start().await.unwrap()
}

async fn post_bytes(server: &MockServer, path: &str, body: Value) -> (reqwest::StatusCode, String, Vec<u8>) {
    let resp = reqwest::Client::new()
        .post(format!("{}{}", server.base_url(), path))
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let ct = resp.headers().get("content-type").map(|v| v.to_str().unwrap().to_string()).unwrap_or_default();
    (status, ct, resp.bytes().await.unwrap().to_vec())
}

/// `data:` payloads of an SSE stream, in order.
fn sse_data(wire: &str) -> Vec<String> {
    wire.lines()
        .filter_map(|l| l.strip_prefix("data:"))
        .map(|d| d.trim().to_string())
        .filter(|d| !d.is_empty())
        .collect()
}

/// Decode AWS event-stream messages into (event-type, payload), checking both CRCs.
fn decode_event_stream(mut buf: &[u8]) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    while !buf.is_empty() {
        let total = u32::from_be_bytes(buf[0..4].try_into().unwrap()) as usize;
        let hlen = u32::from_be_bytes(buf[4..8].try_into().unwrap()) as usize;
        let prelude_crc = u32::from_be_bytes(buf[8..12].try_into().unwrap());
        assert_eq!(crc32fast::hash(&buf[0..8]), prelude_crc, "prelude CRC");
        let msg_crc = u32::from_be_bytes(buf[total - 4..total].try_into().unwrap());
        assert_eq!(crc32fast::hash(&buf[0..total - 4]), msg_crc, "message CRC");

        let mut h = &buf[12..12 + hlen];
        let mut event_type = String::new();
        while !h.is_empty() {
            let n = h[0] as usize;
            let name = std::str::from_utf8(&h[1..1 + n]).unwrap().to_string();
            assert_eq!(h[1 + n], 7, "string header");
            let l = u16::from_be_bytes(h[2 + n..4 + n].try_into().unwrap()) as usize;
            let value = std::str::from_utf8(&h[4 + n..4 + n + l]).unwrap().to_string();
            if name == ":event-type" {
                event_type = value;
            }
            h = &h[4 + n + l..];
        }
        let payload = &buf[12 + hlen..total - 4];
        let json: Value = serde_json::from_slice(payload)
            .unwrap_or_else(|e| panic!("payload not JSON ({e}): {}", String::from_utf8_lossy(payload)));
        out.push((event_type, json));
        buf = &buf[total..];
    }
    out
}

fn claude_body() -> Value {
    json!({
        "anthropic_version": "bedrock-2023-05-31", "max_tokens": 64,
        "messages": [{"role": "user", "content": [{"type": "text", "text": "hi"}]}]
    })
}

/// MG-005: Azure chat streams carry content, a finish_reason chunk, then [DONE].
#[tokio::test]
async fn azure_stream_matches_openai_wire() {
    let s = server().await;
    let (status, ct, body) = post_bytes(&s, "/openai/deployments/gpt4o/chat/completions?api-version=2024-10-21",
        json!({"stream": true, "messages": [{"role": "user", "content": "hello there"}]})).await;
    assert_eq!(status, 200);
    assert!(ct.starts_with("text/event-stream"), "{ct}");
    let wire = String::from_utf8(body).unwrap();
    let data = sse_data(&wire);
    assert_eq!(data.last().map(String::as_str), Some("[DONE]"), "wire:\n{wire}");
    let chunks: Vec<Value> = data[..data.len() - 1].iter()
        .map(|d| serde_json::from_str(d).unwrap_or_else(|e| panic!("{e}: {d}"))).collect();
    let text: String = chunks.iter().filter_map(|c| c["choices"][0]["delta"]["content"].as_str()).collect();
    assert!(!text.trim().is_empty(), "no content streamed, wire:\n{wire}");
    assert!(chunks.iter().any(|c| c["choices"][0]["finish_reason"] == "stop"), "no finish_reason chunk, wire:\n{wire}");
    let ids: std::collections::HashSet<_> = chunks.iter().map(|c| c["id"].clone()).collect();
    assert_eq!(ids.len(), 1, "one id per stream, wire:\n{wire}");
}

/// MG-011: Vertex streams like Gemini — text chunks, a final chunk with
/// finishReason and usageMetadata, and no [DONE].
#[tokio::test]
async fn vertex_stream_matches_gemini_wire() {
    let s = server().await;
    let (status, _, body) = post_bytes(&s,
        "/v1/projects/p/locations/us-central1/publishers/google/models/gemini-2.5-flash:streamGenerateContent?alt=sse",
        json!({"contents": [{"role": "user", "parts": [{"text": "hi"}]}]})).await;
    assert_eq!(status, 200);
    let wire = String::from_utf8(body).unwrap();
    let data = sse_data(&wire);
    assert!(!data.iter().any(|d| d == "[DONE]"), "Vertex never sends [DONE], wire:\n{wire}");
    let chunks: Vec<Value> = data.iter().map(|d| serde_json::from_str(d).unwrap()).collect();
    assert!(chunks.len() >= 3, "expected several chunks, wire:\n{wire}");
    let (last, rest) = chunks.split_last().unwrap();
    assert_eq!(last["candidates"][0]["finishReason"], "STOP");
    assert!(last["usageMetadata"]["totalTokenCount"].is_number());
    for c in rest {
        assert!(c["candidates"][0]["finishReason"].is_null(), "only the last chunk finishes: {c}");
        assert!(c["candidates"][0]["content"]["parts"][0]["text"].is_string());
    }
}

/// MG-010: Bedrock InvokeModel for Claude returns the Anthropic Messages body.
#[tokio::test]
async fn bedrock_invoke_returns_anthropic_messages_body() {
    let s = server().await;
    let (status, _, body) = post_bytes(&s, "/model/anthropic.claude-3-5-sonnet-20240620-v1:0/invoke", claude_body()).await;
    assert_eq!(status, 200);
    let v: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["type"], "message", "{v}");
    assert_eq!(v["role"], "assistant");
    assert_eq!(v["model"], "anthropic.claude-3-5-sonnet-20240620-v1:0");
    assert!(v["content"][0]["text"].is_string(), "{v}");
    assert!(v["stop_reason"].is_string());
    assert!(v["usage"]["input_tokens"].is_number());
}

/// MG-006/MG-014: InvokeModelWithResponseStream sends `chunk` events whose
/// `bytes` decode to the Anthropic stream events, in order.
#[tokio::test]
async fn bedrock_invoke_stream_carries_anthropic_events() {
    let s = server().await;
    let (status, ct, body) = post_bytes(&s, "/model/anthropic.claude-3-haiku-20240307-v1:0/invoke-with-response-stream", claude_body()).await;
    assert_eq!(status, 200);
    assert_eq!(ct, "application/vnd.amazon.eventstream");
    let events = decode_event_stream(&body);
    let inner: Vec<Value> = events.iter().map(|(t, p)| {
        assert_eq!(t, "chunk");
        let b = BASE64_STANDARD.decode(p["bytes"].as_str().expect("bytes field")).unwrap();
        serde_json::from_slice(&b).unwrap()
    }).collect();
    let types: Vec<&str> = inner.iter().map(|e| e["type"].as_str().unwrap()).collect();
    assert_eq!(types.first(), Some(&"message_start"), "{types:?}");
    assert_eq!(types[1], "content_block_start");
    assert!(types.contains(&"content_block_delta"));
    assert_eq!(&types[types.len() - 3..], &["content_block_stop", "message_delta", "message_stop"], "{types:?}");
    let text: String = inner.iter().filter_map(|e| e["delta"]["text"].as_str()).collect();
    assert!(!text.trim().is_empty(), "{types:?}");
}

/// MG-006/MG-014: ConverseStream sends typed events with plain JSON payloads.
#[tokio::test]
async fn bedrock_converse_stream_sends_converse_events() {
    let s = server().await;
    let (status, ct, body) = post_bytes(&s, "/model/anthropic.claude-3-haiku-20240307-v1:0/converse-stream",
        json!({"messages": [{"role": "user", "content": [{"text": "hi"}]}]})).await;
    assert_eq!(status, 200);
    assert_eq!(ct, "application/vnd.amazon.eventstream");
    let events = decode_event_stream(&body);
    let types: Vec<&str> = events.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(types.first(), Some(&"messageStart"), "{types:?}");
    assert!(types[1..types.len() - 3].iter().all(|t| *t == "contentBlockDelta"), "{types:?}");
    assert_eq!(&types[types.len() - 3..], &["contentBlockStop", "messageStop", "metadata"], "{types:?}");
    assert_eq!(events[0].1["role"], "assistant");
    let text: String = events.iter().filter_map(|(_, p)| p["delta"]["text"].as_str()).collect();
    assert_eq!(text.trim(), "This is a simulated Bedrock Converse response.");
    let (_, stop) = &events[events.len() - 2];
    assert_eq!(stop["stopReason"], "end_turn");
    let (_, meta) = events.last().unwrap();
    assert!(meta["usage"]["totalTokens"].is_number(), "{meta}");
}
