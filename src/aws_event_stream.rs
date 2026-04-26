/*
 * Copyright (c) 2025 Vidai UK.
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

use base64::prelude::*;
use crc32fast::Hasher;

pub struct AwsEventStreamEncoder;

impl AwsEventStreamEncoder {
    /// Encodes a payload into an AWS Event Stream binary message.
    ///
    /// The payload is expected to be a valid JSON string (e.g. `{"type":"content_block_delta"...}`).
    /// This function:
    /// 1. Base64 encodes that JSON.
    /// 2. Wraps it in `{"bytes": "..."}` JSON object.
    /// 3. Frames it with AWS Event Stream binary headers and CRCs.
    pub fn encode_chunk(payload_json: &str) -> Vec<u8> {
        // 1. Base64 encode the inner payload
        let b64_payload = BASE64_STANDARD.encode(payload_json);

        // 2. Wrap in {"bytes": "..."}
        let outer_payload = format!(r#"{{"bytes":"{}"}}"#, b64_payload);
        let payload_bytes = outer_payload.as_bytes();

        // 3. Prepare headers
        let mut headers = Vec::new();
        // :event-type = chunk
        write_header(&mut headers, ":event-type", "chunk");
        // :content-type = application/json
        write_header(&mut headers, ":content-type", "application/json");
        // :message-type = event
        write_header(&mut headers, ":message-type", "event");

        // 4. Calculate lengths
        // Total Len (4) + Headers Len (4) + Prelude CRC (4) + Headers + Payload + Message CRC (4)
        let total_len = 4 + 4 + 4 + headers.len() as u32 + payload_bytes.len() as u32 + 4;
        let headers_len = headers.len() as u32;

        let mut message = Vec::with_capacity(total_len as usize);

        // 5. Write Prelude (Total Len + Headers Len)
        message.extend_from_slice(&total_len.to_be_bytes());
        message.extend_from_slice(&headers_len.to_be_bytes());

        // 6. Prelude CRC
        let mut crc = Hasher::new();
        crc.update(&message[0..8]);
        message.extend_from_slice(&crc.finalize().to_be_bytes());

        // 7. Write Headers
        message.extend_from_slice(&headers);

        // 8. Write Payload
        message.extend_from_slice(payload_bytes);

        // 9. Message CRC
        let mut crc = Hasher::new();
        crc.update(&message); // Everything so far
        message.extend_from_slice(&crc.finalize().to_be_bytes());

        message
    }
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
