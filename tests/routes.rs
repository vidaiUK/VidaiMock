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
 * Routes gateways forward that used to answer "No provider matched" or 405
 * (VidaiTestSuit MOCK_GAPS MG-007, MG-012, MG-013, MG-015).
 */

use reqwest::multipart::{Form, Part};
use serde_json::{json, Value};
use vidaimock::MockServer;

async fn server() -> MockServer {
    MockServer::builder().bind("127.0.0.1:0").start().await.unwrap()
}

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

async fn json_of(r: reqwest::Response) -> Value {
    let status = r.status();
    let text = r.text().await.unwrap();
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{status}: not JSON ({e}): {text}"))
}

fn audio_form(model: &str, format: Option<&str>) -> Form {
    let f = Form::new()
        .text("model", model.to_string())
        .part("file", Part::bytes(vec![0u8; 2048]).file_name("a.wav").mime_str("audio/wav").unwrap());
    match format {
        Some(fmt) => f.text("response_format", fmt.to_string()),
        None => f,
    }
}

// MG-007
#[tokio::test]
async fn anthropic_count_tokens() {
    let s = server().await;
    let r = client().post(format!("{}/v1/messages/count_tokens", s.base_url()))
        .json(&json!({"model": "claude-sonnet-4-5", "messages": [{"role": "user", "content": "Hello, how many tokens is this message?"}]}))
        .send().await.unwrap();
    assert_eq!(r.status(), 200);
    let v = json_of(r).await;
    let n = v["input_tokens"].as_u64().expect("input_tokens");
    assert!(n > 5 && n < 100, "{v}");
    assert_eq!(v.as_object().unwrap().len(), 1, "only input_tokens: {v}");
}

// MG-012
#[tokio::test]
async fn legacy_completions_non_stream_and_stream() {
    let s = server().await;
    let url = format!("{}/v1/completions", s.base_url());
    let v = json_of(client().post(&url).json(&json!({"model": "gpt-3.5-turbo-instruct", "prompt": "Say hi"})).send().await.unwrap()).await;
    assert_eq!(v["object"], "text_completion");
    assert!(v["choices"][0]["text"].is_string());
    assert_eq!(v["choices"][0]["finish_reason"], "stop");
    assert!(v["usage"]["total_tokens"].is_number());

    let wire = client().post(&url)
        .json(&json!({"model": "gpt-3.5-turbo-instruct", "prompt": "Say hi", "stream": true, "stream_options": {"include_usage": true}}))
        .send().await.unwrap().text().await.unwrap();
    let data: Vec<&str> = wire.lines().filter_map(|l| l.strip_prefix("data: ")).collect();
    assert_eq!(data.last(), Some(&"[DONE]"), "{wire}");
    let events: Vec<Value> = data[..data.len() - 1].iter().map(|d| serde_json::from_str(d).unwrap()).collect();
    let text: String = events.iter().filter_map(|e| e["choices"][0]["text"].as_str()).collect();
    assert_eq!(text.trim(), "This is a mock completion from VidaiMock.");
    assert!(events.iter().any(|e| e["choices"][0]["finish_reason"] == "stop"), "{wire}");
    assert!(events.last().unwrap()["usage"]["total_tokens"].is_number(), "{wire}");
}

// MG-013
#[tokio::test]
async fn responses_get_delete_and_input_items() {
    let s = server().await;
    let url = format!("{}/v1/responses/resp_abc123", s.base_url());
    let got = json_of(client().get(&url).send().await.unwrap()).await;
    assert_eq!(got["id"], "resp_abc123");
    assert_eq!(got["object"], "response");
    assert_eq!(got["status"], "completed");
    assert_eq!(got["output"][0]["content"][0]["type"], "output_text");

    let items = json_of(client().get(format!("{url}/input_items")).send().await.unwrap()).await;
    assert_eq!(items["object"], "list");

    let del = json_of(client().delete(&url).send().await.unwrap()).await;
    assert_eq!(del, json!({"id": "resp_abc123", "object": "response", "deleted": true}));

    // Creating still goes to the create provider.
    let created = json_of(client().post(format!("{}/v1/responses", s.base_url()))
        .json(&json!({"model": "gpt-4o-mini", "input": "hi"})).send().await.unwrap()).await;
    assert!(created["id"].as_str().unwrap().starts_with("resp_"));
}

/// A GET on a POST-only route is a clear 404 from the matcher, not an empty 405.
#[tokio::test]
async fn unmatched_get_explains_itself() {
    let s = server().await;
    let r = client().get(format!("{}/v1/not-a-thing", s.base_url())).send().await.unwrap();
    assert_eq!(r.status(), 404);
    assert!(r.text().await.unwrap().contains("No provider matched"));
}

// MG-015: audio
#[tokio::test]
async fn audio_speech_returns_audio_bytes() {
    let s = server().await;
    let url = format!("{}/v1/audio/speech", s.base_url());
    let r = client().post(&url).json(&json!({"model": "gpt-4o-mini-tts", "input": "hi", "voice": "alloy"})).send().await.unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers()["content-type"], "audio/mpeg");
    let mp3 = r.bytes().await.unwrap();
    assert_eq!(&mp3[..2], &[0xFF, 0xFB], "MPEG frame sync");

    let r = client().post(&url).json(&json!({"model": "tts-1", "input": "hi", "voice": "alloy", "response_format": "wav"})).send().await.unwrap();
    assert_eq!(r.headers()["content-type"], "audio/wav");
    let wav = r.bytes().await.unwrap();
    assert_eq!(&wav[..4], b"RIFF");
    assert_eq!(&wav[8..12], b"WAVE");
}

