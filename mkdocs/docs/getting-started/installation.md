---
title: Installation
---

# Installation

Three equal-status install paths — Docker, prebuilt binary, or build from
source. Pick whichever fits your workflow.

## Docker

Multi-arch signed image (`linux/amd64` + `linux/arm64`), distroless runtime,
~25 MB.

=== "Docker Compose (recommended)"

    ```bash
    curl -O https://raw.githubusercontent.com/vidaiUK/VidaiMock/main/docker/docker-compose.yml
    docker compose up -d
    curl http://localhost:8100/health    # {"status":"ok"}
    ```

    Proper restart policy, override-friendly via `./overrides/` next to the
    compose file, isolated-mode via one env var. See
    [Docker Compose recipe](../recipes/docker-compose.md) for the full flow.

=== "One-liner"

    ```bash
    docker run --rm -p 8100:8100 ghcr.io/vidaiuk/vidaimock:latest
    ```

    Throwaway; no overrides, no isolated mode. Useful for quick evaluation
    only. For anything beyond `curl`-and-throw, use the compose flow above.

For CI use, pin to a specific digest for reproducibility — see
[CI/CD Integration](../recipes/ci-cd.md).

## Binary download

Each archive extracts to a `vidaimock/` directory containing the binary plus
`config/` and `examples/`.

=== "macOS (Apple Silicon)"

    ```bash
    curl -LO https://github.com/vidaiUK/VidaiMock/releases/latest/download/vidaimock-macos-arm64.tar.gz
    tar -xzf vidaimock-macos-arm64.tar.gz && cd vidaimock
    ./vidaimock
    ```

=== "macOS (Intel)"

    ```bash
    curl -LO https://github.com/vidaiUK/VidaiMock/releases/latest/download/vidaimock-macos-x64.tar.gz
    tar -xzf vidaimock-macos-x64.tar.gz && cd vidaimock
    ./vidaimock
    ```

=== "Linux (ARM64)"

    ```bash
    curl -LO https://github.com/vidaiUK/VidaiMock/releases/latest/download/vidaimock-linux-arm64.tar.gz
    tar -xzf vidaimock-linux-arm64.tar.gz && cd vidaimock
    ./vidaimock
    ```

=== "Linux (x64)"

    ```bash
    curl -LO https://github.com/vidaiUK/VidaiMock/releases/latest/download/vidaimock-linux-x64.tar.gz
    tar -xzf vidaimock-linux-x64.tar.gz && cd vidaimock
    ./vidaimock
    ```

=== "Windows (x64)"

    ```powershell
    Invoke-WebRequest -Uri https://github.com/vidaiUK/VidaiMock/releases/latest/download/vidaimock-windows-x64.zip -OutFile vidaimock-windows-x64.zip
    Expand-Archive vidaimock-windows-x64.zip -DestinationPath .
    cd vidaimock
    .\vidaimock.exe
    ```

!!! warning "OS security notice (macOS / Windows)"
    Because VidaiMock is an unsigned open-source binary, your OS may block it
    on first run.

    - **macOS**: `xattr -d com.apple.quarantine vidaimock`
    - **Windows**: click *More info* in the SmartScreen dialog, then *Run anyway*

## Build from source

Requires a recent stable Rust toolchain (1.70+).

```bash
git clone https://github.com/vidaiUK/VidaiMock.git
cd VidaiMock
cargo build --release
./target/release/vidaimock
```

The bundled `config/` (providers + templates) is embedded into the binary at
compile time, so the binary works standalone with no files alongside it. A
local `config/` directory or `--config-dir` only *overrides* the embedded
defaults — see [Overriding bundled defaults](../configuration/overriding.md).

## Rust library

Rust projects can embed the server directly in their tests rather than running
it as a separate process:

```toml
[dev-dependencies]
vidaimock = "0.3"
```

```rust
use vidaimock::MockServer;

let server = MockServer::builder().bind("127.0.0.1:0").start().await?;
let base_url = server.base_url();
```

This is the same server, providers and templates as the binary — it simply runs
inside your test process. See [Rust Library](../recipes/rust-library.md) for
parallel tests, config overrides, and shutdown semantics.

## Verify release signatures (cosign)

Every release artefact — the Docker image, the tarball, and the bare binary
inside it — is signed with the Vidai release key, published at
**<https://vidai.uk/.well-known/cosign.pub>**. The key is served over
Vidai-controlled TLS, a separate trust path from GitHub and GHCR — an
attacker who tampers with an artefact would also have to compromise
`vidai.uk` to swap the trust anchor.

=== "Verify the Docker image"

    ```bash
    cosign verify \
      --key https://vidai.uk/.well-known/cosign.pub \
      --insecure-ignore-tlog \
      ghcr.io/vidaiuk/vidaimock:latest
    ```

=== "Verify a downloaded tarball"

    ```bash
    cosign verify-blob \
      --key https://vidai.uk/.well-known/cosign.pub \
      --insecure-ignore-tlog \
      --bundle vidaimock-linux-x64.tar.gz.bundle \
      vidaimock-linux-x64.tar.gz
    ```

=== "Verify the bare binary"

    The binary's `.bundle` ships inside the tarball, so you can verify the
    extracted binary even after deleting the archive.

    ```bash
    cosign verify-blob \
      --key https://vidai.uk/.well-known/cosign.pub \
      --insecure-ignore-tlog \
      --bundle vidaimock.bundle \
      vidaimock
    ```

The `--insecure-ignore-tlog` flag is required because VidaiMock does not
publish to the Sigstore transparency log — the trust anchor is the
keyed signature against the `vidai.uk` public key, not a public log
entry. The flag name sounds scary but is correct for keyed (non-keyless)
cosign flows.

## Smoke check

```bash
./vidaimock --version
curl -s http://localhost:8100/health      # {"status":"ok"} once running
```

Next: [Quickstart](quickstart.md).
