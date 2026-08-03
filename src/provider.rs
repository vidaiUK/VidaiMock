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

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::fs;
use glob::glob;
use regex::Regex;
use tera::Tera;
use rand::Rng; // For random functions
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "config/"]
pub struct Asset;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub name: String,
    /// Regex pattern to match the request path (e.g., "^/v1/chat/completions$")
    pub matcher: String,
    /// Mapping of internal variable names to template expressions
    /// e.g. "prompt": "{{ json.messages | last | get('content') }}"
    #[serde(default)]
    pub request_mapping: HashMap<String, String>,
    /// Path to the detailed response template file (relative to config/templates/)
    pub response_template: Option<String>,
    /// Inline response template string (optional override)
    pub response_body: Option<String>,
    #[serde(default)]
    pub stream: Option<ProviderStreamConfig>,
    /// HTTP status code to return (default: 200). Supports static codes ("400")
    /// or Tera expressions ("{{ path_segments | last }}") for dynamic extraction.
    pub status_code: Option<String>,
    /// Optional template rendered when a chaos/failure override triggers an error
    /// response (e.g. `?chaos_status=500` query param, `X-Vidai-Chaos-Drop`).
    /// Falls back to the standard `response_template` when absent.
    /// Lets providers supply provider-shaped error envelopes
    /// (OpenAI: `{"error": {...}}`, Anthropic: `{"type": "error", ...}`, etc.).
    pub error_template: Option<String>,
    /// Priority for matching (higher matches first)
    #[serde(default)]
    pub priority: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderStreamConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Default format string for data tokens if no detailed lifecycle is provided
    /// e.g. "data: {{ json }}\n\n"
    pub format: Option<String>,
    /// Encoding for the stream (e.g. "aws-event-stream"). Default is SSE.
    pub encoding: Option<String>,
    /// Controls SSE frame wrapping. Default "sse" auto-prefixes each chunk with "data: ".
    /// Set to "raw" to emit template output verbatim — the template controls all framing
    /// including event: and data: lines. Required for providers with typed SSE events.
    pub frame_format: Option<String>,
    /// Detailed lifecycle events for complex streams (Anthropic, etc.)
    pub lifecycle: Option<StreamLifecycle>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamLifecycle {
    pub on_start: Option<StreamEvent>,
    pub on_chunk: Option<StreamEvent>,
    pub on_stop: Option<StreamEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamEvent {
    pub event_name: Option<String>, // e.g., "message_start"
    pub template_path: Option<String>,
    pub template_body: Option<String>,
}

pub struct ProviderRegistry {
    pub providers: Vec<ProviderConfig>,
    compiled_matchers: Vec<(Regex, usize)>, // (Regex, index in providers)
    pub tera: Arc<Tera>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
            compiled_matchers: Vec::new(),
            tera: Arc::new(Tera::default()),
        }
    }

    /// Backwards-compatible shim: load with the binary's embedded defaults
    /// merged under the disk overlay. Equivalent to `load_from_dir_with_options(dir, false)`.
    pub fn load_from_dir(&mut self, config_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
        self.load_from_dir_with_options(config_dir, false)
    }

    /// Load provider configs and templates.
    ///
    /// When `isolated` is false (the default), the binary's embedded
    /// `config/providers/*.yaml` and `config/templates/**/*.j2` are loaded
    /// first, then anything in `config_dir` overlays them (disk wins).
    ///
    /// When `isolated` is true, the embedded layer is skipped entirely —
    /// only `config_dir` contributes. Useful when the user wants to lock the
    /// surface down to exactly what they declare (issue #6).
    pub fn load_from_dir_with_options(
        &mut self,
        config_dir: &Path,
        isolated: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let providers_pattern = config_dir.join("providers/*.yaml");
        // let templates_pattern = config_dir.join("templates/**/*");

        // 1. Load Templates
        // Setup Tera with all templates (embedded + disk)
        let mut tera = Tera::default();
        
        // Disable autoescape for JSON generation
        tera.autoescape_on(vec![]);

        // Register custom functions
        tera.register_function("uuid", |_args: &HashMap<String, tera::Value>| -> tera::Result<tera::Value> {
            Ok(tera::Value::String(uuid::Uuid::new_v4().to_string()))
        });

        tera.register_function("timestamp", |_args: &HashMap<String, tera::Value>| -> tera::Result<tera::Value> {
             let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
             Ok(tera::Value::Number(serde_json::Number::from(now)))
        });

        tera.register_function("iso_timestamp", |_args: &HashMap<String, tera::Value>| -> tera::Result<tera::Value> {
             Ok(tera::Value::String(chrono::Utc::now().to_rfc3339()))
        });

        tera.register_function("random_int", |args: &HashMap<String, tera::Value>| -> tera::Result<tera::Value> {
             let min = args.get("min").and_then(|v| v.as_i64()).unwrap_or(0);
             let max = args.get("max").and_then(|v| v.as_i64()).unwrap_or(100);
             let val = rand::rng().random_range(min..=max);
             Ok(tera::Value::Number(serde_json::Number::from(val)))
        });
        
        tera.register_function("random_float", |args: &HashMap<String, tera::Value>| -> tera::Result<tera::Value> {
             let min = args.get("min").and_then(|v| v.as_f64()).unwrap_or(0.0);
             let max = args.get("max").and_then(|v| v.as_f64()).unwrap_or(1.0);
             let val = rand::rng().random_range(min..=max);
             Ok(tera::Value::from(val))
        });

        // has_tool_result(messages=json.messages, provider="openai") -> bool
        //
        // Detects whether the request's conversation history already contains a
        // tool result. Templates use this to terminate agentic tool-calling
        // loops: when a tool result is in the history, emit a plain-text
        // synthesis instead of another tool_calls response (which would loop
        // forever).
        //
        // Done in Rust rather than Tera because deep JSON-array inspection
        // (e.g. looking inside `content` arrays of objects) is unreliable from
        // Tera expressions — we'd need `.` traversal through arrays of mixed
        // types. A native Rust check is both faster and more robust.
        //
        // Provider-specific shapes recognised:
        //   openai    — message with role=="tool"
        //   anthropic — user message whose content array contains type=="tool_result"
        //   gemini    — user content whose parts array contains a functionResponse key
        //
        // Returns false (not an error) on missing/null/malformed inputs so
        // template logic stays branchable without defensive filters.
        tera.register_function("has_tool_result", |args: &HashMap<String, tera::Value>| -> tera::Result<tera::Value> {
            let messages = match args.get("messages") {
                Some(tera::Value::Array(a)) => a,
                _ => return Ok(tera::Value::Bool(false)),
            };
            let provider = args.get("provider").and_then(|v| v.as_str()).unwrap_or("openai");

            let found = messages.iter().any(|msg| match provider {
                "openai" => {
                    // tool-role message anywhere signals a prior tool_call was answered.
                    msg.get("role").and_then(|r| r.as_str()) == Some("tool")
                }
                "anthropic" => {
                    // user message carrying a tool_result content block.
                    let is_user = msg.get("role").and_then(|r| r.as_str()) == Some("user");
                    let has_block = msg.get("content")
                        .and_then(|c| c.as_array())
                        .map(|blocks| blocks.iter().any(|b| {
                            b.get("type").and_then(|t| t.as_str()) == Some("tool_result")
                        }))
                        .unwrap_or(false);
                    is_user && has_block
                }
                "gemini" => {
                    // user content whose parts array includes a functionResponse.
                    let is_user = msg.get("role").and_then(|r| r.as_str()) == Some("user");
                    let has_fr = msg.get("parts")
                        .and_then(|p| p.as_array())
                        .map(|parts| parts.iter().any(|p| p.get("functionResponse").is_some()))
                        .unwrap_or(false);
                    is_user && has_fr
                }
                _ => false,
            });

            Ok(tera::Value::Bool(found))
        });

        let pick_filter = |value: &tera::Value, args: &HashMap<String, tera::Value>| -> tera::Result<tera::Value> {
             let key = args.get("key").and_then(|v| v.as_str())
                 .or_else(|| args.get("0").and_then(|v| v.as_str()))
                 .ok_or_else(|| tera::Error::msg("Filter 'get' or 'pick' requires a 'key' argument"))?;
             match value {
                 tera::Value::Object(map) => {
                     Ok(map.get(key).cloned().unwrap_or(tera::Value::Null))
                 },
                 _ => Ok(tera::Value::Null)
             }
        };

        tera.register_filter("pick", pick_filter);
        tera.register_filter("get", pick_filter);
        
        tera.register_filter("minify", |value: &tera::Value, _args: &HashMap<String, tera::Value>| -> tera::Result<tera::Value> {
            match value {
                tera::Value::String(s) => {
                    // Simple minification: parse as JSON and re-serialize to compact string
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(s) {
                        Ok(tera::Value::String(serde_json::to_string(&json).unwrap()))
                    } else {
                        // Fallback to basic whitespace removal if not valid JSON
                        Ok(tera::Value::String(s.lines().map(|line| line.trim()).collect::<Vec<_>>().join("")))
                    }
                },
                _ => Ok(value.clone())
            }
        });

        // a. Collect all templates (embedded first, then disk for overrides)
        let mut template_map = HashMap::new();

        // 1. Embedded templates (skipped when isolated — user owns the surface)
        if !isolated {
            for file in Asset::iter() {
                if file.starts_with("templates/") {
                    if let Some(content) = Asset::get(&file) {
                        let template_str = std::str::from_utf8(content.data.as_ref())?.to_string();
                        let name = file["templates/".len()..].to_string();
                        tracing::debug!("Found embedded template: {}", name);
                        template_map.insert(name, template_str);
                    }
                }
            }
        }

        // 2. Disk templates (overrides embedded)
        let templates_glob = config_dir.join("templates/**/*");
        if let Ok(entries) = glob(templates_glob.to_str().ok_or("Invalid template path")?) {
            for entry in entries {
                if let Ok(path) = entry {
                    if path.is_file() {
                         let content = fs::read_to_string(&path)?;
                         let rel_path = path.strip_prefix(config_dir.join("templates/"))?;
                         let name = rel_path.to_str().ok_or("Invalid path")?.to_string();
                         template_map.insert(name.clone(), content);
                         tracing::debug!("Found disk template override: {}", name);
                    }
                }
            }
        }

        // b. Add all collected templates to Tera
        for (name, content) in template_map {
            // Register with the relative name (e.g., openai/chat.json.j2)
            if let Err(e) = tera.add_raw_template(&name, &content) {
                 tracing::error!("Failed to parse template '{}': {:?}", name, e);
            } else {
                 tracing::debug!("Registered template: {}", name);
            }
            
            // Also register with config/templates/ prefix for compatibility with some provider configs
            let full_name = format!("config/templates/{}", name);
            if let Err(_) = tera.add_raw_template(&full_name, &content) {
                // Ignore errors for the alias if the template already failed or something
            }
        }

        // 2. Load Providers
        // a. Load embedded and disk providers with shadowing
        let mut loaded_provider_names = std::collections::HashSet::new();
        
        // 2. Load providers into a temporary list for sorting
        let mut all_configs = Vec::new();

        // 1. Load disk providers
        if let Ok(entries) = glob(providers_pattern.to_str().unwrap()) {
            for entry in entries {
                if let Ok(path) = entry {
                    if path.is_file() {
                        let content = fs::read_to_string(&path)?;
                        if let Ok(config) = serde_yaml::from_str::<ProviderConfig>(&content) {
                            tracing::debug!("Discovered disk provider: {} ({})", config.name, path.display());
                            all_configs.push(config);
                            loaded_provider_names.insert(format!("providers/{}", path.file_name().unwrap().to_str().unwrap()));
                        }
                    }
                }
            }
        }

        // 2. Load embedded providers that were NOT on disk (skipped when isolated)
        if !isolated {
            for file in Asset::iter() {
                if file.starts_with("providers/") && file.ends_with(".yaml") && !loaded_provider_names.contains(file.as_ref()) {
                    if let Some(content) = Asset::get(&file) {
                        let config_str = std::str::from_utf8(content.data.as_ref())?;
                        if let Ok(config) = serde_yaml::from_str::<ProviderConfig>(config_str) {
                            tracing::debug!("Discovered embedded provider: {}", config.name);
                            all_configs.push(config);
                        }
                    }
                }
            }
        }

        // 3. Sort by priority (descending)
        all_configs.sort_by(|a, b| b.priority.cmp(&a.priority));

        // 4. Register sorted providers
        self.providers.clear();
        self.compiled_matchers.clear();
        for config in all_configs {
            let regex = Regex::new(&config.matcher)?;
            self.compiled_matchers.push((regex, self.providers.len()));
            self.providers.push(config);
        }

        let mode_label = if isolated { "Disk only — embedded defaults disabled" } else { "Disk + Embedded" };
        if self.providers.is_empty() {
            if isolated {
                // Isolated mode + empty result is almost always a config mistake
                // — embedded defaults aren't there to catch the user.
                tracing::warn!(
                    "No providers loaded from {} (isolated mode — embedded defaults disabled). \
                     Every request will return 404 until you add a provider YAML.",
                    config_dir.display()
                );
            } else {
                tracing::warn!(
                    "No providers found in configuration directory: {}",
                    config_dir.display()
                );
            }
        } else {
            tracing::info!("Registered {} providers ({})", self.providers.len(), mode_label);
        }
        
        self.tera = Arc::new(tera);
        Ok(())
    }

    pub fn find_provider(&self, path: &str) -> Option<&ProviderConfig> {
        for (regex, idx) in &self.compiled_matchers {
            if regex.is_match(path) {
                return Some(&self.providers[*idx]);
            }
        }
        None
    }

    pub fn add_provider(&mut self, config: ProviderConfig) -> Result<(), Box<dyn std::error::Error>> {
        let regex = Regex::new(&config.matcher)?;
        self.compiled_matchers.push((regex, self.providers.len()));
        self.providers.push(config);
        Ok(())
    }

    /// Renders an ad-hoc string using the registered Tera instance
    pub fn render_str(&self, template: &str, context: &tera::Context) -> tera::Result<String> {
        tera::Tera::one_off(template, context, false)
    }
}

