---
title: Templating
---

# Templating

Every VidaiMock response is a [Tera](https://keats.github.io/tera/) template
(Jinja2-style). This is what makes the mock *dynamic* rather than a static
fixture — a template can reflect request data, generate IDs, branch on
fields, and synthesise realistic responses without any mocking code.

## Request context

Templates receive the parsed request and request metadata:

| Variable | Contents |
|---|---|
| `json` | The full parsed request body. `json.model`, `json.messages`, `json.tools`, … |
| `headers` | Request headers (lowercased keys). |
| `query` | URL query parameters. |
| `path_segments` | Path split on `/` (e.g. `["v1", "chat", "completions"]`). |
| `status_code` | The resolved HTTP status (available in error templates). |

## Built-in helper functions

| Helper | Returns | Use |
|---|---|---|
| `uuid()` | random UUID string | IDs — `chatcmpl-{{ uuid() }}`, `msg_{{ uuid() }}` |
| `timestamp()` | unix seconds (int) | `created` / `created_at` fields |
| `iso_timestamp()` | ISO-8601 string | human-readable timestamps |
| `random_int(min, max)` | integer | mock token counts, call IDs |
| `random_float(min, max)` | float | embedding values, scores |
| `has_tool_result(messages, provider)` | bool | agentic loop termination |

### `has_tool_result(messages, provider)`

Detects whether the conversation history already contains a tool result, so
a template can switch from "emit a tool call" to "emit a plain-text answer"
and agentic loops terminate. Implemented in Rust for robust deep-JSON
inspection.

| `provider` | Detection |
|---|---|
| `openai` | any message with `role == "tool"` |
| `anthropic` | a user message whose `content[]` contains `type == "tool_result"` |
| `gemini` | user content whose `parts[]` contains a `functionResponse` |

Default is `openai` when `provider` is omitted. Malformed/missing input
returns `false`, never raises — safe inside any `{% if %}` guard. Full
explanation: [Agentic workflow testing](agentic-testing.md).

## Branching example

The bundled OpenAI chat template, conceptually:

```jinja2
{% if json.tools and has_tool_result(messages=json.messages, provider="openai") %}
  {# tools declared but a tool result is already in history → answer in text #}
  { ... "finish_reason": "stop", "message": {"content": "Based on the tool results..."} }
{% elif json.tools %}
  {# tools declared, no result yet → emit a tool_call echoing the caller's name #}
  { ... "finish_reason": "tool_calls",
    "message": {"tool_calls": [{"function":
      {"name": "{{ json.tools.0.function.name }}", "arguments": "{}"}}]} }
{% elif json.response_format %}
  {# structured output → JSON-string content #}
{% elif json.model is starting_with("o") %}
  {# reasoning model → include reasoning_tokens in usage #}
{% else %}
  {# default → plain assistant text #}
{% endif %}
```

## Reflecting request data

```jinja2
{
  "id": "chatcmpl-{{ uuid() }}",
  "created": {{ timestamp() }},
  "model": "{{ json.model }}",
  "choices": [{
    "message": {
      "role": "assistant",
      "content": "You said: {{ json.messages | last | get(key='content') }}"
    },
    "finish_reason": "stop"
  }]
}
```

!!! note "Deep array access"
    Tera is unreliable at indexing into arrays of mixed-type objects.
    For history-spanning logic (like detecting a tool result) use the
    `has_tool_result()` helper rather than hand-rolling `json.messages | …`
    array walks. This is why that helper exists in Rust.

## Streaming templates

Streaming providers use a `lifecycle` block (`on_start`, `on_chunk`,
`on_stop`) plus optional `frame_format: raw` for full SSE control. The
per-chunk content is available as `chunk`, its zero-based position as
`chunk_index`, and the total number of chunks as `chunk_count` (also set in
`on_start` and `on_stop`). Use `chunk_index` for protocol sequence numbers
rather than deriving them from chunk text, which can repeat. Details:
[Streaming](streaming.md) and
[Writing templates](configuration/templates.md).

## Where templates live

Bundled templates are embedded in the binary under `config/templates/`.
Override any of them, or add your own, via `--config-dir`. See
[Overriding bundled defaults](configuration/overriding.md).
