---
title: Rust Library
---

# Rust Library (in-process testing)

Rust projects can embed VidaiMock directly in their integration tests. Instead
of downloading a binary, picking a free port, spawning a process, polling
`/health`, and tearing it all down again, you get a server your test owns.

!!! note "API stability"
    The library API is new in `0.3.0`. While the crate is `0.x` it may change
    between minor versions — pin a version you have tested against.

## Install

```toml
[dev-dependencies]
vidaimock = "0.3"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

## Basic usage

```rust
use vidaimock::MockServer;

#[tokio::test]
async fn agent_calls_openai() -> Result<(), Box<dyn std::error::Error>> {
    let server = MockServer::builder()
        .bind("127.0.0.1:0")     // ephemeral port
        .start()
        .await?;

    // Point the system under test at the mock.
    let base_url = server.base_url();   // e.g. http://127.0.0.1:54321
    std::env::set_var("OPENAI_BASE_URL", format!("{base_url}/v1"));

    // ... exercise your code, then assert ...

    server.shutdown().await?;
    Ok(())
}
```

`start()` returns once the socket is bound, so the server is ready the moment
it resolves — **no health polling needed**.

## Why port 0

Binding `127.0.0.1:0` lets the OS pick a free port, which is what makes
parallel tests safe. Read the real address back with `addr()` or `base_url()`:

```rust
let server = MockServer::builder().bind("127.0.0.1:0").start().await?;
println!("listening on {}", server.addr());   // 127.0.0.1:54321
```

Hardcoding a port makes tests collide the moment two run at once, or when a
developer already has something on that port.

## Parallel tests

Instances share no global state, so any number can run concurrently — which is
exactly how `cargo test` runs your tests by default:

```rust
#[tokio::test]
async fn test_one() {
    let server = MockServer::builder().bind("127.0.0.1:0").start().await.unwrap();
    // ... independent of every other test ...
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_two() {
    let server = MockServer::builder().bind("127.0.0.1:0").start().await.unwrap();
    // ... runs at the same time, its own port, its own state ...
    server.shutdown().await.unwrap();
}
```

## Shutdown

Two options:

```rust
server.shutdown().await?;   // stops and waits for termination
```

```rust
{
    let server = MockServer::builder().bind("127.0.0.1:0").start().await?;
    // ...
}   // dropped here — best-effort stop, not awaited
```

Prefer `shutdown()` when the test needs the port released before it continues.
`Drop` is a safety net for panics and early returns, not the primary path.

## Custom providers and templates

The bundled providers and templates are embedded in the crate, so the default
setup needs no files on disk. To override them, point at a config directory:

```rust
let server = MockServer::builder()
    .bind("127.0.0.1:0")
    .config_dir("tests/fixtures/providers")
    .start()
    .await?;
```

To use **only** your own definitions and disable the embedded defaults
(equivalent to `--isolated` on the CLI):

```rust
let server = MockServer::builder()
    .bind("127.0.0.1:0")
    .config_dir("tests/fixtures/providers")
    .isolated(true)
    .start()
    .await?;
```

See [Overriding bundled defaults](../configuration/overriding.md) for how the
two layers interact.

## Error handling

Startup problems are returned, never fatal — a library must not terminate its
host process:

```rust
match MockServer::builder().bind("127.0.0.1:8100").start().await {
    Ok(server) => { /* ... */ }
    Err(vidaimock::Error::Bind { addr, .. }) => {
        eprintln!("port busy: {addr}");
    }
    Err(e) => eprintln!("failed to start: {e}"),
}
```

`vidaimock::Error` is `#[non_exhaustive]`, so `match` on it needs a catch-all
arm. That lets new variants be added without breaking your code.

## Logging

The library deliberately does **not** install a tracing subscriber — that is
process-global and would fight with your application's own setup. To see the
server's logs, install your own:

```rust
tracing_subscriber::fmt().with_env_filter("vidaimock=debug").init();
```

Prometheus metrics are likewise not installed by the library; the `/metrics`
endpoint is a feature of the binary.

## When to use the binary instead

| Use | Reach for |
|---|---|
| Rust integration tests | This library |
| Non-Rust test suites | [Docker](docker-compose.md) or the binary |
| Shared instance across a team or CI job | [Docker](docker-compose.md) |
| Manual exploration with `curl` | The binary |
| Language-agnostic CI service container | [CI/CD Integration](ci-cd.md) |