/// Default-mode init: load embedded defaults then overlay `config_path`.
/// Kept as a thin wrapper for callers (and tests) that don't care about isolated mode.
#[allow(dead_code)] // kept for API stability — the binary uses init_registry_with_options
pub fn init_registry(config_path: &Path) -> Arc<ProviderRegistry> {
    init_registry_with_options(config_path, false)
}

/// Init with explicit isolated flag. When `isolated = true`, embedded
/// defaults are skipped and only `config_path` contributes.
pub fn init_registry_with_options(config_path: &Path, isolated: bool) -> Arc<ProviderRegistry> {
    let mut registry = ProviderRegistry::new();
    if let Err(e) = registry.load_from_dir_with_options(config_path, isolated) {
        tracing::error!("Failed to load provider registry: {}", e);
    }
    Arc::new(registry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn test_embedded_assets_load() {
        let mut registry = ProviderRegistry::new();
        // Load from a non-existent directory to ensure it only loads embedded
        registry.load_from_dir(&PathBuf::from("non_existent_dir")).unwrap();
        
        // Check if openai provider is in the list by name
        let has_openai = registry.providers.iter().any(|p| p.name == "openai");
        assert!(has_openai, "Embedded 'openai' provider should be loaded");
    }

    #[test]
    fn test_disk_shadowing() {
        // Use a unique subdir for this test to avoid conflicts
        let temp_base = std::env::current_dir().unwrap().join("target/test_shadowing_unique");
        if temp_base.exists() {
            fs::remove_dir_all(&temp_base).unwrap();
        }
        fs::create_dir_all(temp_base.join("providers")).unwrap();
        
        // We shadow openai.yaml
        let custom_provider = r#"
name: "custom-openai"
matcher: "^/v1/custom/chat/completions$"
response_template: "openai/chat.json.j2"
"#;
        fs::write(temp_base.join("providers/openai.yaml"), custom_provider).unwrap();

        let mut registry = ProviderRegistry::new();
        registry.load_from_dir(&temp_base).unwrap();

        // The disk-based "openai.yaml" should have been loaded instead of embedded "openai.yaml"
        // Let's find the provider that matches our custom matcher
        let provider = registry.find_provider("/v1/custom/chat/completions");
        assert!(provider.is_some(), "Disk provider should be matched");
        assert_eq!(provider.unwrap().name, "custom-openai");

        // ALSO check that the embedded "openai" provider is NOT loaded because it was shadowed by filename
        let embedded_openai = registry.providers.iter().find(|p| p.name == "openai");
        assert!(embedded_openai.is_none(), "Shadowed embedded provider should NOT be loaded");

        // Clean up
        fs::remove_dir_all(temp_base).unwrap();
    }

    // ─────────────────────────────────────────────────────────────────────
    // --isolated mode (issue #6) — registry-level tests
    //
    // These tests build a temp config-dir and call load_from_dir_with_options
    // both with isolated=false (the existing default) and isolated=true to
    // prove the gating works exactly where intended.
    // ─────────────────────────────────────────────────────────────────────

    fn fresh_temp_dir(name: &str) -> PathBuf {
        let dir = std::env::current_dir().unwrap().join(format!("target/{}", name));
        if dir.exists() {
            fs::remove_dir_all(&dir).unwrap();
        }
        fs::create_dir_all(dir.join("providers")).unwrap();
        dir
    }

    /// isolated=true with one disk provider: only that provider is loaded;
    /// none of the ~20 embedded providers are present.
    #[test]
    fn test_isolated_loads_only_disk_providers() {
        let temp_base = fresh_temp_dir("test_isolated_loads_only_disk");
        let only_provider = r#"
name: "lonely-tenant"
matcher: "^/lonely$"
response_body: "{\"ok\":true}"
"#;
        fs::write(temp_base.join("providers/lonely.yaml"), only_provider).unwrap();

        let mut registry = ProviderRegistry::new();
        registry.load_from_dir_with_options(&temp_base, true).unwrap();

        assert_eq!(registry.providers.len(), 1,
            "isolated mode must load exactly the one disk provider, got {}",
            registry.providers.len());
        assert_eq!(registry.providers[0].name, "lonely-tenant");

        // None of the bundled providers should be present.
        for bundled_name in &["openai", "anthropic", "gemini", "openai-responses",
                              "openai-moderations", "error-simulator"] {
            assert!(
                !registry.providers.iter().any(|p| p.name == *bundled_name),
                "isolated mode must not load embedded provider '{}'", bundled_name
            );
        }

        fs::remove_dir_all(temp_base).unwrap();
    }

    /// isolated=true with an empty config-dir: registry has zero providers
    /// (server will 404 everything; the warn log line is fired by the
    /// production code path).
    #[test]
    fn test_isolated_with_empty_dir_yields_zero_providers() {
        let temp_base = fresh_temp_dir("test_isolated_empty_dir");
        // Note: providers/ exists but is empty.

        let mut registry = ProviderRegistry::new();
        registry.load_from_dir_with_options(&temp_base, true).unwrap();

        assert_eq!(registry.providers.len(), 0,
            "isolated + empty dir must load zero providers (got {})",
            registry.providers.len());
        assert!(registry.find_provider("/v1/chat/completions").is_none(),
            "no provider should match any path");

        fs::remove_dir_all(temp_base).unwrap();
    }

    /// isolated=true: a disk provider that references a bundled template
    /// (e.g. openai/chat.json.j2) renders unsuccessfully because the
    /// template is not in the Tera registry. We assert the template is
    /// not present — actual render is exercised end-to-end in server tests.
    #[test]
    fn test_isolated_does_not_load_embedded_templates() {
        let temp_base = fresh_temp_dir("test_isolated_no_embedded_templates");
        // Provider that references a known-bundled template (won't exist in isolated mode).
        let provider_referencing_bundled = r#"
name: "borrows-bundled"
matcher: "^/borrows$"
response_template: "openai/chat.json.j2"
"#;
        fs::write(temp_base.join("providers/borrows.yaml"), provider_referencing_bundled).unwrap();

        let mut registry = ProviderRegistry::new();
        registry.load_from_dir_with_options(&temp_base, true).unwrap();

        // Provider loaded correctly...
        assert_eq!(registry.providers.len(), 1);
        // ...but Tera does NOT have the bundled template registered.
        let mut ctx = tera::Context::new();
        ctx.insert("model", &"test");
        let render = registry.tera.render("openai/chat.json.j2", &ctx);
        assert!(render.is_err(),
            "isolated mode must not load embedded templates; \
             render of bundled path should fail. Got: {:?}", render);

        fs::remove_dir_all(temp_base).unwrap();
    }

    /// Confirm the non-isolated path is unchanged: default load brings in
    /// embedded providers AND embedded templates (no regression).
    #[test]
    fn test_default_mode_still_loads_embedded() {
        let mut registry = ProviderRegistry::new();
        // Use a non-existent dir so only embedded contributes.
        registry.load_from_dir_with_options(&PathBuf::from("non_existent_dir_default_check"), false).unwrap();

        assert!(registry.providers.len() >= 15,
            "default mode should load ~20 embedded providers; got {}", registry.providers.len());
        let has_openai = registry.providers.iter().any(|p| p.name == "openai");
        let has_anthropic = registry.providers.iter().any(|p| p.name == "anthropic");
        assert!(has_openai && has_anthropic,
            "default mode must include flagship embedded providers");

        // Bundled template should be in Tera.
        let mut ctx = tera::Context::new();
        ctx.insert("model", &"test");
        ctx.insert("json", &serde_json::json!({}));
        let render = registry.tera.render("openai/chat.json.j2", &ctx);
        assert!(render.is_ok(),
            "default mode must register embedded templates");
    }

    // ─────────────────────────────────────────────────────────────────────
    // has_tool_result() regression suite — VM-009 (agentic loop termination)
    //
    // These tests exercise the Tera custom function through the exact same
    // code path that templates use. Each test constructs a request JSON,
    // renders a one-off Tera template that calls has_tool_result(), and
    // asserts on the rendered output. This covers:
    //   1. All three provider shapes (OpenAI tool role, Anthropic
    //      tool_result content block, Gemini functionResponse part).
    //   2. Array-index access — the known-flaky Tera concern. We stress
    //      messages with varying length, mixed types, empty arrays, and
    //      null/missing fields to prove the helper tolerates them all.
    //   3. Defensive behaviour: malformed input returns `false`, not an
    //      error, so template branching stays robust.
    // ─────────────────────────────────────────────────────────────────────

    /// Spin up a test Tera instance with only the helper registered, so
    /// these tests don't depend on the bundled provider templates loading.
    fn test_tera_with_helper() -> Tera {
        let mut registry = ProviderRegistry::new();
        registry.load_from_dir(&PathBuf::from("non_existent_dir_for_helper_tests")).unwrap();
        (*registry.tera).clone()
    }

    /// Convenience: render `{{ has_tool_result(messages=..., provider=...) }}`
    /// against a given messages array and provider, return the rendered string.
    fn eval_helper(messages: &serde_json::Value, provider: &str) -> String {
        let mut ctx = tera::Context::new();
        ctx.insert("messages", messages);
        ctx.insert("provider_name", provider);
        // one_off uses a fresh Tera instance without our registered helpers,
        // so we add the template to our pre-built instance and render from it.
        let mut tera = test_tera_with_helper();
        tera.add_raw_template("__helper_test__",
            r#"{{ has_tool_result(messages=messages, provider=provider_name) }}"#).unwrap();
        tera.render("__helper_test__", &ctx)
            .unwrap_or_else(|e| panic!("render failed: {e}"))
    }

    // ─── OpenAI shape ───────────────────────────────────────────────────

    #[test]
    fn test_has_tool_result_openai_detects_tool_role_message() {
        let msgs = serde_json::json!([
            {"role": "user", "content": "What is the weather?"},
            {"role": "assistant", "tool_calls": [{"id": "t1", "function": {"name": "get_weather"}}]},
            {"role": "tool", "tool_call_id": "t1", "content": "15°C cloudy"}
        ]);
        assert_eq!(eval_helper(&msgs, "openai"), "true");
    }

    #[test]
    fn test_has_tool_result_openai_no_tool_role_returns_false() {
        let msgs = serde_json::json!([
            {"role": "user", "content": "Hello"},
            {"role": "assistant", "content": "Hi!"}
        ]);
        assert_eq!(eval_helper(&msgs, "openai"), "false");
    }

    #[test]
    fn test_has_tool_result_openai_tool_role_anywhere_triggers() {
        // Tool role in position 0 should still trigger (defensive).
        let msgs = serde_json::json!([
            {"role": "tool", "tool_call_id": "t0", "content": "X"},
            {"role": "user", "content": "follow-up"}
        ]);
        assert_eq!(eval_helper(&msgs, "openai"), "true");
    }

    // ─── Anthropic shape ────────────────────────────────────────────────

    #[test]
    fn test_has_tool_result_anthropic_detects_tool_result_block() {
        let msgs = serde_json::json!([
            {"role": "user", "content": "Weather?"},
            {"role": "assistant", "content": [
                {"type": "tool_use", "id": "t1", "name": "get_weather", "input": {}}
            ]},
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "t1", "content": "15°C"}
            ]}
        ]);
        assert_eq!(eval_helper(&msgs, "anthropic"), "true");
    }

    #[test]
    fn test_has_tool_result_anthropic_string_content_no_match() {
        // Anthropic allows string content for user messages — must not falsely match.
        let msgs = serde_json::json!([
            {"role": "user", "content": "Hello"},
            {"role": "assistant", "content": "Hi"}
        ]);
        assert_eq!(eval_helper(&msgs, "anthropic"), "false");
    }

    #[test]
    fn test_has_tool_result_anthropic_assistant_with_tool_use_alone_no_match() {
        // Only assistant tool_use blocks, no user tool_result yet — loop should continue.
        let msgs = serde_json::json!([
            {"role": "user", "content": "Weather?"},
            {"role": "assistant", "content": [
                {"type": "tool_use", "id": "t1", "name": "get_weather", "input": {}}
            ]}
        ]);
        assert_eq!(eval_helper(&msgs, "anthropic"), "false");
    }

    #[test]
    fn test_has_tool_result_anthropic_mixed_content_types() {
        // Content block array with mixed types; the tool_result block must
        // be detected regardless of position.
        let msgs = serde_json::json!([
            {"role": "user", "content": [
                {"type": "text", "text": "Note:"},
                {"type": "tool_result", "tool_use_id": "t1", "content": "value"},
                {"type": "text", "text": "End."}
            ]}
        ]);
        assert_eq!(eval_helper(&msgs, "anthropic"), "true");
    }

    // ─── Gemini shape ───────────────────────────────────────────────────

    #[test]
    fn test_has_tool_result_gemini_detects_function_response_part() {
        let msgs = serde_json::json!([
            {"role": "user", "parts": [{"text": "Weather?"}]},
            {"role": "model", "parts": [{"functionCall": {"name": "get_weather", "args": {}}}]},
            {"role": "user", "parts": [{"functionResponse": {"name": "get_weather", "response": {"temp": 15}}}]}
        ]);
        assert_eq!(eval_helper(&msgs, "gemini"), "true");
    }

    #[test]
    fn test_has_tool_result_gemini_text_only_no_match() {
        let msgs = serde_json::json!([
            {"role": "user", "parts": [{"text": "Hello"}]},
            {"role": "model", "parts": [{"text": "Hi"}]}
        ]);
        assert_eq!(eval_helper(&msgs, "gemini"), "false");
    }

    #[test]
    fn test_has_tool_result_gemini_functionresponse_on_model_role_no_match() {
        // functionResponse on a non-user role doesn't count (real Gemini only
        // puts tool results in user messages). Defensive check.
        let msgs = serde_json::json!([
            {"role": "model", "parts": [{"functionResponse": {"name": "x", "response": {}}}]}
        ]);
        assert_eq!(eval_helper(&msgs, "gemini"), "false");
    }

    // ─── Tera array-index edge cases (the user's explicit ask) ──────────

    #[test]
    fn test_has_tool_result_handles_empty_messages_array() {
        assert_eq!(eval_helper(&serde_json::json!([]), "openai"), "false");
        assert_eq!(eval_helper(&serde_json::json!([]), "anthropic"), "false");
        assert_eq!(eval_helper(&serde_json::json!([]), "gemini"), "false");
    }

    #[test]
    fn test_has_tool_result_handles_very_long_messages_array() {
        // Stress test: a 200-message history with the tool result near the end.
        let mut arr: Vec<serde_json::Value> = (0..199)
            .map(|i| serde_json::json!({"role": "user", "content": format!("msg {i}")}))
            .collect();
        arr.push(serde_json::json!({"role": "tool", "content": "final tool result"}));
        assert_eq!(eval_helper(&serde_json::json!(arr), "openai"), "true");
    }

    #[test]
    fn test_has_tool_result_handles_non_array_input() {
        // Tera templates might pass scalar/null when json.messages is absent.
        // Should return false, not error.
        let mut ctx = tera::Context::new();
        ctx.insert("messages", &serde_json::Value::Null);
        ctx.insert("provider_name", &"openai");
        let mut tera = test_tera_with_helper();
        tera.add_raw_template("__null_msgs_test__",
            "{{ has_tool_result(messages=messages, provider=provider_name) }}").unwrap();
        let rendered = tera.render("__null_msgs_test__", &ctx)
            .unwrap_or_else(|e| panic!("should not error on null input: {e}"));
        assert_eq!(rendered, "false");
    }

    #[test]
    fn test_has_tool_result_handles_malformed_messages() {
        // Each element is something other than an object — helper must
        // tolerate without panicking and return false.
        let msgs = serde_json::json!([
            "string element",
            42,
            null,
            ["nested", "array"]
        ]);
        assert_eq!(eval_helper(&msgs, "openai"), "false");
    }

    #[test]
    fn test_has_tool_result_handles_missing_role_field() {
        // Objects without a "role" field at all should silently fail the
        // role check (not raise).
        let msgs = serde_json::json!([
            {"content": "no role here"},
            {"parts": [{"text": "also no role"}]}
        ]);
        assert_eq!(eval_helper(&msgs, "openai"), "false");
        assert_eq!(eval_helper(&msgs, "anthropic"), "false");
        assert_eq!(eval_helper(&msgs, "gemini"), "false");
    }

    #[test]
    fn test_has_tool_result_handles_deeply_nested_tool_result() {
        // Simulate a real Anthropic tool_result with a nested content array
        // (the content field of a tool_result can itself be a list of
        // blocks). Helper must still detect the tool_result by type.
        let msgs = serde_json::json!([
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "t1", "content": [
                    {"type": "text", "text": "nested"},
                    {"type": "image", "source": {"type": "base64", "data": "AAA"}}
                ]}
            ]}
        ]);
        assert_eq!(eval_helper(&msgs, "anthropic"), "true");
    }

    #[test]
    fn test_has_tool_result_unknown_provider_returns_false() {
        // Defensive: unknown provider name returns false rather than erroring.
        let msgs = serde_json::json!([{"role": "tool", "content": "x"}]);
        assert_eq!(eval_helper(&msgs, "martian_ai"), "false");
    }

    #[test]
    fn test_has_tool_result_defaults_to_openai_when_provider_omitted() {
        // If the template author omits `provider=...`, default to openai.
        let mut ctx = tera::Context::new();
        ctx.insert("messages", &serde_json::json!([{"role": "tool", "content": "x"}]));
        let mut tera = test_tera_with_helper();
        tera.add_raw_template("__default_provider_test__",
            "{{ has_tool_result(messages=messages) }}").unwrap();
        let rendered = tera.render("__default_provider_test__", &ctx).unwrap();
        assert_eq!(rendered, "true");
    }
}
