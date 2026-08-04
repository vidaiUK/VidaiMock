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
 * Gateway / soak-tester contract.
 *
 * VidaiMock is used as the hermetic backend for long-running soak tests of LLM
 * gateways — sustained closed-loop traffic through a router, with no live
 * provider credentials or cost. A public example is NVIDIA NeMo Switchyard's
 * release soak rehearsal:
 *
 *   NVIDIA-NeMo/Switchyard#176, scripts/soak-rehearsal.sh
 *   vidaimock --port 8100 --mode realistic --latency 40
 *
 * That workload depends on behaviour that no other test pins down, and which
 * is easy to break without noticing:
 *
 *   - `--mode realistic --latency N` actually delays responses by ~N ms.
 *     They tune this to mimic provider latency, AND deliberately raise it past
 *     their request timeout to prove their soak gate fails closed. Latency
 *     that silently stopped applying would turn a fault-injection rehearsal
 *     into a false pass.
 *   - /health stays available under sustained concurrent load. Their runner
 *     polls it every interval and fails the run if a liveness check misses.
 *   - The server survives thousands of sequential requests without degrading
 *     — the entire point of a soak test.
 *   - Response shapes stay stable across a long run, since every response body
 *     is validated, not just the status code.
 *
 * These tests are deliberately about the *sustained* contract rather than
 * single-request correctness, which the VM-### suites already cover.
 */

use std::time::{Duration, Instant};
use vidaimock::MockServer;

/// Boot a server with a fixed latency, the way a soak rehearsal configures it.
///
/// Equivalent to `--mode realistic --latency N` on the CLI.
async fn server_with_latency(ms: u64) -> MockServer {
    MockServer::builder()
        .bind("127.0.0.1:0")
        .mode("realistic")
        .latency_ms(ms)
        .start()
        .await
        .expect("server should start")
}

fn chat_body() -> serde_json::Value {
    serde_json::json!({
        "model": "gpt-4",
        "messages": [{"role": "user", "content": "ping"}],
        "max_tokens": 8
    })
}

/// Sustained closed-loop traffic must not degrade or drop requests.
///
/// A soak run keeps N requests in flight for hours and fails on a single
/// error. This is the same shape, compressed.
#[tokio::test]
async fn sustained_concurrent_load_has_no_errors() {
    let server = MockServer::builder().bind("127.0.0.1:0").start().await.unwrap();
    let url = format!("{}/v1/chat/completions", server.base_url());
    let client = reqwest::Client::new();

    const CONCURRENCY: usize = 8;
    const ROUNDS: usize = 25; // 200 requests total

    for round in 0..ROUNDS {
        let mut handles = Vec::with_capacity(CONCURRENCY);
        for _ in 0..CONCURRENCY {
            let c = client.clone();
            let u = url.clone();
            handles.push(tokio::spawn(async move {
                let r = c.post(&u).json(&chat_body()).send().await?;
                let status = r.status();
                let j: serde_json::Value = r.json().await?;
                Ok::<_, reqwest::Error>((status, j))
            }));
        }
        for h in handles {
            let (status, json) = h.await.expect("task panicked").expect("request failed");
            assert_eq!(status, 200, "round {}: every request must succeed", round);
            // The runner validates bodies, not just status codes.
            assert_eq!(json["object"], "chat.completion",
                "round {}: response shape must stay stable", round);
            assert!(json["choices"][0]["message"]["content"].is_string(),
                "round {}: completion must carry content", round);
        }
    }

    server.shutdown().await.unwrap();
}

