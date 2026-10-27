---
title: Changelog
---

# Changelog

Release history, newest first. Sourced from the annotated git tags — the
authoritative record of what shipped.

!!! note
    The repo's `CHANGELOG.md` is currently behind the tags (it stops at
    0.1.0). This page reflects the actual released tags `v0.1.0` … `v0.2.7`.

## v0.4.0 — Wire fidelity for gateway testing

- Responses report the requested model on every provider, streaming or not.
- Stream templates get `chunk_index`, `chunk_count` and `stream_text`; the
  Responses stream numbers events without gaps and closes with the streamed
  text (#11).
- Stream text is JSON-escaped everywhere; Azure, Vertex and Bedrock streams
  match the real wire formats, including Bedrock's binary event streams.
- `response_headers` (request ids, rate limits, `retry-after`), new routes
  (Anthropic `count_tokens`, legacy completions, stored responses, audio,
  videos, OCR, image edits), scenario playback, `X-Vidai-Usage`,
  `X-Vidai-Output-Tokens` and a configurable body limit.
- Full list and behaviour changes: `CHANGELOG.md` in the repository.

## v0.2.7 — Tool-call echo & tool-streaming wire format

- OpenAI chat template now **echoes the caller's declared tool name**
  (was hard-coded `get_weather`/`Paris`). Anthropic and Gemini already
  echoed correctly; regression-guarded.
- Content extractor recognises Anthropic `tool_use` and Gemini
  `functionCall` as structured tool responses (parity with OpenAI
  `tool_calls`), so streaming-with-tools emits a single clean structured
  frame instead of word-chunking the JSON body.
- Gemini & Anthropic streaming templates updated for structured chunks.
- New reusable single-line-JSON wire-format test helper.

## v0.2.6 — Agentic tool-loop termination

- `has_tool_result(messages, provider)` Tera helper (implemented in Rust)
  detects a prior tool result across OpenAI / Anthropic / Gemini history.
- Bundled chat templates branch on it: emit plain-text synthesis instead
  of looping another tool call → ADK / LangGraph / LangChain Runner loops
  terminate in mock mode with zero live tokens.
- Extensive array-index edge-case test coverage.

## v0.2.5 — Wire-shape fidelity & unified error pipeline

- Single error pipeline: any resolved status ≥ 400 renders the provider's
  `error_template`, regardless of trigger.
- `?chaos_status=` URL query injection — enables one-instance
  primary/fallback testing without header forwarding.
- `X-Vidai-Chaos-Drop` now returns a JSON error envelope with
  `application/json` (was plain text).
- `status_code` accepts `{% … %}` statements as well as `{{ … }}`.
- Anthropic `/v1/messages` validates `max_tokens` → real 400 envelope.
- OpenAI streaming: terminal `finish_reason` chunk + correct blank-line
  SSE termination. Gemini: no `[DONE]` sentinel; `finishReason`/
  `usageMetadata` only on the terminal chunk. Anthropic: blank-line
  event separation.

## v0.2.4 — Anthropic depth

- `tool_use` conditional branching with `caller.type`.
- 2025+ usage fields: cache tiers, `service_tier`, `inference_geo`,
  `stop_details`.
- Full 7-event streaming lifecycle via `frame_format: raw`.

## v0.2.3 — Comprehensive Gemini support

- Gemini 2.5 fields: `thoughtsTokenCount`, `promptTokensDetails`,
  `modelVersion`, `responseId`.
- Dedicated `:embedContent` and `:countTokens` providers (distinct
  envelopes).
- `GET /v1beta/models` listing and the `/v1beta/openai/*` shim.

## v0.2.2 — Per-request error simulation

- `X-Mock-Status` header override on any endpoint.

## v0.2.1 — Provider priority

- Fixed deterministic matcher ordering when patterns overlap.

## v0.2.0 — Custom status codes & bundled OpenAI surface

- `status_code` provider field; `frame_format: raw` streaming.
- Bundled OpenAI Responses API, embeddings, images, moderations.

## v0.1.x — Foundation

- Initial release: multi-provider (OpenAI / Anthropic / Gemini /
  OpenRouter), Axum/Tokio async server, `mimalloc`, latency simulation
  (benchmark / realistic), Prometheus `/metrics`, `/health`, `/status`,
  echo handler, graceful shutdown, path-traversal hardening, proptest
  fuzzing, Vertex AI provider, provider `priority`, stable per-request
  `{{ uuid }}` / `{{ timestamp }}`.

---

For the precise diff of any release, see the
[GitHub tags](https://github.com/vidaiUK/VidaiMock/tags).
