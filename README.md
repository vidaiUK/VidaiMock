# VidaiMock

[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.70%2B-orange.svg)](https://www.rust-lang.org/)

[Home Page](https://Vidai.uk) | [Documentation](https://vidai.uk/docs/mock/intro/)

**Batteries-included mock server for LLM APIs** — works instantly with OpenAI, Anthropic, Gemini, Bedrock, and more. Zero config required.

## ⚡ 30-Second Demo

```bash
# Download and bundle (macOS Apple Silicon)
curl -LO https://github.com/vidaiUK/VidaiMock/releases/latest/download/vidaimock-macos-arm64.tar.gz
tar -xzf vidaimock-macos-arm64.tar.gz && cd vidaimock

# Run and enjoy!
./vidaimock

# (In another terminal) Test it!
curl -N http://localhost:8100/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model": "gpt-4", "stream": true, "messages": [{"role": "user", "content": "Hello!"}]}'
```

Watch tokens appear one by one — that's realistic LLM simulation.

## 🔋 Batteries Included

No configuration needed. These providers work immediately:

| Provider | Endpoint | Streaming |
|----------|----------|-----------|
| **OpenAI Chat** | `/v1/chat/completions` | ✅ |
| **OpenAI Responses** | `/v1/responses` | ✅ (typed SSE events) |
| **OpenAI Embeddings** | `/v1/embeddings` | — |
| **OpenAI Images** | `/v1/images/generations` | — |
| **OpenAI Moderations** | `/v1/moderations` | — |
| **Anthropic** | `/v1/messages` | ✅ (all 7 SSE event types) |
| **Gemini Generate** | `/v1beta/models/*:generateContent` | ✅ |
| **Gemini Embeddings** | `/v1beta/models/*:embedContent` | — |
| **Gemini Token Count** | `/v1beta/models/*:countTokens` | — |
| **Gemini Models** | `GET /v1beta/models` | — |
| **Gemini OpenAI Shim** | `/v1beta/openai/*` | ✅ |
| **Azure OpenAI** | `/openai/deployments/*` | ✅ |
| **Bedrock** | `/model/*/invoke` | ✅ |
| **Cohere, Mistral, Groq** | OpenAI-compatible | ✅ |
| **Error Simulator** | `/error/{code}` | — |

Plus: Tool calling (OpenAI `tool_calls` + Anthropic `tool_use` + Gemini `functionCall`), reasoning model tokens, Gemini 2.5 `thoughtsTokenCount`, Anthropic cache/cost fields, and more.

## ✨ Key Features

- **🚀 Zero Config / Zero Fixtures**: Single **~7MB binary**, instant startup, no Docker/DB, and zero setup required.
- **🌊 Physics-Accurate Streaming**: Realistic TTFT and token-by-token delivery with **provider-native streaming payloads** (OpenAI SSE, Responses API typed events, Anthropic EventStream, Gemini, etc.)
- **⚡ High Performance**: 50,000+ RPS in benchmark mode
- **🎛️ Chaos & Error Testing**: Inject failures, latency, malformed responses, and **custom HTTP status codes** (400, 401, 404, 429, 500, etc.) for error path testing
- **🏢 Multi-Tenant**: Isolate teams or CI pipelines on one mock instance with per-tenant latency/chaos overrides and optional PSK authentication
- **🧠 Smart Response Branching**: Templates auto-detect tool calls (OpenAI `tool_calls`, Anthropic `tool_use`, Gemini `functionCall`), reasoning models (o-series), structured output, and respond with the correct shape
- **🎯 Per-Request Overrides**: `X-Mock-Status` header returns any HTTP status on any endpoint — test error paths on real provider routes without path rewriting
- **📝 Customizable**: YAML configs + Tera templates for any API

## 🛡️ Built for Vidai.Server

VidaiMock is the official development environment for [Vidai.Server](https://vidai.uk)—the **High-Density Enterprise AI Gateway**. 

The same logic that powers VidaiMock's simulation of network jitter, latency, and failure modes is used in production to provide sovereign control planes for enterprise LLM infrastructure.

### 🌊 More than a Mock
Unlike tools that just record and replay static data or intercept browser requests, **VidaiMock is a standalone Simulation Engine**. It emulates the exact wire-format and per-token timing of LLM streaming payloads, making it the perfect tool for testing streaming UI/UX and SDK resilience.

*   **Truly Dynamic**: Every response is a Tera template. You can reflect request data, generate random IDs, or use complex logic to make your mock feel alive.
*   **Physics-Accurate**: Emulates real-world network protocols (SSE, EventStream) and silver-level latency.
*   **Error Path Testing**: Custom HTTP status codes via `status_code` in YAML (static or dynamic) and `X-Mock-Status` request header let you test upstream error handling — 400s, 401s, 404s, 429s, 500s — on any real provider endpoint without path rewriting.
*   **Smart Branching**: Templates auto-detect OpenAI `tools`/`response_format`/o-series models, Anthropic `tools`, and Gemini `functionDeclarations` from the request and return the correctly shaped response — no per-scenario config needed.
*   **Typed SSE Streaming**: Beyond plain `data:` chunks — supports OpenAI Responses API typed events (`response.output_text.delta`, etc.), Anthropic's 7-event lifecycle (`content_block_start`, `message_delta`, `ping`, etc.), and `stream_options.include_usage` for final usage chunks.

## 📂 Project Structure

- `bin/`: The VidaiMock executable
- `config/`: Default provider YAMLs and J2 templates
- `examples/`: 20+ advanced templates (RAG, Tool calling, Fuzzing, etc.)
- `scripts/`: Diagnostic and verification helpers

## 📦 Installation

**Download Bundled Release** (Recommended):
Releases come bundled with the binary, default providers, templates, and usage examples.

```bash
# macOS Apple Silicon
curl -LO https://github.com/vidaiUK/VidaiMock/releases/latest/download/vidaimock-macos-arm64.tar.gz
tar -xzf vidaimock-macos-arm64.tar.gz && cd vidaimock

# macOS Intel
curl -LO https://github.com/vidaiUK/VidaiMock/releases/latest/download/vidaimock-macos-x64.tar.gz
tar -xzf vidaimock-macos-x64.tar.gz && cd vidaimock

# Linux ARM64
curl -LO https://github.com/vidaiUK/VidaiMock/releases/latest/download/vidaimock-linux-arm64.tar.gz
tar -xzf vidaimock-linux-arm64.tar.gz && cd vidaimock

# Linux x64
curl -LO https://github.com/vidaiUK/VidaiMock/releases/latest/download/vidaimock-linux-x64.tar.gz
tar -xzf vidaimock-linux-x64.tar.gz && cd vidaimock

# Windows x64 (PowerShell)
Invoke-WebRequest -Uri https://github.com/vidaiUK/VidaiMock/releases/latest/download/vidaimock-windows-x64.zip -OutFile vidaimock-windows-x64.zip
Expand-Archive vidaimock-windows-x64.zip -DestinationPath .
cd vidaimock

./vidaimock
```

### 🔐 Security Notice (macOS/Windows)
Since VidaiMock is an open-source project, your OS may show a security warning:

*   **macOS**: Run `xattr -d com.apple.quarantine vidaimock` in your terminal to allow the binary to run.
*   **Windows**: Click "More info" in the SmartScreen popup and select "Run anyway".

**Build from source**:
```bash
git clone https://github.com/vidaiUK/VidaiMock.git
cd vidaimock && cargo build --release
./target/release/vidaimock
```

## 🎮 Quick Examples

```bash
# OpenAI chat completion
curl http://localhost:8100/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model": "gpt-4", "messages": [{"role": "user", "content": "Hi"}]}'

# Tool calling — auto-detects tools and returns tool_calls response
curl http://localhost:8100/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model": "gpt-4o", "messages": [{"role": "user", "content": "Weather?"}], "tools": [{"type": "function", "function": {"name": "get_weather", "parameters": {}}}]}'

# Reasoning models — returns reasoning_tokens in usage
curl http://localhost:8100/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model": "o4-mini", "messages": [{"role": "user", "content": "2+2"}]}'

# OpenAI Responses API (non-streaming)
curl http://localhost:8100/v1/responses \
  -H "Content-Type: application/json" \
  -d '{"model": "gpt-4o-mini", "input": "Say hello", "max_output_tokens": 50}'

# OpenAI Responses API (streaming with typed SSE events)
curl -N http://localhost:8100/v1/responses \
  -H "Content-Type: application/json" \
  -d '{"model": "gpt-4o-mini", "input": "Say hello", "stream": true}'

# Streaming with usage reporting
curl -N http://localhost:8100/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model": "gpt-4o", "stream": true, "stream_options": {"include_usage": true}, "messages": [{"role": "user", "content": "Hi"}]}'

# Embeddings, images, moderations
curl http://localhost:8100/v1/embeddings -H "Content-Type: application/json" \
  -d '{"model": "text-embedding-3-small", "input": "Hello"}'
curl http://localhost:8100/v1/images/generations -H "Content-Type: application/json" \
  -d '{"model": "dall-e-2", "prompt": "a red circle", "n": 1}'
curl http://localhost:8100/v1/moderations -H "Content-Type: application/json" \
  -d '{"model": "omni-moderation-latest", "input": "Hello"}'

# Gemini generateContent
curl http://localhost:8100/v1beta/models/gemini-2.5-flash:generateContent \
  -H "Content-Type: application/json" \
  -d '{"contents": [{"role": "user", "parts": [{"text": "Hello"}]}]}'

# Gemini tool calling (returns functionCall)
curl http://localhost:8100/v1beta/models/gemini-2.5-flash:generateContent \
  -H "Content-Type: application/json" \
  -d '{"contents": [{"role": "user", "parts": [{"text": "Weather?"}]}], "tools": [{"functionDeclarations": [{"name": "get_weather", "parameters": {"type": "OBJECT", "properties": {"city": {"type": "STRING"}}}}]}]}'

# Gemini embedContent, countTokens, model listing
curl http://localhost:8100/v1beta/models/gemini-embedding-001:embedContent \
  -H "Content-Type: application/json" -d '{"content": {"parts": [{"text": "Hello"}]}}'
curl http://localhost:8100/v1beta/models/gemini-2.5-flash:countTokens \
  -H "Content-Type: application/json" -d '{"contents": [{"role": "user", "parts": [{"text": "Hello"}]}]}'
curl http://localhost:8100/v1beta/models

# Error simulation — any HTTP status code, provider-agnostic
curl http://localhost:8100/error/400 -H "Content-Type: application/json" -d '{}'
curl http://localhost:8100/error/429 -H "Content-Type: application/json" -d '{}'

# X-Mock-Status header — force any HTTP status on any real endpoint
# Returns HTTP 429 with the normal response body for error passthrough testing
curl -H "X-Mock-Status: 429" http://localhost:8100/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model": "gpt-4", "messages": [{"role": "user", "content": "Hi"}]}'

# Anthropic messages
curl http://localhost:8100/v1/messages \
  -H "Content-Type: application/json" \
  -d '{"model": "claude-haiku-4-5-20251001", "max_tokens": 200, "messages": [{"role": "user", "content": "Hi"}]}'

# Anthropic tool calling (returns tool_use block)
curl http://localhost:8100/v1/messages \
  -H "Content-Type: application/json" \
  -d '{"model": "claude-haiku-4-5-20251001", "max_tokens": 500, "messages": [{"role": "user", "content": "Weather in London?"}], "tools": [{"name": "get_weather", "description": "Get weather", "input_schema": {"type": "object", "properties": {"city": {"type": "string"}}}}]}'

# Anthropic streaming (all 7 event types)
curl -N http://localhost:8100/v1/messages \
  -H "Content-Type: application/json" \
  -d '{"model": "claude-haiku-4-5-20251001", "max_tokens": 200, "stream": true, "messages": [{"role": "user", "content": "Count to 5"}]}'

# With latency simulation
./vidaimock --latency 500 --mode realistic

# Force chaos errors (test retry logic)
curl -H "X-Vidai-Chaos-Drop: 100" http://localhost:8100/v1/chat/completions \
  -H "Content-Type: application/json" -d '{"model": "gpt-4", "messages": [{"role": "user", "content": "Hi"}]}'
```

## 📚 Documentation

The documentation for VidaiMock is available at our [Documentation Site](https://vidai.uk/docs/mock/intro/). 

For more information about Vidai, visit our [Home Page](https://Vidai.uk).

## 🛠️ CLI Reference

```
Usage: vidaimock [OPTIONS]

Options:
  --host <HOST>              Bind address [default: 0.0.0.0]
  -p, --port <PORT>          Listen port [default: 8100]
  -w, --workers <N>          Worker threads [default: num cpus]
  --config <FILE>            Config file path [default: mock-server.toml]
  --config-dir <DIR>         Custom provider configs directory (overlays bundled)
  --latency <MS>             Base response delay in milliseconds
  --mode <MODE>              benchmark | realistic | debug
  --endpoints <PATHS>        Comma-separated endpoints to serve (overrides config)
  --format <FORMAT>          Response format: openai, anthropic, gemini, etc.
  --response-file <FILE>     Custom response file for default endpoints
  --content-type <TYPE>      Override Content-Type header
  -h, --help                 Print help
  -V, --version              Print version
```

### Runtime Headers

Any endpoint accepts these headers to override behavior per-request:

| Header | Effect |
|--------|--------|
| `X-Mock-Status: <code>` | Return this HTTP status (e.g. `429`, `500`) instead of 200 |
| `X-Vidai-Latency: <ms>` | Override base latency for this request |
| `X-Vidai-Jitter: <pct>` | Override latency jitter percentage |
| `X-Vidai-Chaos-Drop: <pct>` | Probability of simulated 500 |
| `X-Vidai-Chaos-Malformed: <pct>` | Probability of malformed JSON response |
| `X-Vidai-Chaos-Trickle: <ms>` | Per-chunk delay during streaming |
| `X-Vidai-Chaos-Disconnect: <pct>` | Probability of mid-stream disconnect |

## 🎯 Provider Config Reference

Provider YAML files in `config/providers/` define how endpoints match and respond:

```yaml
name: "my-provider"
matcher: "^/v1/my/endpoint$"         # Regex path match
response_template: "my/template.j2"  # Tera template path
status_code: "200"                   # HTTP status (static or Tera expression)
priority: 10                         # Higher matches first
stream:
  enabled: true
  frame_format: raw                  # "raw" = template controls SSE framing
  lifecycle:
    on_start:
      template_path: "my/stream_start.j2"
    on_chunk:
      template_path: "my/stream_delta.j2"
    on_stop:
      template_path: "my/stream_stop.j2"
```

**`status_code`** accepts static values (`"400"`) or Tera expressions (`"{{ path_segments | last }}"`) for dynamic HTTP status codes. The bundled `/error/{code}` endpoint uses this to simulate any error.

**`frame_format: raw`** gives the template full control over SSE framing — essential for providers like OpenAI's Responses API that use typed `event:` lines.

## 🏢 Multi-Tenant Support

A single VidaiMock instance can serve multiple isolated teams or CI pipelines simultaneously. Each **tenant** gets its own identity, optional authentication, and independent latency/chaos settings that fully override the global defaults.

### How It Works

Every incoming request is inspected by the `extract_tenant` middleware before it reaches a handler:

1. If there is no `X-Tenant-ID` header the request is treated as **anonymous** and the global `[latency]` / `[chaos]` settings apply.
2. If the header is present but matches no configured tenant, the request falls through to the anonymous defaults (the mock stays open by design).
3. If the tenant is found and has **no** `api_key` configured, the request is accepted on name alone (low-friction dev / CI mode).
4. If the tenant has an `api_key`, the caller must supply a matching `X-Tenant-Key` header — a mismatch returns **HTTP 401** without leaking which tenant was requested.

### Configuration (`mock-server.toml`)

Add one `[[tenants]]` block per tenant. No restart is needed beyond editing the file and restarting the process (or sending `SIGHUP` if your OS supports it).

```toml
# ── Tenant: Team A ──────────────────────────────────────────────────────────
[[tenants]]
id = "team-a"
api_key = "sk-mock-team-a-abc123"   # omit to accept any request for this ID

[tenants.latency]
mode = "realistic"
base_ms = 200
jitter_pct = 0.1

[tenants.chaos]
drop_pct = 5.0        # 5 % of requests return HTTP 500
malformed_pct = 2.0   # 2 % of responses are deliberately malformed JSON

# ── Tenant: CI Pipeline (no auth, zero latency) ──────────────────────────────
[[tenants]]
id = "ci-pipeline"
# No api_key — any caller claiming this ID is accepted

[tenants.latency]
mode = "benchmark"
base_ms = 0
jitter_pct = 0.0
```

#### Tenant lifecycle

| Action | How |
|--------|-----|
| **Create** | Add a `[[tenants]]` block and restart |
| **Modify** | Edit the block and restart |
| **Delete** | Remove the block and restart |

### Runtime Headers

| Header | Purpose |
|--------|---------|
| `X-Tenant-ID: <id>` | Identify the tenant for this request |
| `X-Tenant-Key: <key>` | Authenticate (required only when the tenant has `api_key` set) |

Per-tenant `X-Vidai-*` chaos/latency overrides (see [Runtime Headers](#runtime-headers-1) above) can still be applied on top of tenant-level settings.

### Per-Tenant Overrides

The `[tenants.latency]` and `[tenants.chaos]` blocks accept exactly the same fields as the top-level `[latency]` and `[chaos]` sections. When a tenant block is present it completely replaces the global setting for that tenant; when it is absent the global setting is used.

```toml
[tenants.chaos]
drop_pct        = 10.0   # % of requests that return HTTP 500
malformed_pct   = 5.0    # % of responses with deliberately broken JSON
trickle_ms      = 50     # extra per-chunk delay during streaming (ms)
disconnect_pct  = 3.0    # % of streaming responses that cut off mid-stream
```

### Secrets via Environment Variables

`api_key` values can be supplied through environment variables instead of plain text in the TOML file:

```bash
# Index corresponds to the position of the [[tenants]] block (0-based)
export VIDAIMOCK_TENANTS__0__API_KEY=sk-mock-team-a-abc123
export VIDAIMOCK_TENANTS__1__API_KEY=sk-mock-team-b-xyz789
```

### `/status` and Security

The `api_key` field is **never** serialized to JSON. Calling `GET /status` returns tenant IDs and their latency/chaos settings but never exposes any secret keys.

### Observability

#### Prometheus Metrics (`GET /metrics`)

Every request is counted and timed with a `tenant` label:

| Metric | Type | Labels |
|--------|------|--------|
| `http_requests_total` | Counter | `method`, `path`, `status`, **`tenant`** |
| `http_request_duration_seconds` | Histogram | `method`, `path`, **`tenant`** |

The `tenant` label is set to the raw value of the `X-Tenant-ID` header, or `"anonymous"` when the header is absent. This lets you build per-team dashboards and alerts directly in Prometheus/Grafana without any additional instrumentation.

```promql
# Total requests per tenant over the last 5 minutes
sum by (tenant) (rate(http_requests_total[5m]))

# 95th-percentile latency per tenant
histogram_quantile(0.95, sum by (tenant, le) (rate(http_request_duration_seconds_bucket[5m])))
```

#### Structured Logs / Tracing

Every request span is emitted with a `tenant_id` field (value: the resolved tenant ID or `"anonymous"`). When running with a structured-log backend (e.g. `RUST_LOG=info`) all log lines for a request automatically carry this field:

```
INFO request{tenant_id="team-a"}: mock response sent in 23ms
INFO request{tenant_id="anonymous"}: mock response sent in 1ms
```

This makes per-tenant log filtering trivial with any log aggregation tool (Datadog, Loki, CloudWatch, etc.).

### Quick Example

```bash
# Start mock with tenant config in mock-server.toml
./vidaimock

# Request as Team A (with auth)
curl http://localhost:8100/v1/chat/completions \
  -H "Content-Type: application/json" \
  -H "X-Tenant-ID: team-a" \
  -H "X-Tenant-Key: sk-mock-team-a-abc123" \
  -d '{"model": "gpt-4", "messages": [{"role": "user", "content": "Hi"}]}'

# Request as CI pipeline (no auth required)
curl http://localhost:8100/v1/chat/completions \
  -H "Content-Type: application/json" \
  -H "X-Tenant-ID: ci-pipeline" \
  -d '{"model": "gpt-4", "messages": [{"role": "user", "content": "Hi"}]}'

# Wrong key → 401
curl http://localhost:8100/v1/chat/completions \
  -H "Content-Type: application/json" \
  -H "X-Tenant-ID: team-a" \
  -H "X-Tenant-Key: wrong-key" \
  -d '{"model": "gpt-4", "messages": [{"role": "user", "content": "Hi"}]}'
```

---

## 📄 License

Apache 2.0 — See [LICENSE](LICENSE).

---

### 🌐 Looking for Centralized Test Infrastructure?

VidaiMock runs locally, but we offer a managed control plane for enterprise teams. 

**[Get Started with Vidai Managed](https://vidai.uk)**

---

## 💜 Acknowledgments

VidaiMock is built on the shoulders of giants in the Rust ecosystem:
- [Axum](https://github.com/tokio-rs/axum) & [Tokio](https://github.com/tokio-rs/tokio) for the high-performance async foundation.
- [Tera](https://github.com/Keats/tera) for the flexible templating engine.
- [rust-embed](https://github.com/pyrossh/rust-embed) for the zero-config binary magic.
- [Mimalloc](https://github.com/microsoft/mimalloc) for the lightning-fast memory allocation.

---

## 👥 Contributors

A special thanks to everyone who helps make VidaiMock better!

| Contributor | Highlights |
| :--- | :--- |
| [<img src="https://github.com/NiltonVolpato.png?size=64" width="64" alt="NiltonVolpato"/><br/>**NiltonVolpato**](https://github.com/NiltonVolpato) | 🛠️ Improvements to listener address reporting and OS-assigned port support. |
| [<img src="https://github.com/bbRLdev.png?size=64" width="64" alt="bbRLdev"/><br/>**bbRLdev**](https://github.com/bbRLdev) | 🌊 Improvements to OpenAI streaming logic and termination events. |
| [<img src="https://github.com/nagug.png?size=64" width="64" alt="nagug"/><br/>**nagug**](https://github.com/nagug) | 🚀 Core architecture, high-density engine design, and project maintainer. |

---

Built with ❤️ by [Vidai](https://vidai.uk) from Scotland 🏴󠁧󠁢󠁳󠁣󠁴󠁿
