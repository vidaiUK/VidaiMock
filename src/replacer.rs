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

use tera::Context;
use serde_json::Value;
use std::collections::HashMap;
use chrono::{Utc, SecondsFormat};
use uuid::Uuid;


pub struct Replacer;

impl Replacer {
    /// Builds a Tera Context from the request data
    pub fn build_context(
        request_json: Option<&Value>,
        headers: &HashMap<String, String>,
        query_params: &HashMap<String, String>,
        path_segments: &[String],
        model: &str,
    ) -> Context {
        let mut context = Context::new();

        // 1. Standard Variables
        context.insert("timestamp", &Utc::now().timestamp());
        context.insert("iso_timestamp", &Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true));
        context.insert("uuid", &Uuid::new_v4().to_string());
        context.insert("request_id", &format!("req_{}", &Uuid::new_v4().to_string()[..8]));
        context.insert("model", model);

        // 2. Request Data
        if let Some(json) = request_json {
            context.insert("json", json);
        }
        context.insert("headers", headers);
        context.insert("query", query_params);
        let mut usage = usage_override(headers);
        if let Some(n) = requested_output_tokens(headers, request_json) {
            // Scaled output: N words of filler text, and N reported output tokens.
            context.insert("generated_text", &filler_text(n));
            context.insert("output_tokens", &n);
            usage.entry("output".to_string()).or_insert(Value::from(n));
        }
        context.insert("usage_override", &usage);
        context.insert("path_segments", path_segments);

        // 3. Helper Functions (via register_function if we had mutable access to terra, 
        // but for Context we just add data. Complex logic should be in filters/functions registered on Tera instance)
        
        context
    }
}

/// Parse `X-Vidai-Usage: input=120,output=30,cached=100,reasoning=12,cache_write=0`
/// into an object templates read as `usage_override`. Unknown keys and bad
/// numbers are ignored; no header gives an empty object.
pub fn usage_override(headers: &HashMap<String, String>) -> serde_json::Map<String, Value> {
    const KEYS: &[&str] = &["input", "output", "cached", "cache_write", "reasoning"];
    let mut out = serde_json::Map::new();
    let Some(raw) = headers.get("x-vidai-usage") else { return out };
    for pair in raw.split(',') {
        let Some((k, v)) = pair.split_once('=') else { continue };
        let k = k.trim().to_ascii_lowercase();
        if let (true, Ok(n)) = (KEYS.contains(&k.as_str()), v.trim().parse::<u64>()) {
            out.insert(k, Value::from(n));
        }
    }
    out
}

/// Upper bound for `X-Vidai-Output-Tokens`, so one request cannot ask for
/// gigabytes of text.
const MAX_OUTPUT_TOKENS: u64 = 100_000;

/// `X-Vidai-Output-Tokens: N` (or `max`, meaning the request's own limit:
/// max_tokens, max_completion_tokens, max_output_tokens or Gemini's
/// generationConfig.maxOutputTokens). None when the header is absent.
pub fn requested_output_tokens(headers: &HashMap<String, String>, request_json: Option<&Value>) -> Option<u64> {
    let raw = headers.get("x-vidai-output-tokens")?.trim().to_ascii_lowercase();
    let n = if raw == "max" {
        let j = request_json?;
        ["max_tokens", "max_completion_tokens", "max_output_tokens"]
            .iter()
            .find_map(|k| j.get(*k).and_then(Value::as_u64))
            .or_else(|| j.pointer("/generationConfig/maxOutputTokens").and_then(Value::as_u64))?
    } else {
        raw.parse::<u64>().ok()?
    };
    Some(n.clamp(1, MAX_OUTPUT_TOKENS))
}

/// `n` words of plain, JSON-safe filler text ending with a full stop.
pub fn filler_text(n: u64) -> String {
    const WORDS: &[&str] = &[
        "the", "mock", "model", "streams", "this", "text", "so", "clients", "see", "a",
        "realistic", "number", "of", "tokens", "for", "load", "and", "billing", "tests", "today",
    ];
    let mut out = String::with_capacity(n as usize * 7);
    for i in 0..n as usize {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(WORDS[i % WORDS.len()]);
    }
    out.push('.');
    out
}
