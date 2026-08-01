#!/usr/bin/env python3
"""
Tool-calling parity matrix — provider x mode (non-stream/stream) x turn.

Why this exists
---------------
VidaiMock renders SEPARATE templates for the non-streaming and streaming paths.
Nothing structurally enforces that the two agree, so a provider can support tool
calls when `stream=false` and silently fall back to filler text when
`stream=true`. That is issue #8 (OpenAI Responses), and the same class of defect
can appear in any provider added later.

This matrix asserts the invariant directly:

    for every provider that advertises tool support,
    a request carrying `tools` must produce a tool call in BOTH modes,
    and a follow-up turn carrying a tool RESULT must stop calling tools.

The second half matters as much as the first: a mock that always returns a tool
call sends an agentic client (LangGraph, ADK, Codex) into an infinite loop.

Each provider declares its own protocol-specific markers because the wire shapes
differ (OpenAI `tool_calls`, Anthropic `tool_use`, Gemini `functionCall`,
Responses `function_call`). We assert on those real markers rather than a
lowest-common-denominator substring, so a provider emitting the *wrong* shape
fails instead of passing on a coincidental match.

Usage:
    python3 tests/test_tool_calling_matrix.py            # starts its own server
    python3 tests/test_tool_calling_matrix.py --port 8100  # use a running one
"""

import argparse
import json
import subprocess
import sys
import time
import urllib.error
import urllib.request

DEFAULT_PORT = 8100
BINARY = "./target/release/vidaimock"


# --------------------------------------------------------------------------
# Provider matrix.
#
# tool_markers  : substrings that prove a real tool call was emitted. ALL must
#                 be present (not any), because a streaming tool call is a
#                 multi-event contract: e.g. Anthropic must emit both a
#                 tool_use content_block_start AND an input_json_delta. Matching
#                 on "any" lets a half-broken stream (correct delta, wrong
#                 block start) pass — verified against an injected regression.
#                 Per-mode lists are supported via a {"non-stream": [...],
#                 "stream": [...]} dict when the shapes genuinely differ.
# text_markers  : substrings that prove the provider produced plain text. The
#                 streaming and non-streaming wire shapes differ (SSE deltas vs
#                 a whole JSON body, which is pretty-printed with spaces after
#                 the colons), so markers are matched against whitespace-
#                 normalized output and listed per mode where they diverge.
# second_turn   : request body replaying a tool RESULT back to the mock. The
#                 provider must then answer with text, not another tool call.
#                 None => this provider has no defined tool-result encoding yet.
# known_broken  : issue reference. These are EXPECTED to fail until fixed; they
#                 are reported separately so the suite's exit code stays
#                 meaningful while the bug is open.
# unimplemented : provider has no tool branching in EITHER mode. Distinct from
#                 a streaming regression: nothing was lost, it was never built.
# --------------------------------------------------------------------------

def openai_chat_body(stream, with_result=False):
    messages = [{"role": "user", "content": "What is the weather?"}]
    if with_result:
        messages += [
            {"role": "assistant", "tool_calls": [
                {"id": "call_1", "type": "function",
                 "function": {"name": "get_weather", "arguments": "{}"}}]},
            {"role": "tool", "tool_call_id": "call_1", "content": "15C cloudy"},
        ]
    body = {
        "model": "gpt-4",
        "messages": messages,
        "tools": [{"type": "function", "function": {
            "name": "get_weather",
            "parameters": {"type": "object", "properties": {}}}}],
    }
    if stream:
        body["stream"] = True
    return body


def anthropic_body(stream, with_result=False):
    messages = [{"role": "user", "content": "What is the weather?"}]
    if with_result:
        messages += [
            {"role": "assistant", "content": [
                {"type": "tool_use", "id": "toolu_1", "name": "get_weather", "input": {}}]},
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "toolu_1", "content": "15C"}]},
        ]
    body = {
        "model": "claude-haiku-4-5",
        "max_tokens": 100,
        "messages": messages,
        "tools": [{"name": "get_weather",
                   "input_schema": {"type": "object", "properties": {}}}],
    }
    if stream:
        body["stream"] = True
    return body


def gemini_body(stream, with_result=False):
    contents = [{"role": "user", "parts": [{"text": "What is the weather?"}]}]
    if with_result:
        contents += [
            {"role": "model", "parts": [
                {"functionCall": {"name": "get_weather", "args": {}}}]},
            {"role": "user", "parts": [
                {"functionResponse": {"name": "get_weather",
                                      "response": {"temp": "15C"}}}]},
        ]
    return {
        "contents": contents,
        "tools": [{"functionDeclarations": [{
            "name": "get_weather",
            "parameters": {"type": "object", "properties": {}}}]}],
    }


