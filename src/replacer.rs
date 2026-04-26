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

use chrono::{SecondsFormat, Utc};
use serde_json::Value;
use std::collections::HashMap;
use tera::Context;
use uuid::Uuid;

use crate::tenancy::TenantTemplateMetadata;

pub struct Replacer;

impl Replacer {
    /// Builds a Tera Context from the request data
    pub fn build_context(
        request_json: Option<&Value>,
        headers: &HashMap<String, String>,
        query_params: &HashMap<String, String>,
        path_segments: &[String],
        model: &str,
        tenant: &TenantTemplateMetadata,
    ) -> Context {
        let mut context = Context::new();

        // 1. Standard Variables
        context.insert("timestamp", &Utc::now().timestamp());
        context.insert(
            "iso_timestamp",
            &Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
        );
        context.insert("uuid", &Uuid::new_v4().to_string());
        context.insert(
            "request_id",
            &format!("req_{}", &Uuid::new_v4().to_string()[..8]),
        );
        context.insert("model", model);

        // 2. Request Data
        if let Some(json) = request_json {
            context.insert("json", json);
        }
        context.insert("headers", headers);
        context.insert("query", query_params);
        context.insert("path_segments", path_segments);
        context.insert("tenant", &tenant);

        // 3. Helper Functions (via register_function if we had mutable access to terra,
        // but for Context we just add data. Complex logic should be in filters/functions registered on Tera instance)

        context
    }
}