#[tokio::test]
async fn audio_transcription_multipart_formats_and_usage() {
    let s = server().await;
    let url = format!("{}/v1/audio/transcriptions", s.base_url());

    let v = json_of(client().post(&url).multipart(audio_form("whisper-1", None)).send().await.unwrap()).await;
    assert!(v["text"].is_string());
    assert_eq!(v["usage"]["type"], "duration");

    let v = json_of(client().post(&url).multipart(audio_form("gpt-4o-transcribe", None)).send().await.unwrap()).await;
    assert_eq!(v["usage"]["type"], "tokens");
    assert!(v["usage"]["total_tokens"].is_number());

    let r = client().post(&url).multipart(audio_form("whisper-1", Some("text"))).send().await.unwrap();
    assert!(r.headers()["content-type"].to_str().unwrap().starts_with("text/plain"));
    assert_eq!(r.text().await.unwrap().trim(), "This is a mock transcription from VidaiMock.");

    let v = json_of(client().post(&url).multipart(audio_form("whisper-1", Some("verbose_json"))).send().await.unwrap()).await;
    assert_eq!(v["task"], "transcribe");
    assert!(v["segments"][0]["text"].is_string());

    let r = client().post(&url).multipart(audio_form("whisper-1", Some("vtt"))).send().await.unwrap();
    assert!(r.text().await.unwrap().starts_with("WEBVTT"));

    let v = json_of(client().post(format!("{}/v1/audio/translations", s.base_url()))
        .multipart(audio_form("whisper-1", None)).send().await.unwrap()).await;
    assert!(v["text"].is_string());
}

// MG-015: videos
#[tokio::test]
async fn video_job_lifecycle() {
    let s = server().await;
    let base = format!("{}/v1/videos", s.base_url());
    let created = json_of(client().post(&base).json(&json!({"model": "sora-2", "prompt": "a cat", "seconds": "8"})).send().await.unwrap()).await;
    assert_eq!(created["object"], "video");
    assert_eq!(created["status"], "queued");
    assert_eq!(created["seconds"], "8");
    assert!(created["completed_at"].is_null());
    let id = created["id"].as_str().unwrap().to_string();
    assert!(id.starts_with("video_"));

    let got = json_of(client().get(format!("{base}/{id}")).send().await.unwrap()).await;
    assert_eq!(got["id"], id.as_str());
    assert_eq!(got["status"], "completed");
    assert!(got["completed_at"].is_number());

    let list = json_of(client().get(&base).send().await.unwrap()).await;
    assert_eq!(list["object"], "list");

    let content = client().get(format!("{base}/{id}/content")).send().await.unwrap();
    assert_eq!(content.headers()["content-type"], "video/mp4");
    assert_eq!(&content.bytes().await.unwrap()[4..8], b"ftyp");

    let del = json_of(client().delete(format!("{base}/{id}")).send().await.unwrap()).await;
    assert_eq!(del["deleted"], true);
}

// MG-015: OCR
#[tokio::test]
async fn mistral_ocr_pages_and_usage() {
    let s = server().await;
    let v = json_of(client().post(format!("{}/v1/ocr", s.base_url()))
        .json(&json!({"model": "mistral-ocr-latest", "document": {"type": "document_url", "document_url": "https://x/doc.pdf"}, "pages": [0, 1, 2]}))
        .send().await.unwrap()).await;
    assert_eq!(v["pages"].as_array().unwrap().len(), 3, "{v}");
    assert_eq!(v["pages"][2]["index"], 2);
    assert_eq!(v["usage_info"]["pages_processed"], 3);
}

// MG-015: images
#[tokio::test]
async fn images_honour_n_and_report_usage_for_gpt_image() {
    let s = server().await;
    let url = format!("{}/v1/images/generations", s.base_url());
    let v = json_of(client().post(&url).json(&json!({"model": "gpt-image-1", "prompt": "a cat", "n": 2})).send().await.unwrap()).await;
    assert_eq!(v["data"].as_array().unwrap().len(), 2, "{v}");
    assert!(v["data"][0]["b64_json"].is_string());
    assert!(v["usage"]["output_tokens"].is_number());

    let v = json_of(client().post(&url).json(&json!({"model": "dall-e-3", "prompt": "a \"quoted\" cat"})).send().await.unwrap()).await;
    assert!(v["data"][0]["url"].is_string());
    assert_eq!(v["data"][0]["revised_prompt"], "a \"quoted\" cat");
    assert!(v.get("usage").is_none());

    let form = Form::new().text("model", "gpt-image-1").text("prompt", "edit it").text("n", "3")
        .part("image", Part::bytes(vec![1u8; 64]).file_name("in.png").mime_str("image/png").unwrap());
    let v = json_of(client().post(format!("{}/v1/images/edits", s.base_url())).multipart(form).send().await.unwrap()).await;
    assert_eq!(v["data"].as_array().unwrap().len(), 3, "{v}");

    let form = Form::new().text("n", "1")
        .part("image", Part::bytes(vec![1u8; 64]).file_name("in.png").mime_str("image/png").unwrap());
    let v = json_of(client().post(format!("{}/v1/images/variations", s.base_url())).multipart(form).send().await.unwrap()).await;
    assert_eq!(v["data"].as_array().unwrap().len(), 1, "{v}");
}
