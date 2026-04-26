# VidaiMock

[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.70%2B-orange.svg)](https://www.rust-lang.org/)

[Home Page](https://Vidai.uk) | [Documentation](https://vidai.uk/docs/mock/intro/)

**Batteries-included mock server for LLM APIs and agents** — works instantly with OpenAI, Anthropic, Gemini, Bedrock, and more. Run ADK / LangGraph / LangChain agentic workflows against it without a single live-provider token. Zero config required.

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
| **Gemini Generate** | `/v1beta/models/*:generateContent` | ✅ (text deltas + terminal `finishReason: STOP` chunk with `usageMetadata`, no `[DONE]`) |
| **Gemini Embeddings** | `/v1beta/models/*:embedContent` | — |
| **Gemini Token Count** | `/v1beta/models/*:countTokens` | — |
| **Gemini Models** | `GET /v1beta/models` | — |
| **Gemini OpenAI Shim** | `/v1beta/openai/*` | ✅ |
| **Azure OpenAI** | `/openai/deployments/*` | ✅ |
| **Bedrock** | `/model/*/invoke` | ✅ |
| **Cohere, Mistral, Groq** | OpenAI-compatible | ✅ |
| **Error Simulator** | `/error/{code}` | — |

Plus: Tool calling (OpenAI `tool_calls` + Anthropic `tool_use` + Gemini `functionCall`), **agentic loop termination** (tool-result detection across all three providers — see [Agentic Workflow Testing](#-agentic-workflow-testing)), reasoning model tokens, Gemini 2.5 `thoughtsTokenCount`, Anthropic cache/cost fields, and more.

## ✨ Key Features

- **🚀 Zero Config / Zero Fixtures**: Single **~7MB binary**, instant startup, no Docker/DB, and zero setup required in default single mode.
- **🌊 Physics-Accurate Streaming**: Realistic TTFT and token-by-token delivery with **provider-native streaming payloads** (OpenAI SSE, Responses API typed events, Anthropic EventStream, Gemini, etc.)
- **⚡ High Performance**: 50,000+ RPS in benchmark mode
- **🎛️ Chaos & Error Testing**: Inject failures, latency, malformed responses, and **custom HTTP status codes** (400, 401, 404, 429, 500, etc.) — every error returns a **provider-shaped JSON envelope** (OpenAI, Anthropic, Gemini)
- **🧠 Smart Response Branching**: Templates auto-detect tool calls (OpenAI `tool_calls`, Anthropic `tool_use`, Gemini `functionCall`), reasoning models (o-series), structured output, and respond with the correct shape
- **🔁 Agentic Loop Termination**: When a tool result is already in the request history (OpenAI `role: tool`, Anthropic `tool_result` block, Gemini `functionResponse` part), the mock switches to plain-text synthesis instead of looping another `tool_call` — ADK/LangGraph/LangChain agentic runs terminate naturally
- **🎯 Per-Request Overrides**: `X-Mock-Status` header, `?chaos_status=500` URL query, and `X-Vidai-Chaos-*` headers all return real provider error envelopes — test error paths on real provider routes without path rewriting
- **✅ Request Validation**: Known-required fields are enforced per provider (e.g. Anthropic `/v1/messages` without `max_tokens` → HTTP 400 with correct `invalid_request_error` envelope and a per-field message like `max_tokens: Field required`)
- **🔬 SDK-Level Wire Accuracy**: Streams survive strict SDK parsers end-to-end — `openai-python`, `anthropic`, `google-genai` all iterate the mock without hand-crafted compat shims. Text streaming, tool-call streaming, and agentic-loop streaming all emit single-line SSE JSON with correct typed events. Regression-tested byte-level against captured real-provider wire format.
- **📝 Customizable**: YAML configs + Tera templates for any API

## 🛡️ Built for Vidai.Server

VidaiMock is the official development environment for [Vidai.Server](https://vidai.uk)—the **High-Density Enterprise AI Gateway**. 

The same logic that powers VidaiMock's simulation of network jitter, latency, and failure modes is used in production to provide sovereign control planes for enterprise LLM infrastructure.

### 🌊 More than a Mock
Unlike tools that just record and replay static data or intercept browser requests, **VidaiMock is a standalone Simulation Engine**. It emulates the exact wire-format and per-token timing of LLM streaming payloads, making it the perfect tool for testing streaming UI/UX and SDK resilience.

*   **Truly Dynamic**: Every response is a Tera template. You can reflect request data, generate random IDs, or use complex logic to make your mock feel alive.
*   **Physics-Accurate**: Emulates real-world network protocols (SSE, EventStream) and silver-level latency.
*   **Error Path Testing**: Custom HTTP status codes via `status_code` in YAML (static or dynamic) and `X-Mock-Status` request header let you test upstream error handling — 400s, 401s, 404s, 429s, 500s — on any real provider endpoint without path rewriting.
*   **Smart Branching**: Templates auto-detect OpenAI `tools`/`response_format`/o-series models, Anthropic `tools`, Gemini `functionDeclarations`, and tool-result presence in the message history — so agentic testing against ADK, LangGraph, and LangChain Runner loops terminates correctly instead of calling the mock forever.
*   **Typed SSE Streaming**: Beyond plain `data:` chunks — supports OpenAI Responses API typed events (`response.output_text.delta`, etc.), Anthropic's 7-event lifecycle (`content_block_start`, `message_delta`, `ping`, etc.), Gemini's "text-delta chunks + terminal `finishReason` chunk" pattern, and `stream_options.include_usage` for final usage chunks.

### 🤖 Agentic Workflow Testing

Agent frameworks wrap an LLM in a tool-calling loop: **model → tool_call → tool executes → tool_result → model → …**. The loop terminates when the model stops requesting tools and produces a plain-text answer. Naïve mocks can't replicate this — they either always return tool calls (infinite loop) or never return them (breaks tool tests). VidaiMock's bundled chat templates do both, correctly:

- **Tools defined + no tool result yet** → emit a `tool_call` / `tool_use` / `functionCall`
- **Tools defined + tool result already in history** → emit plain-text synthesis with `finish_reason: "stop"` / `stop_reason: "end_turn"`

The heuristic is a built-in Tera helper, `has_tool_result()`, that inspects the request's conversation history for:

| Provider | Signal |
|---|---|
| OpenAI | message with `role: "tool"` |
| Anthropic | user message whose `content[]` contains `type: "tool_result"` |
| Gemini | user content whose `parts[]` contains `functionResponse` |

This means you can run **Google ADK, LangGraph, or LangChain Runner loops end-to-end in CI against VidaiMock with zero live-provider spend** — the loop terminates naturally just like it does against real providers. Same heuristic works with custom provider templates; call `has_tool_result(messages=json.messages, provider="openai")` in your own `.j2` files.

Concrete example — the full OpenAI round trip, no API key, no cost:

```bash
# Turn 1: user asks a question; mock returns a tool_call (because tools are defined).
curl -s http://localhost:8100/v1/chat/completions -H 'Content-Type: application/json' \
  -d '{"model":"gpt-4o","tools":[{"type":"function","function":{"name":"get_weather","parameters":{}}}],
       "messages":[{"role":"user","content":"Weather in London?"}]}'
# -> finish_reason: "tool_calls", message.tool_calls: [...]

# Your agent executes the tool, appends the result, calls again.
# Turn 2: same tools, now with a role:tool result in history.
# Mock detects the tool result, returns plain-text synthesis instead of looping.
curl -s http://localhost:8100/v1/chat/completions -H 'Content-Type: application/json' \
  -d '{"model":"gpt-4o","tools":[{"type":"function","function":{"name":"get_weather","parameters":{}}}],
       "messages":[
         {"role":"user","content":"Weather in London?"},
         {"role":"assistant","tool_calls":[{"id":"c1","type":"function","function":{"name":"get_weather","arguments":"{}"}}]},
         {"role":"tool","tool_call_id":"c1","content":"15°C cloudy"}
       ]}'
# -> finish_reason: "stop", message.content: "Based on the tool results..."
```

## 📂 Project Structure

- `bin/`: The VidaiMock executable
- `config/`: Legacy single-mode provider YAMLs and templates
- `tenants/`: Multi-mode tenant overlays when tenancy mode is `multi`
- `examples/`: 20+ advanced templates (RAG, Tool calling, Fuzzing, etc.)
- `scripts/`: Diagnostic and verification helpers

## 🏢 Single Mode vs Multi Mode

VidaiMock stays a small stateless runtime with no database requirement. Tenancy is explicit and configured in `mock-server.toml`:

- `single`: backward-compatible mode that uses `config/`
- `multi`: tenant-aware mode that uses `tenants/`

Single mode is still the simplest path: run the binary, keep your YAML and templates under `config/`, and VidaiMock behaves exactly like the legacy layout.

Multi mode keeps one shared runtime but resolves each accepted request to a tenant before provider matching. Isolation here is logical workspace isolation inside one process, not OS-level or process-level isolation.

In multi mode, `mock-server.toml` keeps the global runtime settings (`mode`, `tenants_dir`, `tenant_header`, global admin auth). Named tenant metadata lives with the tenant itself in `tenants/<id>/tenant.toml`.

Tenant-owned policy can live there too. A tenant can override the global `latency` and `chaos` defaults in its own `tenant.toml`, and accepted requests use the resolved tenant policy before applying any per-request `X-Vidai-*` header overrides.

Tenant metadata can live there as well. Safe fields such as `display_name` and `labels` are exposed to templates as `tenant.display_name` and `tenant.labels`, while tenant auth material stays out of template context.

```toml
# tenants/acme/tenant.toml
id = "acme"
display_name = "Acme Corp"

[labels]
tier = "gold"
region = "eu-west"
```

```json
{"tenant_id":"{{ tenant.id }}","tenant_name":"{{ tenant.display_name }}","tenant_region":"{{ tenant.labels.region }}"}
```

## 🗂️ Directory Layout

Single mode uses the legacy layout:

```text
config/
  providers/
  templates/
```

Multi mode uses a tenant workspace layout:

```text
tenants/
  default/
    # tenant.toml optional: only needed for explicit default-tenant metadata
    providers/
    templates/
  acme/
    tenant.toml
    providers/
    templates/
  globex/
    tenant.toml
    providers/
    templates/
```

Built-ins are always the base layer. The effective runtime is built-ins plus that tenant's own overrides.

- The default tenant always exists internally.
- `tenants/default/` is only required when you want to override the built-in default tenant behavior.
- `tenants/default/tenant.toml` is optional and is only needed when the default tenant has explicit metadata of its own.
- Tenant-owned `latency` and `chaos` settings override the global defaults for that tenant only.
- Named tenants do not inherit from each other.
- Named tenants do not inherit from the default tenant.
- The default tenant is fallback-only when no tenant signal is provided.

What is isolated per tenant:
- Providers, templates, and model listing
- Tenant metadata exposed to templates
- Tenant request keys and tenant-admin management auth
- Tenant latency and chaos defaults

What remains global:
- The HTTP server process and streaming engine
- Built-in base assets
- Metrics/export pipeline and reload coordination
- Global `/admin/*` operations

## 🔀 Tenant Resolution

In multi mode, a request can resolve a tenant in three ways:

- Header only
- Key only
- Header plus key

Resolution rules:

- Header-only requests are allowed only when that tenant does not require a key.
- Key-only requests are allowed when the key uniquely resolves a tenant.
- Header plus key must resolve to the same tenant.
- No header and no key falls back to the internal default tenant.
- Unknown tenant, unknown key, or header/key conflict is rejected.
- In shared multi-tenant mode, the public rejection response is intentionally generic. Internal metrics still keep structured rejection reasons such as unknown tenant, unknown key, missing key, and conflict.
- Tenant key sources supported in config are `header` and `query`; `host` and `path` are rejected during validation.
- Accepted requests use tenant-labelled metrics.
- Rejected requests use separate rejection metrics.
- Request duration metrics measure handler time until the response object is returned; for streaming routes they are closer to response setup / TTFB than full stream drain time.

## 🔐 Management Endpoints

Global admin endpoints live under `/admin/` and use dedicated admin auth configured under `tenancy.admin_auth`.

- `GET /admin/tenants`
- `GET /admin/tenants/{id}`
- `POST /admin/reload`

If `tenancy.admin_auth` is unset, `/admin/*` stays closed.

Tenant self-management lives under `/tenant/` and uses tenant-admin auth, not global admin auth.

- `GET /tenant`
- `POST /tenant/reload`

Normal tenant request keys stay on the mock traffic path only. Tenant self-management uses a separate tenant-local `management_auth` secret in `tenants/<id>/tenant.toml`, with `value_file` preferred over `value_env`, and `value_env` preferred over inline `value`.

Relative `value_file` paths resolve from the owning config file:
- `mock-server.toml` paths resolve relative to that file's directory
- `tenants/<id>/tenant.toml` paths resolve relative to that tenant metadata file's directory

By default the tenant-admin header is `x-tenant-admin-key`, and it can be overridden per tenant if needed.

Tenant-admin credentials must be unique across tenants for each effective header+secret pair. Reusing the same secret under different tenant-admin headers is treated as a different identity, but sharing the same header+secret across tenants is rejected during validation and reload.

`/tenant/*` is intentionally self-management, not a second global admin surface. It derives tenant identity from tenant-admin auth so a tenant can inspect or reload only its own workspace.

Tenant self-management is scoped to the tenant resolved from tenant-admin auth. An optional tenant header may confirm that identity, but it cannot retarget management to another tenant.

In single mode there is no tenant-local management auth surface, so `/tenant/*` stays closed unless global admin auth is intentionally configured and supplied for default-tenant inspection/reload.

## 🔄 Reload Semantics

`POST /admin/reload` re-runs startup config loading from the original startup source, then rebuilds the live tenancy, admin-auth, provider, and template runtime atomically.

- Reload validates before activation.
- Invalid provider YAML or template syntax rejects the reload instead of being skipped.
- Failed reload keeps the previous working runtime active.
- It refreshes the live tenancy/admin/provider/template runtime, not the whole process shape.
- Changes to process-shaped settings such as `host`, `port`, `workers`, `log_level`, `latency`, `chaos`, `endpoints`, and `response_file` are rejected at reload time and still require a restart.
- This is an explicit reload operation, not watch mode.

`POST /tenant/reload` refreshes only the resolved tenant.

- It rebuilds that tenant's runtime, request-auth lookup state, and tenant-admin auth state.
- Invalid tenant provider YAML or template syntax rejects that tenant reload.
- It does not reload every tenant.
- Failed tenant reload keeps the previous working tenant state active.

## 🔒 Security and Isolation Notes

- VidaiMock is stateless and does not require a database.
- Secret config supports `value`, `value_file`, and `value_env`.
- `value_file` is preferred over `value_env`, and `value_env` is preferred over inline `value`.
- Secret-bearing fields are intentionally omitted from serialized status/config output and sanitized management responses.
- File- or env-backed secrets are preferred over inline values where possible.
- Logical isolation here means request resolution, provider matching, templates, and tenant policy are separated by workspace, but tenants still share one process, one memory space, and one telemetry pipeline.
- This is not hard isolation such as separate processes, containers, or OS sandboxes.

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

# X-Mock-Status header — force any HTTP status on any real endpoint.
# Returns HTTP 429 with an OpenAI-shape error envelope (provider-accurate).
curl -H "X-Mock-Status: 429" http://localhost:8100/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model": "gpt-4", "messages": [{"role": "user", "content": "Hi"}]}'

# ?chaos_status=503 URL query — stateless per-URL chaos.
# Lets a gateway register one "broken" endpoint and one "healthy" endpoint
# against the same mock instance for fallback/circuit-breaker testing.
curl "http://localhost:8100/v1/chat/completions?chaos_status=503" \
  -H "Content-Type: application/json" \
  -d '{"model": "gpt-4", "messages": [{"role": "user", "content": "Hi"}]}'

# Anthropic request validation — missing max_tokens returns real 400 envelope
curl http://localhost:8100/v1/messages \
  -H "Content-Type: application/json" \
  -d '{"model": "claude", "messages": [{"role": "user", "content": "Hi"}]}'
# -> HTTP 400 {"type":"error","error":{"type":"invalid_request_error","message":"max_tokens: Field required"}}

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

# Agentic tool loop — send a tool result back and get plain-text synthesis
# instead of another tool_calls. Lets ADK/LangGraph/LangChain Runner loops
# terminate against the mock the same way they do against real providers.
curl http://localhost:8100/v1/chat/completions \
  -H "Content-Type: application/json" -d '{
    "model": "gpt-4o",
    "tools": [{"type": "function", "function": {"name": "get_weather", "parameters": {}}}],
    "messages": [
      {"role": "user", "content": "Weather in London?"},
      {"role": "assistant", "tool_calls": [{"id":"c1","type":"function","function":{"name":"get_weather","arguments":"{}"}}]},
      {"role": "tool", "tool_call_id": "c1", "content": "15°C cloudy"}
    ]
  }'
# -> {"choices":[{"message":{"content":"Based on the tool results, ...","tool_calls":null}, "finish_reason":"stop"}]}

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

Provider YAML files in `config/providers/` define how endpoints match and respond in single mode. In multi mode, tenant-owned metadata lives in `tenants/<id>/tenant.toml`, and tenant overlays live under `tenants/<id>/providers/` and `tenants/<id>/templates/`.

```yaml
name: "my-provider"
matcher: "^/v1/my/endpoint$"         # Regex path match
response_template: "my/template.j2"  # Tera template path (HTTP 2xx responses)
error_template: "my/error.j2"        # Tera template path (HTTP 4xx / 5xx responses)
status_code: "200"                   # HTTP status — static or Tera expression
priority: 10                         # Higher matches first
stream:
  enabled: true                      # Compatibility field; the stream block itself enables streaming
  frame_format: raw                  # "raw" = template controls SSE framing
  lifecycle:
    on_start:
      template_path: "my/stream_start.j2"
    on_chunk:
      template_path: "my/stream_delta.j2"
    on_stop:
      template_path: "my/stream_stop.j2"
```

**`status_code`** accepts static values (`"400"`) or Tera expressions
(`"{% if json.max_tokens %}200{% else %}400{% endif %}"`) so a provider can
validate required fields before returning success. Both `{{ ... }}` expressions
and `{% ... %}` statements are rendered.

**`error_template`** is rendered instead of `response_template` whenever the
resolved HTTP status is ≥ 400. This is how chaos injection, `X-Mock-Status`,
`?chaos_status=`, and provider-side validation all produce correctly-shaped
error envelopes (OpenAI's `{"error": {...}}`, Anthropic's `{"type":"error",...}`,
Gemini's `{"error":{"code","message","status"}}`). The rendered template has a
`status_code` variable in scope so it can self-describe per status.

**`frame_format: raw`** gives the template full control over SSE framing —
essential for providers like OpenAI's Responses API that use typed `event:`
lines. The renderer preserves blank lines as frame separators so templates
can emit multi-event sequences (e.g. terminal `finish_reason` chunk → usage
chunk → `[DONE]`) without framing drift.

### Overriding bundled providers and templates

The bundled providers (`config/providers/*.yaml`) and templates
(`config/templates/**/*.j2`) are embedded into the binary as sensible
defaults. Anything in `--config-dir` overrides them by filename — disk
beats embedded.

- To change how `/v1/chat/completions` responds, drop a
  `providers/openai.yaml` into your config dir. VidaiMock loads yours
  instead of the bundled one.
- To change a template while keeping the provider config, drop a same-path
  `templates/openai/chat.json.j2` into your config dir. Templates are
  overridable independently of provider configs.
- To add a new endpoint, drop any YAML into `providers/` with a unique
  `matcher`. Higher-`priority` providers match before lower-priority ones.

No restart-tricks, no forking, no git submodules — the overlay is the
upgrade path. Bundled defaults can change between versions without
disrupting your customisations.

### Chaos & error injection modes

VidaiMock has four ways to trigger a non-200 response, all funnelling through
the same `error_template` pipeline:

| Trigger | Scope | Use case |
|---|---|---|
| `?chaos_status=503` URL query | Per URL | Gateway registers one "broken" and one "healthy" endpoint against the same mock instance — fallback/circuit-breaker testing |
| `X-Mock-Status: 429` header | Per request | SDK-level test wants a specific status on a real provider route |
| `X-Vidai-Chaos-Drop: 100` header | Probabilistic | Chaos testing; returns provider-shaped 500 JSON |
| Provider `status_code` Tera expression | Per request field | Request validation (e.g. Anthropic's `max_tokens` requirement) |

All four route to the provider's `error_template`, so SDK clients see a
parseable error envelope regardless of how the failure was injected.

### Tera template helpers

Response templates can call built-in functions to keep logic declarative:

| Helper | Returns | Use case |
|---|---|---|
| `uuid()` | random UUID string | IDs (`chatcmpl-{{ uuid() }}`, `msg_{{ uuid() }}`) |
| `timestamp()` | current unix seconds (int) | `created` / `created_at` fields |
| `iso_timestamp()` | ISO-8601 string | Human-readable timestamps |
| `random_int(min, max)` | integer | Mock token counts, call IDs |
| `random_float(min, max)` | float | Embeddings, scores |
| `has_tool_result(messages, provider)` | bool | Agentic loop termination — see below |

**`has_tool_result(messages, provider)`** detects whether the request's
conversation history already contains a tool result, so chat templates can
switch from "emit another `tool_call`" to "emit plain-text synthesis" and
agentic Runner loops (ADK, LangGraph, LangChain) terminate correctly.
Provider-specific shapes recognised:

| `provider` | Detection |
|---|---|
| `openai` | any message with `role == "tool"` |
| `anthropic` | user message whose `content` array contains a block with `type == "tool_result"` |
| `gemini` | user content whose `parts` array contains a `functionResponse` key |

Default is `openai` when `provider` is omitted. Malformed/missing inputs
return `false` rather than raising — safe to use unconditionally in
`{% if %}` guards.

Usage example in a custom OpenAI-compat template:

```jinja2
{% if json.tools and has_tool_result(messages=json.messages, provider="openai") %}
  {# Plain-text synthesis branch #}
{% elif json.tools %}
  {# Tool call branch #}
{% else %}
  {# Default text branch #}
{% endif %}
```

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
