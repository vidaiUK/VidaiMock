# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0] - 2026-08-03
### Added
- **Rust library target** — embed the mock server directly in Rust
  integration tests instead of orchestrating an external process
  ([#9](https://github.com/vidaiUK/VidaiMock/issues/9)):

  ```rust
  let server = vidaimock::MockServer::builder()
      .bind("127.0.0.1:0")
      .start()
      .await?;
  ```

  Same providers, templates and behaviour as the binary. `start()`
  returns once the socket is bound, so no health polling is needed.
  Instances share no global state, so parallel tests are safe. See the
  [Rust Library recipe](https://vidai.uk/docs/mock/recipes/rust-library/).
- Published to [crates.io](https://crates.io/crates/vidaimock) — the
  library is consumed as a normal `[dev-dependencies]` entry.
- 10 integration tests covering ephemeral ports, parallel instances,
  shutdown, drop cleanup, isolated mode and typed error variants.

### Changed
- `start_server()` now returns `Err` on a bind failure instead of calling
  `std::process::exit(1)`. A library must never terminate its host
  process. CLI output and exit code are unchanged — verified
  byte-identical against v0.2.11.
- README and installation docs present Docker, binary, source and library
  as peer options ordered by effort, rather than leading Docker-first.
- Copyright year updated to 2026.

### Notes
- Process-global initialisation (Prometheus recorder, tracing subscriber,
  allocator) stays in the binary. The library installs none of them, so it
  will not fight with a host application's own setup.
- The public API is deliberately small. Everything else sits behind a
  `#[doc(hidden)]` `internal` module that is explicitly **not** covered by
  semver.

## [0.2.11] - 2026-08-02
### Added
- Tool calling for the OpenAI-compatible providers — `azure-openai`,
  `mistral`, `openai-compatible`, `groq` and `openrouter` previously
  ignored `tools` entirely and echoed the request back. They now return a
  proper `tool_calls` response and terminate the agentic loop with a text
  synthesis once a tool result is in the history.
- VM-013 test suite covering all five providers in both modes.

### Fixed
- The shared OpenAI-compatible streaming chunk template emitted a tool
  call as the literal text `[[object]]` instead of a `tool_calls` delta —
  the same defect fixed for `/v1/responses` in 0.2.10.

### Notes
- Each provider keeps its own response envelope: Groq's timing fields and
  system fingerprint, OpenRouter's `provider` and `total_cost`.
- `groq` and `openrouter` declare no `stream:` block and remain
  non-streaming only.

## [0.2.10] - 2026-08-02
### Fixed
- **Streaming `/v1/responses` never returned a tool call**
  ([#8](https://github.com/vidaiUK/VidaiMock/issues/8)). A request
  carrying `tools` with `"stream": true` fell back to generic filler text
  while the identical non-streaming request correctly returned a
  `function_call`. The streaming templates had no tool branch, so the
  detected tool call was stringified as `[[object]]`.
- **Agentic tool loops never terminated** on `/v1/responses`. Branching
  was on the presence of `tools` alone, so replaying a tool result
  produced another tool call indefinitely — in both streaming and
  non-streaming modes.
- **Unstable stream ids.** Templates called `uuid()` per interpolation, so
  `resp_`/`msg_` ids differed across events within a single stream. Real
  OpenAI keeps them stable.

### Added
- VM-012 test suite for the Responses API across both modes and both
  turns of a tool loop.
- `tests/test_tool_calling_matrix.py` — a provider × mode × turn parity
  matrix asserting that any provider supporting tool calls does so in
  **both** streaming and non-streaming modes.

## [0.2.9] - 2026-05-24
### Added
- **Docker Compose setup** at [`docker/`](docker/) — `curl -O … && docker
  compose up` flow with optional `./overrides` mount for editing
  providers/templates, and `VIDAIMOCK_ISOLATED=true` env var to lock
  the surface down. See [docker/README.md](docker/README.md) and
  [Docker Compose recipe](https://vidai.uk/docs/mock/recipes/docker-compose/).
- `workflow_dispatch` on `.github/workflows/docker.yml` lets us push
  Docker-only RC images without re-running the entire Release pipeline
  (sources binaries from a previously published tarball).

### Changed
- README + mkdocs lead with Docker Compose as the recommended Docker
  path; bare `docker run` kept as the throwaway evaluator one-liner.
- No Rust code changes in this release — binary is byte-identical to
  v0.2.8. Docker image at `:0.2.9` and `:latest` cosign-signed against
  the same Vidai release key.

## [0.2.8] - 2026-05-24
### Added
- `--isolated` flag (also `VIDAIMOCK_ISOLATED` env / `isolated = true`
  in TOML) — skip embedded providers + templates and serve only what
  `--config-dir` declares. For production CI rigs and security audits.
  Closes issue #6.
- Signed multi-arch Docker image at `ghcr.io/vidaiuk/vidaimock` with
  cosign — `linux/amd64` + `linux/arm64`. Closes issue #5.
- Release tarballs (all 5 platforms) now ship with `.bundle` sidecar
  cosign signatures for the binary AND the tarball. Verifiable against
  the Vidai release key at `https://vidai.uk/.well-known/cosign.pub`.
- 404 response in isolated mode includes a mode-aware hint pointing
  users at `/status` for diagnostics.

### Changed
- `models_handler` no longer returns a misleading `gpt-4` fallback when
  no providers are loaded — returns an empty list instead.

## [Unreleased]
### Added
- Vertex AI provider with support for Google Cloud endpoint patterns.
- Robust Google Gemini AI Studio vs Vertex AI matching logic.
- Comprehensive documentation restructure (10+ new guides).
- Enhanced `extract_content_from_str` for better Gemini/Vertex streaming support.
- **Provider Priority**: New `priority` field in YAML configs for deterministic matching when patterns overlap.
- **Stable Context Variables**: `{{ uuid }}` and `{{ timestamp }}` (Number) are now stable across the entire request.

### Fixed
- Route conflict between Gemini POST and Anthropic GET paths.
- Tera template syntax for `random_int` (requires named arguments).
- Regression in template context variables (`uuid`, `timestamp`).

## [0.1.0] - 2025-12-15

### Added
- Initial release of VidaiMock
- Multi-provider support: OpenAI, Anthropic, Gemini, OpenRouter formats
- High-performance async server using Axum and Tokio
- `mimalloc` allocator for improved performance
- Latency simulation modes: `benchmark` (zero-latency) and `realistic` (configurable delay + jitter)
- Custom preset support via JSON files in `presets/` directory
- Custom response file override via `--response-file` flag
- Configurable endpoints via CLI or TOML config file
- Prometheus metrics endpoint (`/metrics`)
- Health check endpoint (`/health`)
- Status endpoint (`/status`)
- Echo handler for debugging
- Path traversal protection (security hardened)
- Fuzz testing with proptest
- Configurable bind address via `--host` flag
- Graceful shutdown on SIGTERM/SIGINT
- Structured JSON logging via tracing

### Security
- Path traversal protection tested and verified
- No `unsafe` code blocks
- Configurable network binding (localhost vs all interfaces)

### Documentation
- README with quick start guide
- USER_GUIDE with detailed configuration
- TUNING guide for performance optimization
- SECURITY.md for vulnerability reporting
- CONTRIBUTING.md for contributors