def responses_body(stream, with_result=False):
    # The Responses API uses `input` (not `messages`) and encodes a tool result
    # as an item of type `function_call_output` (not a `tool` role).
    inp = [{"role": "user", "content": "What is the weather?"}]
    if with_result:
        inp += [
            {"type": "function_call", "call_id": "c1",
             "name": "get_weather", "arguments": "{}"},
            {"type": "function_call_output", "call_id": "c1", "output": "15C"},
        ]
    body = {
        "model": "gpt-4o-mini",
        "input": inp,
        "tools": [{"type": "function", "name": "get_weather",
                   "parameters": {"type": "object", "properties": {}}}],
    }
    if stream:
        body["stream"] = True
    return body


MATRIX = [
    {
        "name": "openai-chat",
        "path": "/v1/chat/completions",
        "body": openai_chat_body,
        "tool_markers": ['"tool_calls"'],
        # non-stream: message.content string; stream: delta.content
        "text_markers": ['"content":"'],
        "second_turn": True,
    },
    {
        "name": "anthropic",
        "path": "/v1/messages",
        "body": anthropic_body,
        # Streaming requires BOTH the tool_use block start and the args delta.
        "tool_markers": {
            "non-stream": ['"type":"tool_use"'],
            "stream": ['"type":"tool_use"', '"input_json_delta"'],
        },
        # non-stream: a {"type":"text"} content block; stream: text_delta events
        "text_markers": ['"text_delta"', '"type":"text"'],
        "second_turn": True,
    },
    {
        "name": "gemini",
        "path": "/v1beta/models/gemini-2.5-flash:generateContent",
        "stream_path": "/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse",
        "body": gemini_body,
        "tool_markers": ['"functionCall"'],
        "text_markers": ['"text"'],
        "second_turn": True,
    },
    {
        "name": "openai-responses",
        "path": "/v1/responses",
        "body": responses_body,
        # Streaming requires the whole function-call event contract: the item
        # must be announced, its arguments streamed, and finalized. Asserting
        # only on '"function_call"' would pass on a stream that announces the
        # item then never sends arguments (issue #8's shape after a partial
        # fix). Fixed in VM-012.
        "tool_markers": {
            "non-stream": ['"type":"function_call"'],
            "stream": [
                '"type":"function_call"',
                "response.function_call_arguments.delta",
                "response.function_call_arguments.done",
            ],
        },
        "text_markers": ['"output_text"', "mock response complete"],
        "second_turn": True,
    },
    # Tool calling was never implemented for these in EITHER mode: they point at
    # chat_completion.json.j2, which has no tool branch (unlike chat.json.j2).
    # Tracked so the gap stays visible; not counted as failures.
    {
        "name": "azure-openai",
        "path": "/openai/deployments/gpt-4/chat/completions?api-version=2024-02-01",
        "body": openai_chat_body,
        "tool_markers": ['"tool_calls"'],
        "text_markers": ['"content"'],
        "unimplemented": True,
    },
    {
        "name": "mistral",
        "path": "/v1/mistral/chat/completions",
        "body": openai_chat_body,
        "tool_markers": ['"tool_calls"'],
        "text_markers": ['"content"'],
        "unimplemented": True,
    },
    {
        "name": "groq",
        "path": "/groq/v1/chat/completions",
        "body": openai_chat_body,
        "tool_markers": ['"tool_calls"'],
        "text_markers": ['"content"'],
        "unimplemented": True,
    },
    {
        "name": "openrouter",
        "path": "/api/v1/chat/completions",
        "body": openai_chat_body,
        "tool_markers": ['"tool_calls"'],
        "text_markers": ['"content"'],
        "unimplemented": True,
    },
]


def post(url, body, timeout=15):
    req = urllib.request.Request(
        url, data=json.dumps(body).encode(),
        headers={"Content-Type": "application/json"}, method="POST")
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.status, r.read().decode("utf-8", "replace")
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode("utf-8", "replace")
    except Exception as e:
        return 0, f"<transport error: {e}>"


def normalize(s):
    """Collapse JSON pretty-printing so one marker matches both wire shapes.

    Non-streaming bodies are pretty-printed (`"content": "x"`) while SSE frames
    are compact (`"content":"x"`). Removing whitespace after structural colons
    lets a single marker list cover both modes.
    """
    return s.replace('": "', '":"').replace('": ', '":').replace('\n', '')


def diagnose(raw, spec, stream=False):
    """Explain WHY a tool call was missing, so failures are actionable."""
    if "[[object]]" in raw:
        return ("template stringified a structured chunk as [[object]] — "
                "the tool call reached the template but it has no tool branch")
    norm = normalize(raw)
    for m in markers_for(spec, "text_markers", stream):
        if m in norm:
            return f"fell back to plain text (found {m!r})"
    return "no tool markers and no recognizable text markers"


