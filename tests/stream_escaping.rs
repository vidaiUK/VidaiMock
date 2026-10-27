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
 * Every bundled stream template must JSON-escape the chunk text. Before this,
 * text with a quote or backslash produced frames no client could parse.
 */

use std::path::PathBuf;
use vidaimock::MockServer;

const TRICKY: &str = r#"say \"hi\" back\\slash ok"#; // JSON-encoded: say "hi" back\slash ok

/// Every non-blank line is an SSE field, and every `data:` payload except
/// `[DONE]` is valid JSON.
fn assert_frames_parse(wire: &str) {
    for line in wire.lines().filter(|l| !l.trim().is_empty()) {
        assert!(
            line.starts_with("data:") || line.starts_with("event:"),
            "line is not an SSE field: {line}\nwire:\n{wire}"
        );
    }
    let payloads: Vec<&str> = wire
        .lines()
        .filter_map(|l| l.strip_prefix("data:"))
        .map(str::trim)
        .filter(|p| !p.is_empty() && *p != "[DONE]")
        .collect();
    assert!(!payloads.is_empty(), "no data frames, wire:\n{wire}");
    for p in payloads {
        serde_json::from_str::<serde_json::Value>(p)
            .unwrap_or_else(|e| panic!("frame is not JSON ({e}): {p}\nwire:\n{wire}"));
    }
}

async fn post(server: &MockServer, path: &str, body: serde_json::Value) -> String {
    reqwest::Client::new()
        .post(format!("{}{}", server.base_url(), path))
        .json(&body)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap()
}

/// The stock OpenAI chat stream in JSON mode streams `{"x":1}`.
#[tokio::test]
async fn openai_json_mode_stream_frames_parse() {
    let server = MockServer::builder().bind("127.0.0.1:0").start().await.unwrap();
    let wire = post(&server, "/v1/chat/completions", serde_json::json!({
        "model": "gpt-4o", "stream": true,
        "messages": [{"role": "user", "content": "json please"}],
        "response_format": {"type": "json_object"}
    })).await;
    assert_frames_parse(&wire);
    assert!(wire.contains(r#"{\"x\":1}"#), "escaped content missing, wire:\n{wire}");
}

/// One test provider per bundled per-chunk template, each streaming text with
/// quotes and a backslash. Embedded defaults stay on, so the bundled templates
/// are the ones rendered.
fn write_config() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vidaimock-escape-{}", std::process::id()));
    let providers = dir.join("providers");
    std::fs::create_dir_all(&providers).unwrap();
    let body = format!(r#"'{{"choices":[{{"message":{{"role":"assistant","content":"{TRICKY}"}}}}]}}'"#);
    for (name, template, raw) in [
        ("openai", "openai/stream_chunk.json.j2", false),
        ("anthropic", "anthropic/stream/delta.json.j2", true),
        ("gemini", "gemini/stream_chunk.json.j2", false),
        ("cohere", "cohere/stream_chunk.json.j2", false),
    ] {
        let frame = if raw { "\n  frame_format: raw" } else { "" };
        std::fs::write(
            providers.join(format!("esc_{name}.yaml")),
            format!(
                "name: \"esc-{name}\"\nmatcher: \"^/esc/{name}$\"\nresponse_body: {body}\nstream:\n  enabled: true{frame}\n  lifecycle:\n    on_chunk:\n      template_path: \"{template}\"\n"
            ),
        )
        .unwrap();
    }
    dir
}

#[tokio::test]
async fn bundled_chunk_templates_escape_text() {
    let dir = write_config();
    let server = MockServer::builder()
        .bind("127.0.0.1:0")
        .config_dir(&dir)
        .start()
        .await
        .unwrap();
    for name in ["openai", "anthropic", "gemini", "cohere"] {
        let wire = post(&server, &format!("/esc/{name}"), serde_json::json!({
            "model": "m", "stream": true,
            "messages": [{"role": "user", "content": "x"}]
        })).await;
        assert_frames_parse(&wire);
        assert!(wire.contains(r#"\"hi\""#), "{name}: escaped quote missing, wire:\n{wire}");
        assert!(wire.contains(r#"back\\slash"#), "{name}: escaped backslash missing, wire:\n{wire}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
