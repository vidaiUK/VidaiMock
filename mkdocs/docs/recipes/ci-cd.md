---
title: CI/CD Integration
---

# CI/CD Integration

VidaiMock's whole reason for existing is to make AI integration tests
**deterministic, free, and offline**. In CI, the recommended way to run
it is the same Docker image you used locally, pinned by digest for
reproducibility. For environments that can't run Docker, the static
binary works identically — see the alternative pattern below.

## The pattern (Docker, recommended)

```bash
# 1. Start the mock in the background, pinned by digest
docker run -d --name vidaimock -p 8100:8100 \
  ghcr.io/vidaiuk/vidaimock:latest@sha256:<pin-to-digest>

# 2. Wait for liveness
until curl -sf http://localhost:8100/health >/dev/null; do sleep 0.1; done

# 3. Point your app's provider base URL at the mock
export OPENAI_BASE_URL=http://localhost:8100/v1
export ANTHROPIC_BASE_URL=http://localhost:8100

# 4. Run your test suite
pytest

# 5. Tear down
docker rm -f vidaimock
```

No API keys needed — VidaiMock ignores `Authorization` on mock routes.

!!! tip "Rust projects"
    If your tests are in Rust, you can skip this orchestration entirely —
    [embed the server in your test process](rust-library.md) and drop the
    start/wait/teardown steps above.

### Why pin by digest

A digest-pinned image is bit-identical every CI run, no matter what the
`:latest` tag points at today. Once you've verified the digest with
cosign, every subsequent pull of that same digest stays verified.

```bash
# Verify once, write the digest into your workflow file
cosign verify \
  --key https://vidai.uk/.well-known/cosign.pub \
  --insecure-ignore-tlog \
  ghcr.io/vidaiuk/vidaimock:latest
# -> the manifest digest in the output is what you pin
```

See [Installation → Verify release signatures](../getting-started/installation.md#verify-release-signatures-cosign)
for the full trust model.

## Zero-token agentic CI

Because VidaiMock terminates tool-calling loops correctly (see
[Agentic workflow testing](../agentic-testing.md)), full ADK / LangGraph /
LangChain Runner tests run start-to-finish in CI without a single live
token. This is the difference between "we test our agent logic on every PR"
and "we test it manually before release because real tokens cost money."

## Resilience / fallback testing

Verify retry and circuit-breaker logic by registering a forced-failure
upstream alongside a healthy one — same instance, different URL:

```
primary  endpoint: http://localhost:8100/v1?chaos_status=500
fallback endpoint: http://localhost:8100/v1
```

Or inject probabilistic chaos for soak tests:

```bash
./vidaimock --port 8100 &
# 5% of requests fail with a provider-shaped 500, 5% of streams disconnect
curl -H "X-Vidai-Chaos-Drop: 5" ...
```

See [Chaos & error injection](../chaos-and-errors.md).

## Per-pipeline custom behaviour

Ship a `--config-dir` with your test fixtures so the mock returns exactly
what a given suite needs, without touching the binary:

```bash
./vidaimock --config-dir ./tests/mock-fixtures --port 8100 &
```

See [Overriding bundled defaults](../configuration/overriding.md).

## GitHub Actions sketch (Docker)

```yaml
- name: Start VidaiMock
  run: |
    docker run -d --name vidaimock -p 8100:8100 \
      ghcr.io/vidaiuk/vidaimock:latest@sha256:<pin-to-digest>
    until curl -sf http://localhost:8100/health >/dev/null; do sleep 0.1; done

- name: Run tests
  env:
    OPENAI_BASE_URL: http://localhost:8100/v1
    ANTHROPIC_BASE_URL: http://localhost:8100
  run: pytest -q

- name: Stop mock
  if: always()
  run: docker rm -f vidaimock
```

The image is multi-arch (linux/amd64 + linux/arm64), distroless, and
~25 MB. Pulled-once-and-cached by your CI runner; first pull adds a
few seconds, subsequent runs are near-instant.

## Alternative: static binary

For environments that can't run Docker (some restricted CI runners,
airgapped builds, lambda-style executors), the binary works identically:

```bash
# 1. Start the mock in the background
./vidaimock --port 8100 &
MOCK_PID=$!

# 2. Wait for liveness
until curl -sf http://localhost:8100/health >/dev/null; do sleep 0.1; done

# 3. Run tests as above
pytest

# 4. Tear down
kill $MOCK_PID
```

### GitHub Actions sketch (binary)

```yaml
- name: Start VidaiMock
  run: |
    curl -LO https://github.com/vidaiUK/VidaiMock/releases/latest/download/vidaimock-linux-x64.tar.gz
    tar -xzf vidaimock-linux-x64.tar.gz
    ./vidaimock/vidaimock --port 8100 &
    until curl -sf http://localhost:8100/health; do sleep 0.1; done

- name: Run tests
  env:
    OPENAI_BASE_URL: http://localhost:8100/v1
  run: pytest -q
```

The binary is ~7 MB and starts instantly. Same wire behaviour as the
Docker image — they ship from the same release commit.

## Why not a recorded-cassette mock?

Cassette/VCR-style mocks replay static captures. They can't simulate
streaming physics, won't terminate an agentic loop, and drift the moment
the provider changes a field. VidaiMock generates responses dynamically
from templates that are regression-tested byte-level against real captures —
so it stays accurate without re-recording.
