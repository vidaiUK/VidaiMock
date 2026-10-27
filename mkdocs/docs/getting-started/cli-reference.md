---
title: CLI & Headers Reference
---

# CLI & Headers Reference

## Command-line flags

```
Usage: vidaimock [OPTIONS]
```

| Flag | Default | Description |
|---|---|---|
| `--host <HOST>` | `0.0.0.0` | Bind address. Use `127.0.0.1` for local-only. |
| `-p, --port <PORT>` | `8100` | Listen port. |
| `-w, --workers <N>` | num CPUs | Worker thread count. |
| `--config <FILE>` | `mock-server.toml` | Config file path. |
| `--config-dir <DIR>` | — | Directory of provider configs/templates that **overrides** the embedded defaults. See [Overriding](../configuration/overriding.md). |
| `--isolated` | `false` | Ignore the binary's embedded providers and templates; only load `--config-dir`. Locks the surface down to exactly what you declare. Equivalent env var: `VIDAIMOCK_ISOLATED=true`. See [Isolated mode](../configuration/overriding.md#isolated-mode). |
| `--latency <MS>` | `0` | Base artificial latency added to every request. |
| `--mode <MODE>` | `benchmark` | `benchmark` (fastest), `realistic` (token pacing + TTFT), or `debug`. |
| `--response-file <FILE>` | — | Custom response file overriding default-endpoint format. |
| `--endpoints <PATHS>` | — | Comma-separated endpoint list (overrides config). |
| `--format <FORMAT>` | — | Response format for default endpoints (`openai`, `anthropic`, …). |
| `--content-type <TYPE>` | — | Override the `Content-Type` header. |
| `--max-body-mb <MB>` | `32` | Largest request body accepted; larger bodies get 413. Env: `VIDAIMOCK_MAX_BODY_MB`; TOML: `max_body_mb`. |
| `-h, --help` | — | Print help. |
| `-V, --version` | — | Print version. |

## Configuration precedence

Highest priority first:

1. CLI flags — `./vidaimock --port 3000`
2. Environment variables — `VIDAIMOCK_PORT=3000`
3. Config file — `mock-server.toml`
4. Embedded defaults

### `mock-server.toml`

```toml
host = "0.0.0.0"
port = 8100
log_level = "info"

[latency]
mode = "realistic"   # token-by-token pacing
base_ms = 150        # TTFT before first token
jitter_pct = 0.2     # ±20% timing variance

[chaos]
enabled = false
drop_pct = 0.01        # 1% of requests fail (provider-shaped 500)
malformed_pct = 0.005  # 0.5% return deliberately broken JSON
trickle_ms = 0         # per-chunk delay during streaming
disconnect_pct = 0.05  # 5% of streams sever mid-generation
```

## Runtime headers

Any endpoint honours these request headers — no config or restart needed.
They override the configured defaults **for that single request**.

| Header | Effect |
|---|---|
| `X-Mock-Status: <code>` | Return this HTTP status (e.g. `429`, `500`) with a provider-shaped error body. |
| `X-Vidai-Latency: <ms>` | Override base latency for this request. |
| `X-Vidai-Jitter: <pct>` | Override latency jitter percentage. |
| `X-Vidai-Chaos-Drop: <pct>` | Probability (0–100) of a simulated provider-shaped 500. |
| `X-Vidai-Chaos-Malformed: <pct>` | Probability of a deliberately malformed (non-JSON) body. |
| `X-Vidai-Chaos-Trickle: <ms>` | Per-chunk delay during streaming. |
| `X-Vidai-Chaos-Disconnect: <pct>` | Probability of a mid-stream disconnect. |
| `X-Vidai-Usage: <k=v,…>` | Exact usage numbers, rendered in each provider's own shape. Keys: `input`, `output`, `cached`, `cache_write`, `reasoning`. E.g. `input=22740,output=12,cached=22515,reasoning=41`. OpenAI/Gemini count cached tokens inside the input; Anthropic reports them separately; Gemini adds `reasoning` to the total. |
| `X-Vidai-Output-Tokens: <n\|max>` | Generate `n` words of output (one per stream chunk) and report `n` output tokens. `max` uses the request's `max_tokens` / `max_completion_tokens` / `max_output_tokens` / `generationConfig.maxOutputTokens`. Capped at 100 000. |
| `X-Vidai-Scenario: <id>` | Replay scenario `<id>` on providers that declare `scenarios`. See [Scenario playback](../streaming.md#scenario-playback). |

## URL query parameters

| Query | Effect |
|---|---|
| `?chaos_status=<code>` | Like `X-Mock-Status` but encoded in the URL — lets your routing layer register one "broken" endpoint and one "healthy" endpoint against the same instance without forwarding client headers. See [Chaos & error injection](../chaos-and-errors.md). |

## Built-in paths

| Path | Purpose |
|---|---|
| `GET /health` | Liveness — `{"status":"ok"}`. |
| `GET /status` | Effective config (secrets redacted). |
| `GET /metrics` | Prometheus metrics (when metrics enabled). |
| `POST /error/{code}` | Provider-agnostic error simulator. |

All provider endpoints are listed in the [Providers overview](../providers/index.md).
