---
title: Streaming
---

# Streaming

Traditional mocks "stream" by chunking a static download. VidaiMock
simulates the **physics** of LLM streaming — time-to-first-token, per-token
pacing, and the exact SSE framing each provider uses on the wire — so
streaming UI, SDK iterators, and resilience logic behave as they would
against the real API.

## Realistic vs benchmark mode

| Mode | Behaviour |
|---|---|
| `benchmark` (default) | Emits chunks as fast as possible. Best for throughput tests. |
| `realistic` | Adds TTFT (`--latency`) before the first token and paces subsequent tokens. Best for streaming-UX tests. |
| `debug` | Verbose logging for template/stream diagnosis. |

```bash
./vidaimock --mode realistic --latency 300
```

Per-request overrides: `X-Vidai-Latency`, `X-Vidai-Chaos-Trickle`
(per-chunk delay), `X-Vidai-Chaos-Disconnect` (mid-stream sever).

## Provider-accurate framing

Every streaming response is byte-level accurate to the provider it emulates.
This is regression-tested against captured real-provider output.

### OpenAI chat

```
data: {... "delta": {"content": "Hel"} ...}

data: {... "delta": {"content": "lo"} ...}

data: {... "delta": {}, "finish_reason": "stop"}

data: {... "choices": [], "usage": {...}}     # only if stream_options.include_usage

data: [DONE]

```

Every event ends with a blank line (`\n\n`). The terminal sequence is a
finish-reason chunk → optional usage chunk → `[DONE]`. Strict parsers like
`openai-python` iterate this cleanly.

### OpenAI Responses API

Typed events, not bare `data:` chunks:

```
event: response.created
data: {"type":"response.created", ...}

event: response.output_text.delta
data: {"type":"response.output_text.delta","delta":"Hel"}

event: response.completed
data: {"type":"response.completed", ...}
```

### Anthropic — 7-event lifecycle

```
event: message_start
event: content_block_start
event: ping
event: content_block_delta   (one or more)
event: content_block_stop
event: message_delta
event: message_stop
```

Events are blank-line separated so the SDK attributes each `data:` payload
to the correct `event:` header. Tool-mode emits a `tool_use`
`content_block_start` with `input_json_delta` deltas.

### Gemini

No `[DONE]` sentinel — the stream simply ends (OpenAI convention, wrong for
Gemini). Intermediate frames carry text deltas with `finishReason: null`
and **no** `usageMetadata`; the terminal frame carries `finishReason: STOP`
**and** `usageMetadata`.

## `frame_format: raw`

By default the stream engine wraps each rendered chunk in `data: …\n\n`.
Set `frame_format: raw` on a provider's `stream` block to take full control
of framing — the template emits its own `event:` / `data:` lines verbatim.
This is how typed-event providers (Responses API, Anthropic) are built.

```yaml
stream:
  enabled: true
  frame_format: raw
  lifecycle:
    on_start: { template_path: "my/stream_start.j2" }
    on_chunk: { template_path: "my/stream_delta.j2" }
    on_stop:  { template_path: "my/stream_stop.j2" }
```

The renderer preserves blank lines as frame separators, so a single template
can emit a multi-event sequence (e.g. finish chunk → usage chunk → `[DONE]`)
without framing drift. See [Writing templates](configuration/templates.md).

## Tool-call streaming

A streaming request that returns a tool call is **not** word-chunked as fake
text. The tool call (`tool_calls` / `tool_use` / `functionCall`) is emitted
as a single structured frame, single-line JSON, exactly as the real provider
does. See [Tool calling](tool-calling.md).


## Scenario playback

For exact, scripted responses (event order, argument fragmentation, error
bodies, timing) a provider can replay scenario files picked per request:

```yaml
name: "scn-openai"
matcher: "^/scn/v1/chat/completions$"
response_body: '{}'            # used when no scenario is requested
scenarios:
  dir: "openai"                # <config-dir>/scenarios/openai/
  key: "{% if json.model is starting_with('scn-') %}{{ json.model | replace(from='scn-', to='') }}{% endif %}"
stream: { enabled: true, frame_format: raw }
```

The id comes from the `X-Vidai-Scenario` header, or else from rendering `key`.
An empty id means a normal response; an id with no files returns a 500 that
names it, so a mis-wired test fails loudly. Files (all Tera templates):

| File | Used for |
|---|---|
| `<id>.sse` / `<id>.sse.j2` | streaming requests: sent as written |
| `<id>.body` / `<id>.json` (`.j2` too) | non-streaming requests, and any error status |
| `<id>.status` | optional status code (default 200) |

Each blank-line-separated event is its own network write, so clients see real
event boundaries. Two SSE comment lines, removed before sending, script the
timing:

```text
:vidai-delay 250      wait 250 ms before sending this event
:vidai-split          end the current write here, mid-event
```

`X-Vidai-Chaos-Trickle` (or `chaos.trickle_ms`) sets the gap between events
that have no `:vidai-delay`. Providers with `encoding: aws-event-stream` replay
through the Bedrock binary encoder.