def markers_for(spec, key, stream):
    """Resolve a marker list that may be per-mode."""
    m = spec[key]
    if isinstance(m, dict):
        return m["stream" if stream else "non-stream"]
    return m


def check(base, spec, stream, with_result):
    path = spec.get("stream_path", spec["path"]) if stream else spec["path"]
    status, raw = post(base + path, spec["body"](stream, with_result))
    if status != 200:
        return False, f"HTTP {status}", raw

    norm = normalize(raw)
    tool_markers = markers_for(spec, "tool_markers", stream)
    # ALL markers required: a streaming tool call is a multi-event contract and
    # a partially-correct stream must not pass.
    missing = [m for m in tool_markers if m not in norm]
    has_tool = not missing
    has_any_tool = len(missing) < len(tool_markers)

    # A template that stringifies a structured chunk is ALWAYS broken, in both
    # turn types. Checked before the tool/no-tool logic below, because on the
    # second turn "no tool call" would otherwise look like correct termination
    # when in fact the stream is emitting the literal text "[[object]]".
    if "[[object]]" in raw:
        return False, diagnose(raw, spec, stream), raw

    text_markers = markers_for(spec, "text_markers", stream)

    if with_result:
        # Contract inverts: a tool result is in history, so the mock must
        # synthesize text instead of calling the tool again. Termination must
        # be REAL text, not an empty/garbled stream, so require a text marker.
        if has_any_tool:
            return False, "still returns a tool call (agentic loop never terminates)", raw
        if not any(m in norm for m in text_markers):
            return False, "no tool call, but no coherent text either", raw
        return True, "terminated with text", raw

    if has_tool:
        return True, "tool call present", raw
    if has_any_tool:
        # Partially-correct stream: some events right, some wrong.
        return False, f"incomplete tool call — missing {', '.join(repr(m) for m in missing)}", raw
    return False, diagnose(raw, spec, stream), raw


def wait_healthy(base, timeout=30):
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            with urllib.request.urlopen(base + "/health", timeout=2) as r:
                if r.status == 200:
                    return True
        except Exception:
            time.sleep(0.2)
    return False


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=None,
                    help="use an already-running server on this port")
    args = ap.parse_args()

    proc = None
    port = args.port or DEFAULT_PORT
    base = f"http://127.0.0.1:{port}"

    if args.port is None:
        proc = subprocess.Popen([BINARY, "--port", str(port)],
                                stdout=subprocess.DEVNULL,
                                stderr=subprocess.DEVNULL)
    if not wait_healthy(base):
        print(f"ERROR: no healthy server at {base}")
        if proc:
            proc.terminate()
        return 2

    real_failures = []
    known = []
    unimpl = []

    try:
        print(f"Tool-calling parity matrix -> {base}\n")
        header = f"{'provider':<18} {'turn':<12} {'non-stream':<12} {'stream':<12}"
        print(header)
        print("-" * len(header))

        for spec in MATRIX:
            for with_result in (False, True):
                if with_result and not spec.get("second_turn"):
                    continue
                turn = "tools+result" if with_result else "tools"
                cells = []
                for stream in (False, True):
                    ok, why, _ = check(base, spec, stream, with_result)
                    mode = "stream" if stream else "non-stream"

                    broken_map = (spec.get("known_broken_second_turn", {})
                                  if with_result else spec.get("known_broken", {}))
                    issue = broken_map.get(mode)

                    if spec.get("unimplemented"):
                        cells.append("n/a" if not ok else "PASS")
                        if not ok:
                            unimpl.append((spec["name"], mode, turn, why))
                    elif ok:
                        cells.append("PASS")
                    elif issue:
                        cells.append(f"KNOWN({issue})")
                        known.append((spec["name"], mode, turn, issue, why))
                    else:
                        cells.append("FAIL")
                        real_failures.append((spec["name"], mode, turn, why))

                print(f"{spec['name']:<18} {turn:<12} {cells[0]:<12} {cells[1]:<12}")

        print()
        if known:
            print("Known-broken (expected until fixed):")
            for name, mode, turn, issue, why in known:
                print(f"  [{issue}] {name} / {mode} / {turn}: {why}")
            print()
        if unimpl:
            print("Not implemented in either mode (feature gap, not a regression):")
            for name, mode, turn, why in unimpl:
                if mode == "non-stream":
                    print(f"  {name} / {turn}: {why}")
            print()
        if real_failures:
            print("FAILURES:")
            for name, mode, turn, why in real_failures:
                print(f"  {name} / {mode} / {turn}: {why}")
            print(f"\n{len(real_failures)} unexpected failure(s)")
            return 1

        print("No unexpected failures.")
        return 0
    finally:
        if proc:
            proc.terminate()
            try:
                proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                proc.kill()


if __name__ == "__main__":
    sys.exit(main())
