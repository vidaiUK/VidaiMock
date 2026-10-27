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
 * VidaiMock: High-performance LLM API Mock Server.
 */

use crc32fast::Hasher;
use base64::prelude::*;

pub struct AwsEventStreamEncoder;

/// How lifecycle template output becomes AWS event-stream messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AwsPayload {
    /// InvokeModelWithResponseStream: every event is `:event-type chunk` with
    /// payload `{"bytes":"<base64 of the template's JSON>"}`.
    Bytes,
    /// ConverseStream: `:event-type` comes from the template's `event:` line
    /// and the payload is the JSON as written.
    Json,
}

impl AwsPayload {
    pub fn from_config(value: Option<&str>) -> Self {
        match value {
            Some("json") => AwsPayload::Json,
            _ => AwsPayload::Bytes,
        }
    }
}

impl AwsEventStreamEncoder {
    /// Encodes a payload into an AWS Event Stream binary message.
    ///
    /// The payload is base64-encoded, wrapped in `{"bytes": "..."}` and framed
    /// as a `chunk` event — the InvokeModelWithResponseStream shape.
    pub fn encode_chunk(payload_json: &str) -> Vec<u8> {
        Self::encode_event("chunk", &bytes_envelope(payload_json))
    }

    /// Encodes one AWS Event Stream message with the given `:event-type` and
    /// payload, sent as is.
    pub fn encode_event(event_type: &str, payload: &str) -> Vec<u8> {
        let payload_bytes = payload.as_bytes();

        let mut headers = Vec::new();
        write_header(&mut headers, ":event-type", event_type);
        write_header(&mut headers, ":content-type", "application/json");
        write_header(&mut headers, ":message-type", "event");

        // Total Len (4) + Headers Len (4) + Prelude CRC (4) + Headers + Payload + Message CRC (4)
        let total_len = 4 + 4 + 4 + headers.len() as u32 + payload_bytes.len() as u32 + 4;
        let headers_len = headers.len() as u32;

        let mut message = Vec::with_capacity(total_len as usize);
        message.extend_from_slice(&total_len.to_be_bytes());
        message.extend_from_slice(&headers_len.to_be_bytes());

        let mut crc = Hasher::new();
        crc.update(&message[0..8]);
        message.extend_from_slice(&crc.finalize().to_be_bytes());

        message.extend_from_slice(&headers);
        message.extend_from_slice(payload_bytes);

        let mut crc = Hasher::new();
        crc.update(&message);
        message.extend_from_slice(&crc.finalize().to_be_bytes());

        message
    }

    /// Encodes one lifecycle template's output as one or more messages.
    ///
    /// Output written as SSE-style blocks (`event: x` / `data: {...}`,
    /// separated by blank lines) yields one message per `data:` line, so one
    /// template can emit several events. Output with no `data:` line is sent
    /// as a single message (the original behaviour).
    pub fn encode_template_output(output: &str, mode: AwsPayload) -> Vec<u8> {
        let has_blocks = output.lines().any(|l| l.trim_start().starts_with("data:"));
        if !has_blocks {
            return match mode {
                AwsPayload::Bytes => Self::encode_chunk(output.trim()),
                AwsPayload::Json => Self::encode_event("chunk", output.trim()),
            };
        }
        let mut out = Vec::new();
        let mut event_type: Option<String> = None;
        for line in output.lines().map(str::trim) {
            if line.is_empty() {
                event_type = None;
            } else if let Some(name) = line.strip_prefix("event:") {
                event_type = Some(name.trim().to_string());
            } else if let Some(data) = line.strip_prefix("data:") {
                let data = data.trim();
                out.extend(match mode {
                    AwsPayload::Bytes => Self::encode_chunk(data),
                    AwsPayload::Json => Self::encode_event(event_type.as_deref().unwrap_or("chunk"), data),
                });
            }
        }
        out
    }
}

fn bytes_envelope(payload_json: &str) -> String {
    format!(r#"{{"bytes":"{}"}}"#, BASE64_STANDARD.encode(payload_json))
}

/// Helper to write an AWS Event Stream String Header
fn write_header(buf: &mut Vec<u8>, key: &str, value: &str) {
    // Key Len (1 byte)
    buf.push(key.len() as u8);
    // Key
    buf.extend_from_slice(key.as_bytes());
    // Type (7 = String)
    buf.push(7);
    // Value Len (2 bytes)
    buf.extend_from_slice(&(value.len() as u16).to_be_bytes());
    // Value
    buf.extend_from_slice(value.as_bytes());
}
