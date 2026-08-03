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
 * Integration tests for the public library API (issue #9, Phase 1).
 *
 * These live in tests/ rather than in a #[cfg(test)] module on purpose: an
 * integration test crate can only reach `pub` items, so this file exercises
 * VidaiMock through exactly the surface an external consumer sees. If
 * something here needs a private item, the API is wrong.
 *
 * Test IDs (T-nn) refer to the Phase 1 design document.
 */

use std::time::Duration;
use vidaimock::MockServer;

/// Issues a GET and returns (status, body).
async fn get(url: &str) -> (u16, String) {
    let resp = reqwest::get(url).await.expect("request failed");
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    (status, body)
}

/// T-02: binding to port 0 must pick a real ephemeral port.
#[tokio::test]
async fn t02_starts_on_ephemeral_port() {
    let server = MockServer::builder()
        .bind("127.0.0.1:0")
        .start()
        .await
        .expect("server should start");

    let port = server.addr().port();
    assert_ne!(port, 0, "an ephemeral bind must resolve to a real port");
    assert!(server.base_url().starts_with("http://127.0.0.1:"),
        "base_url should reflect the bound address, got {}", server.base_url());

    server.shutdown().await.expect("shutdown should succeed");
}

/// T-03: the advertised base_url must actually be reachable.
#[tokio::test]
async fn t03_base_url_is_reachable() {
    let server = MockServer::builder().bind("127.0.0.1:0").start().await.unwrap();

    let (status, body) = get(&format!("{}/health", server.base_url())).await;
    assert_eq!(status, 200, "health check should return 200");
    assert!(body.contains("ok"), "health body should report ok, got: {}", body);

    server.shutdown().await.unwrap();
}

/// T-04: the library must serve real provider behaviour, not an empty shell.
/// This proves the bundled providers and embedded templates load in-process.
#[tokio::test]
async fn t04_serves_bundled_provider_behaviour() {
    let server = MockServer::builder().bind("127.0.0.1:0").start().await.unwrap();

    let resp = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", server.base_url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .send()
        .await
        .expect("request failed");

    assert_eq!(resp.status().as_u16(), 200);
    let json: serde_json::Value = resp.json().await.expect("response should be JSON");
    assert_eq!(json["object"], "chat.completion",
        "should return a real OpenAI-shaped completion, got:\n{}", json);
    assert!(json["choices"][0]["message"]["content"].is_string(),
        "completion should carry message content, got:\n{}", json);

    server.shutdown().await.unwrap();
}

/// T-05: a bind failure must return Err — NOT kill the process.
///
/// This is the regression guard for the one behavioural change in Phase 1
/// (server.rs previously called std::process::exit(1) here). If that exit
/// survives, this test binary dies and the whole suite reports a failure
/// rather than an assertion error — which is itself the signal.
#[tokio::test]
async fn t05_bind_failure_returns_error_and_does_not_exit() {
    // Occupy a port, then try to bind the same one.
    let occupied = MockServer::builder().bind("127.0.0.1:0").start().await.unwrap();
    let addr = occupied.addr();

    let result = MockServer::builder()
        .bind(addr.to_string())
        .start()
        .await;

    // Assert the specific variant, not merely "some error" — a port clash must
    // be reported as Bind so the message can point at the real cause.
    match result {
        Err(vidaimock::Error::Bind { .. }) => {}
        Err(other) => panic!("expected Error::Bind for an occupied port, got: {:?}", other),
        Ok(_) => panic!("binding an occupied port must fail"),
    }

    // The critical assertion: we are still running. Reaching this line at all
    // proves the library did not call process::exit.
    assert_eq!(occupied.addr(), addr, "original server should be unaffected");

    occupied.shutdown().await.unwrap();
}

/// T-06: multiple instances must coexist — the property that makes parallel
/// test suites possible.
#[tokio::test]
async fn t06_parallel_instances_are_isolated() {
    let a = MockServer::builder().bind("127.0.0.1:0").start().await.unwrap();
    let b = MockServer::builder().bind("127.0.0.1:0").start().await.unwrap();
    let c = MockServer::builder().bind("127.0.0.1:0").start().await.unwrap();

    let ports = [a.addr().port(), b.addr().port(), c.addr().port()];
    let unique: std::collections::HashSet<_> = ports.iter().collect();
    assert_eq!(unique.len(), 3, "each instance needs its own port, got {:?}", ports);

    for s in [&a, &b, &c] {
        let (status, _) = get(&format!("{}/health", s.base_url())).await;
        assert_eq!(status, 200, "every instance should serve independently");
    }

    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    c.shutdown().await.unwrap();
}

/// T-07: explicit shutdown must actually stop serving.
#[tokio::test]
async fn t07_shutdown_stops_the_server() {
    let server = MockServer::builder().bind("127.0.0.1:0").start().await.unwrap();
    let url = format!("{}/health", server.base_url());

    let (status, _) = get(&url).await;
    assert_eq!(status, 200, "should serve before shutdown");

    server.shutdown().await.expect("shutdown should succeed");

    // Give the listener a moment to release.
    tokio::time::sleep(Duration::from_millis(200)).await;

    let result = reqwest::Client::new()
        .get(&url)
        .timeout(Duration::from_secs(2))
        .send()
        .await;
    assert!(result.is_err(), "server must not answer after shutdown");
}

/// T-08: dropping without calling shutdown() must not leak the task or port.
#[tokio::test]
async fn t08_drop_cleans_up() {
    let url = {
        let server = MockServer::builder().bind("127.0.0.1:0").start().await.unwrap();
        let url = format!("{}/health", server.base_url());
        let (status, _) = get(&url).await;
        assert_eq!(status, 200);
        url
        // dropped here without shutdown()
    };

    tokio::time::sleep(Duration::from_millis(300)).await;

    let result = reqwest::Client::new()
        .get(&url)
        .timeout(Duration::from_secs(2))
        .send()
        .await;
    assert!(result.is_err(), "dropping the handle should stop the server");
}

/// T-10: no process-global side effects.
///
/// Prometheus install_recorder() and tracing set_global_default() both error
/// on a second call. If either leaked into the library path, starting a second
/// server in one process would fail. Starting several sequentially and
/// concurrently proves they did not.
#[tokio::test]
async fn t10_no_global_side_effects() {
    for i in 0..3 {
        let s = MockServer::builder()
            .bind("127.0.0.1:0")
            .start()
            .await
            .unwrap_or_else(|e| panic!("instance {} failed to start: {}", i, e));
        let (status, _) = get(&format!("{}/health", s.base_url())).await;
        assert_eq!(status, 200, "instance {} should serve", i);
        s.shutdown().await.unwrap();
    }

    // And concurrently, in case a global is initialised lazily on first request.
    let servers = futures::future::join_all(
        (0..3).map(|_| async {
            MockServer::builder().bind("127.0.0.1:0").start().await.unwrap()
        })
    ).await;
    for s in &servers {
        let (status, _) = get(&format!("{}/health", s.base_url())).await;
        assert_eq!(status, 200);
    }
    for s in servers {
        s.shutdown().await.unwrap();
    }
}

/// T-11: isolated mode must be reachable from the builder, mirroring
/// `--isolated` on the CLI (embedded defaults disabled).
#[tokio::test]
async fn t11_isolated_mode_via_builder() {
    let server = MockServer::builder()
        .bind("127.0.0.1:0")
        .config_dir("config")
        .isolated(true)
        .start()
        .await
        .expect("isolated mode should start when a config dir is present");

    let (status, body) = get(&format!("{}/status", server.base_url())).await;
    assert_eq!(status, 200);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["isolated"], serde_json::json!(true),
        "/status should report isolated=true, got: {}", body);

    server.shutdown().await.unwrap();
}

/// T-12: startup problems must surface as Err, never a panic or exit.
#[tokio::test]
async fn t12_invalid_bind_address_returns_error() {
    let result = MockServer::builder()
        .bind("not-a-valid-address")
        .start()
        .await;

    match result {
        Err(vidaimock::Error::InvalidAddress { addr, .. }) => {
            assert_eq!(addr, "not-a-valid-address",
                "the error should name the address the caller supplied");
        }
        Err(other) => panic!("expected Error::InvalidAddress, got: {:?}", other),
        Ok(_) => panic!("an unparseable bind address must fail"),
    }

    // The error must render usefully — this is what a developer sees first.
    let msg = MockServer::builder()
        .bind("not-a-valid-address")
        .start()
        .await
        .unwrap_err()
        .to_string();
    assert!(msg.contains("not-a-valid-address"),
        "error message should quote the bad address, got: {}", msg);
}
