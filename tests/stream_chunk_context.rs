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
 * Streaming lifecycle templates get `chunk_index` (zero-based) and
 * `chunk_count` alongside `chunk` (issue #11). Templates use these to render
 * monotonic protocol sequence numbers even when chunk text repeats.
 */

use std::path::PathBuf;
use vidaimock::MockServer;

/// Write a one-provider config dir whose stream is raw-framed and whose
/// lifecycle templates print the chunk metadata.
fn write_config(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "vidaimock-chunk-ctx-{}-{}",
        name,
        std::process::id()
    ));
    let providers = dir.join("providers");
    std::fs::create_dir_all(&providers).unwrap();
    std::fs::write(
        providers.join("chunk_ctx.yaml"),
        r#"name: "chunk-ctx"
matcher: "^/v1/chunk-ctx"
response_body: '{"choices":[{"message":{"role":"assistant","content":"same same"}}]}'
stream:
  enabled: true
  frame_format: raw
  lifecycle:
    on_start:
      template_body: "event: start\ndata: count={{ chunk_count }}"
    on_chunk:
      template_body: "event: chunk\ndata: index={{ chunk_index }} count={{ chunk_count }} chunk={{ chunk }}|"
    on_stop:
      template_body: "event: stop\ndata: count={{ chunk_count }}"
"#,
    )
    .unwrap();
    dir
}

#[tokio::test]
async fn on_chunk_receives_index_and_count() {
    let dir = write_config("basic");
    let server = MockServer::builder()
        .bind("127.0.0.1:0")
        .config_dir(&dir)
        .isolated(true)
        .start()
        .await
        .expect("server should start");

    let body = reqwest::Client::new()
        .post(format!("{}/v1/chunk-ctx", server.base_url()))
        .json(&serde_json::json!({"stream": true}))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    let chunk_frames: Vec<&str> = body
        .split("\n\n")
        .filter(|f| f.starts_with("event: chunk"))
        .collect();

    assert_eq!(chunk_frames.len(), 2, "expected two chunk frames, wire:\n{body}");
    assert!(chunk_frames[0].contains("data: index=0 count=2 chunk=same |"), "frame 0: {}", chunk_frames[0]);
    assert!(chunk_frames[1].contains("data: index=1 count=2 chunk=same |"), "frame 1: {}", chunk_frames[1]);

    // The count is known before the first chunk, so start/stop can use it too.
    assert!(body.contains("event: start\ndata: count=2"), "start frame, wire:\n{body}");
    assert!(body.contains("event: stop\ndata: count=2"), "stop frame, wire:\n{body}");

    server.shutdown().await.unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Pull every `sequence_number` off a Responses SSE wire, in order.
fn sequence_numbers(wire: &str) -> Vec<u64> {
    wire.lines()
        .filter_map(|l| l.strip_prefix("data: "))
        .filter_map(|d| serde_json::from_str::<serde_json::Value>(d).ok())
        .filter_map(|v| v["sequence_number"].as_u64())
        .collect()
}

async fn responses_stream(body: serde_json::Value) -> String {
    let server = MockServer::builder().bind("127.0.0.1:0").start().await.unwrap();
    let wire = reqwest::Client::new()
        .post(format!("{}/v1/responses", server.base_url()))
        .json(&body)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    server.shutdown().await.unwrap();
    wire
}

/// The bundled Responses stream numbers events 0,1,2,... with no repeats,
/// however many text deltas the reply splits into.
#[tokio::test]
async fn bundled_responses_text_stream_sequence_is_contiguous() {
    let wire = responses_stream(serde_json::json!({
        "model": "gpt-4o-mini", "input": "hello", "stream": true
    }))
    .await;
    let seq = sequence_numbers(&wire);
    let deltas = wire.matches("event: response.output_text.delta").count();
    assert!(deltas >= 2, "need a multi-chunk reply to test this, wire:\n{wire}");
    // 4 start events + one per delta + 4 stop events
    assert_eq!(seq, (0..(8 + deltas) as u64).collect::<Vec<_>>(), "wire:\n{wire}");
}

#[tokio::test]
async fn bundled_responses_tool_stream_sequence_is_contiguous() {
    let wire = responses_stream(serde_json::json!({
        "model": "gpt-4o-mini", "input": "weather?", "stream": true,
        "tools": [{"type": "function", "name": "get_weather", "parameters": {"type": "object"}}]
    }))
    .await;
    // 3 start events + 1 arguments delta + 3 stop events
    assert_eq!(sequence_numbers(&wire), (0..7).collect::<Vec<_>>(), "wire:\n{wire}");
}

/// Every closing event repeats exactly the text the deltas streamed, so a
/// client that checks accumulated deltas against `output_text.done` agrees.
#[tokio::test]
async fn bundled_responses_done_text_matches_streamed_deltas() {
    let wire = responses_stream(serde_json::json!({
        "model": "gpt-4o-mini", "input": "hello", "stream": true
    }))
    .await;
    let events: Vec<serde_json::Value> = wire
        .lines()
        .filter_map(|l| l.strip_prefix("data: "))
        .filter_map(|d| serde_json::from_str(d).ok())
        .collect();
    let of = |t: &'static str| events.iter().filter(move |e| e["type"] == t);

    let streamed: String = of("response.output_text.delta")
        .map(|e| e["delta"].as_str().unwrap().to_string())
        .collect();
    assert!(!streamed.is_empty(), "wire:\n{wire}");

    let done = of("response.output_text.done").next().expect("output_text.done");
    assert_eq!(done["text"], streamed.as_str());
    let part = of("response.content_part.done").next().expect("content_part.done");
    assert_eq!(part["part"]["text"], streamed.as_str());
    let item = of("response.output_item.done").next().expect("output_item.done");
    assert_eq!(item["item"]["content"][0]["text"], streamed.as_str());
    let completed = of("response.completed").next().expect("response.completed");
    assert_eq!(completed["response"]["output"][0]["content"][0]["text"], streamed.as_str());
}