/// /health must stay responsive while inference traffic is in flight.
///
/// The soak runner polls /health on a timer and fails the run on a single
/// missed liveness check, so health must not be starved by request load.
#[tokio::test]
async fn health_stays_live_under_load() {
    let server = MockServer::builder().bind("127.0.0.1:0").start().await.unwrap();
    let chat_url = format!("{}/v1/chat/completions", server.base_url());
    let health_url = format!("{}/health", server.base_url());
    let client = reqwest::Client::new();

    // Background load.
    let load = {
        let c = client.clone();
        let u = chat_url.clone();
        tokio::spawn(async move {
            for _ in 0..60 {
                let _ = c.post(&u).json(&chat_body()).send().await;
            }
        })
    };

    // Poll health throughout, as the reporter does.
    for i in 0..10 {
        let r = client
            .get(&health_url)
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .unwrap_or_else(|e| panic!("health poll {} failed while under load: {}", i, e));
        assert_eq!(r.status(), 200, "health poll {} must return 200", i);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    load.await.unwrap();
    server.shutdown().await.unwrap();
}

/// Configured latency must actually delay responses.
///
/// Soak rehearsals set this to mimic provider latency and, critically, raise it
/// past the client's request timeout to prove their failure gate trips. If
/// latency silently stopped applying, that fault-injection rehearsal would
/// pass when it should fail — a false green on a release gate.
#[tokio::test]
async fn configured_latency_is_actually_applied() {
    const LATENCY_MS: u64 = 300;
    let server = server_with_latency(LATENCY_MS).await;
    let url = format!("{}/v1/chat/completions", server.base_url());

    let start = Instant::now();
    let r = reqwest::Client::new().post(&url).json(&chat_body()).send().await.unwrap();
    let elapsed = start.elapsed();

    assert_eq!(r.status(), 200);
    // Allow generous slack for scheduling; the point is that the delay exists
    // and is in the right ballpark, not that it is precise.
    assert!(elapsed >= Duration::from_millis(LATENCY_MS - 50),
        "configured latency of {}ms must delay the response; took {:?}",
        LATENCY_MS, elapsed);
    assert!(elapsed < Duration::from_millis(LATENCY_MS * 6),
        "latency should be roughly as configured, not wildly larger; took {:?}", elapsed);

    server.shutdown().await.unwrap();
}

/// The per-request latency header override must work too.
///
/// This is the finer-grained fault-injection knob: it lets a harness slow a
/// single request past a timeout without restarting the server.
#[tokio::test]
async fn latency_header_override_is_applied() {
    let server = MockServer::builder().bind("127.0.0.1:0").start().await.unwrap();
    let url = format!("{}/v1/chat/completions", server.base_url());

    let start = Instant::now();
    let r = reqwest::Client::new()
        .post(&url)
        .header("x-vidai-latency", "300")
        .json(&chat_body())
        .send()
        .await
        .unwrap();
    let elapsed = start.elapsed();

    assert_eq!(r.status(), 200);
    assert!(elapsed >= Duration::from_millis(250),
        "x-vidai-latency must delay this request; took {:?}", elapsed);

    server.shutdown().await.unwrap();
}

/// A gateway translates between provider formats, so a soak run drives several
/// endpoints in one pass. All must stay healthy together.
#[tokio::test]
async fn all_soak_endpoints_respond_correctly() {
    let server = MockServer::builder().bind("127.0.0.1:0").start().await.unwrap();
    let client = reqwest::Client::new();
    let base = server.base_url();

    // OpenAI Chat Completions
    let r: serde_json::Value = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&chat_body())
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(r["object"], "chat.completion");

    // Anthropic Messages
    let r: serde_json::Value = client
        .post(format!("{base}/v1/messages"))
        .json(&serde_json::json!({
            "model": "claude-haiku-4-5",
            "max_tokens": 64,
            "messages": [{"role": "user", "content": "ping"}]
        }))
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(r["type"], "message");

    // OpenAI Responses
    let r: serde_json::Value = client
        .post(format!("{base}/v1/responses"))
        .json(&serde_json::json!({
            "model": "gpt-4o-mini",
            "input": [{"role": "user", "content": "ping"}]
        }))
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(r["object"], "response");

    server.shutdown().await.unwrap();
}

/// Streaming must frame correctly under a mixed streaming/non-streaming run
/// (soak rehearsals use `--stream-ratio 0.5`), since SSE framing is checked
/// per response.
#[tokio::test]
async fn streaming_framing_holds_across_repeated_requests() {
    let server = MockServer::builder().bind("127.0.0.1:0").start().await.unwrap();
    let url = format!("{}/v1/chat/completions", server.base_url());
    let client = reqwest::Client::new();

    for i in 0..15 {
        let body = client
            .post(&url)
            .json(&serde_json::json!({
                "model": "gpt-4",
                "stream": true,
                "messages": [{"role": "user", "content": "ping"}]
            }))
            .send().await.unwrap()
            .text().await.unwrap();

        assert!(body.contains("data: "), "iteration {}: SSE frames expected", i);
        assert!(body.contains("[DONE]"), "iteration {}: stream must terminate with [DONE]", i);
        assert!(!body.contains("[[object]]"),
            "iteration {}: no stringified structured chunk may leak into the stream", i);
    }

    server.shutdown().await.unwrap();
}

/// A long run must not leave the server wedged: after sustained traffic it
/// still answers a fresh request correctly.
#[tokio::test]
async fn server_remains_correct_after_sustained_traffic() {
    let server = MockServer::builder().bind("127.0.0.1:0").start().await.unwrap();
    let url = format!("{}/v1/chat/completions", server.base_url());
    let client = reqwest::Client::new();

    for _ in 0..150 {
        let r = client.post(&url).json(&chat_body()).send().await.unwrap();
        assert_eq!(r.status(), 200);
    }

    // The response after the load must be as correct as the first one.
    let j: serde_json::Value = client
        .post(&url).json(&chat_body()).send().await.unwrap()
        .json().await.unwrap();
    assert_eq!(j["object"], "chat.completion");
    assert!(j["choices"][0]["message"]["content"].is_string());
    assert_eq!(j["choices"][0]["finish_reason"], "stop");

    server.shutdown().await.unwrap();
}
