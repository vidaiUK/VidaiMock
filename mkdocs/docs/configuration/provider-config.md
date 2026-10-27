---
title: Provider Config Reference
---

# Provider Config Reference

A provider is a YAML file in `config/providers/`. It maps an incoming request
path to a response template and controls status, errors, and streaming. The
bundled providers are embedded in the binary; you override or extend them via
`--config-dir` ([Overriding bundled defaults](overriding.md)).

## Schema

```yaml
name: "my-provider"                  # unique identifier
matcher: "^/v1/my/endpoint$"         # regex matched against the request path
response_template: "my/template.j2"  # Tera template for 2xx responses
error_template: "my/error.j2"        # Tera template for 4xx/5xx responses
status_code: "200"                   # static, or a Tera expression
priority: 10                         # higher wins when multiple matchers hit
response_headers:                    # extra headers; values are Tera templates
  x-request-id: "req_{{ uuid }}"
  retry-after: "{% if status_code == 429 %}2{% endif %}"   # empty -> not sent

stream:
  enabled: true
  frame_format: raw                  # "raw" = template controls SSE framing
  lifecycle:
    on_start: { template_path: "my/stream_start.j2" }
    on_chunk: { template_path: "my/stream_delta.j2" }
    on_stop:  { template_path: "my/stream_stop.j2" }
```

## Fields

### `matcher`

A regex tested against the request path. The first provider (by `priority`,
then load order) whose matcher hits serves the request.

### `priority`

Integer, default `0`. Higher matches first. Use this to make a specific
provider win over a broad catch-all.

### `response_template`

Path (relative to `config/templates/`) of the Tera template rendered for
successful (2xx) responses. See [Templating](../templating.md).

### `error_template`

Rendered **instead of** `response_template` whenever the resolved HTTP
status is ≥ 400 — whether the status came from chaos injection,
`X-Mock-Status`, `?chaos_status=`, or a `status_code` expression. This is
how every failure path produces a provider-shaped error envelope. The
template has a `status_code` variable in scope so it can self-describe per
status.

### `status_code`

Controls the HTTP status. Accepts:

- A static string — `"400"`.
- A Tera **expression** — `"{{ path_segments | last }}"`.
- A Tera **statement** — `"{% if json.max_tokens %}200{% else %}400{% endif %}"`.

Both `{{ … }}` and `{% … %}` forms are rendered. This is how the bundled
Anthropic provider enforces the `max_tokens` requirement.

### `response_headers`

Extra headers on every response from this provider: normal, streaming and
error. Each value is a Tera template rendered with the request context plus
`status_code`, so headers can depend on the outcome. A value that renders empty
is not sent. A `content-type` entry overrides the default. The bundled OpenAI
and Anthropic providers use this to send request ids, rate-limit headers, and
`retry-after` on 429/503.

### `methods`

HTTP methods the provider answers, e.g. `["GET", "DELETE"]`. Default: `["POST"]`.
Templates can branch on the `method` variable.

### `body_encoding`

`base64`: the rendered body is base64 and is decoded before sending, for binary
responses such as audio or video. Pair it with a `content-type` in
`response_headers`.

### `scenarios`

`{dir, key}`: replay scripted responses from `<config-dir>/scenarios/<dir>/`,
picked by the `X-Vidai-Scenario` header or the rendered `key`. See
[Scenario playback](../streaming.md#scenario-playback).

### Request bodies

Templates see the request body as `json`. JSON bodies are passed through;
`multipart/form-data` becomes an object of its fields, with file parts as
`{"filename", "content_type", "size"}` (multipart sends numbers as strings, so
use `| int`). An empty or non-JSON body is `{}`.

### `stream`

Present (and `enabled: true`) to make the provider stream. `lifecycle`
defines `on_start` / `on_chunk` / `on_stop` templates. `frame_format: raw`
hands full SSE framing to the templates (needed for typed-event providers
like the Responses API and Anthropic). See
[Writing templates](templates.md) and [Streaming](../streaming.md).

`encoding: aws-event-stream` sends the binary AWS event stream that Bedrock
uses instead of SSE. Templates write events in the same `event:` / `data:`
form as raw SSE, and one template may emit several events. `aws_payload`
picks the Bedrock flavour:

| `aws_payload` | Wire shape | Used by |
|---|---|---|
| `bytes` (default) | every event is `:event-type chunk` with payload `{"bytes": "<base64 of the data JSON>"}`; `event:` lines are ignored | InvokeModelWithResponseStream |
| `json` | `:event-type` is the template's `event:` name; payload is the data JSON as written | ConverseStream |

## Chaos & error injection summary

| Trigger | Where |
|---|---|
| `?chaos_status=<code>` | URL query — per-URL, no header forwarding needed |
| `X-Mock-Status: <code>` | request header — per-request |
| `X-Vidai-Chaos-Drop: <pct>` | request header — probabilistic |
| `status_code` Tera expression | provider config — per request field |

All route through `error_template`. Full detail:
[Chaos & error injection](../chaos-and-errors.md).

## `mock-server.toml` (global)

Provider YAMLs control per-endpoint behaviour. Global runtime settings
(host, port, latency, chaos defaults) live in `mock-server.toml` — see the
[CLI & headers reference](../getting-started/cli-reference.md).
