/*
 * Copyright (c) 2026 Vidai UK.
 * Author: n@gu
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! VidaiMock as an embeddable mock server.
//!
//! Runs the same server, providers and templates as the `vidaimock` binary,
//! inside your test process — so integration tests do not have to download a
//! binary, pick a free port, spawn a process, poll for health, and tear it
//! down again.
//!
//! ```no_run
//! use vidaimock::MockServer;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let server = MockServer::builder()
//!     .bind("127.0.0.1:0")   // ephemeral port
//!     .start()
//!     .await?;
//!
//! // Point the system under test at this URL.
//! let base_url = server.base_url();
//!
//! server.shutdown().await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Parallel tests
//!
//! Instances share no global state, so any number can run concurrently. Bind
//! to port `0` and read the real address back with [`MockServer::addr`].
//!
//! # Logging and metrics
//!
//! The library deliberately does **not** install a tracing subscriber or a
//! Prometheus recorder — both are process-global and would conflict with the
//! host application's own setup. Install your own subscriber if you want the
//! server's logs.

use std::net::SocketAddr;
use std::path::PathBuf;

use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

mod aws_event_stream;
mod handlers;
mod replacer;

// Reachable from the binary via `internal`, hidden from docs and excluded
// from the public API contract. See the `internal` module below.
#[doc(hidden)]
pub mod config;
#[doc(hidden)]
pub mod provider;
#[doc(hidden)]
pub mod server;

/// Internals shared with the `vidaimock` binary.
///
/// **Not part of the public API.** Nothing here is covered by semver — it may
/// change or disappear in any release. It exists only because the binary and
/// the library are the same crate, and the binary needs `AppConfig::load()`
/// and `start_server()` to drive the CLI. Library users want [`MockServer`].
#[doc(hidden)]
pub mod internal {
    pub use crate::config;
    pub use crate::provider;
    pub use crate::server;
}

/// Errors returned when starting or stopping an embedded server.
///
/// `#[non_exhaustive]`: new variants may be added in a future release without
/// a breaking change, so `match` on this must include a catch-all arm.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The bind address could not be parsed or resolved.
    InvalidAddress { addr: String, source: std::io::Error },
    /// The address could not be bound — most often the port is already in use.
    Bind { addr: String, source: std::io::Error },
    /// Configuration could not be assembled.
    Config(String),
    /// The server task failed or panicked.
    Runtime(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::InvalidAddress { addr, source } => {
                write!(f, "invalid bind address '{}': {}", addr, source)
            }
            Error::Bind { addr, source } => {
                write!(f, "failed to bind {}: {} (is the port already in use?)", addr, source)
            }
            Error::Config(msg) => write!(f, "configuration error: {}", msg),
            Error::Runtime(msg) => write!(f, "server runtime error: {}", msg),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::InvalidAddress { source, .. } | Error::Bind { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Builds a [`MockServer`].
///
/// Fields are private so that new options can be added without breaking
/// callers. Obtain one with [`MockServer::builder`].
pub struct MockServerBuilder {
    bind: String,
    config_dir: PathBuf,
    isolated: bool,
}

impl MockServerBuilder {
    fn new() -> Self {
        Self {
            bind: "127.0.0.1:0".to_string(),
            config_dir: PathBuf::from("config"),
            isolated: false,
        }
    }

    /// Address to bind, e.g. `"127.0.0.1:0"` for an ephemeral port.
    ///
    /// Defaults to `127.0.0.1:0`. Prefer port `0` in tests so parallel runs
    /// never collide.
    pub fn bind(mut self, addr: impl Into<String>) -> Self {
        self.bind = addr.into();
        self
    }

    /// Directory holding provider and template overrides.
    ///
    /// Defaults to `config`. Files found here take precedence over the
    /// providers and templates embedded in the crate. A missing directory is
    /// not an error — the embedded defaults are used.
    pub fn config_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.config_dir = dir.into();
        self
    }

    /// Disable the embedded provider/template defaults, using only
    /// [`config_dir`](Self::config_dir). Mirrors `--isolated` on the CLI.
    pub fn isolated(mut self, isolated: bool) -> Self {
        self.isolated = isolated;
        self
    }

    /// Bind the listener and start serving.
    ///
    /// Returns as soon as the socket is bound, so the server is ready for
    /// requests the moment this resolves — no health polling required.
    pub async fn start(self) -> Result<MockServer, Error> {
        let listener = TcpListener::bind(&self.bind).await.map_err(|e| {
            // Distinguish "you typed it wrong" from "the port is taken";
            // they need different fixes.
            match e.kind() {
                std::io::ErrorKind::InvalidInput | std::io::ErrorKind::AddrNotAvailable => {
                    Error::InvalidAddress { addr: self.bind.clone(), source: e }
                }
                _ => Error::Bind { addr: self.bind.clone(), source: e },
            }
        })?;

        let addr = listener
            .local_addr()
            .map_err(|e| Error::Bind { addr: self.bind.clone(), source: e })?;

        let mut config = config::AppConfig::for_embedded(&self.config_dir, self.isolated)
            .map_err(|e| Error::Config(e.to_string()))?;
        // Report the real port on /status, matching CLI behaviour.
        config.port = addr.port();
        config.host = addr.ip().to_string();

        let registry = provider::init_registry_with_options(&self.config_dir, self.isolated);

        // `None` for metrics: install_recorder() is process-global and errors
        // on a second call, which would break parallel instances.
        let app = server::create_app(config, None, registry).await;

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await;
        });

        Ok(MockServer {
            addr,
            shutdown: Some(shutdown_tx),
            handle: Some(handle),
        })
    }
}

/// A running in-process VidaiMock server.
///
/// Dropping this stops the server on a best-effort basis; prefer
/// [`shutdown`](Self::shutdown) when you want to await termination.
///
/// Fields are private so that new state can be added without breaking
/// callers.
pub struct MockServer {
    addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<()>>,
}

impl MockServer {
    /// Start configuring a server.
    pub fn builder() -> MockServerBuilder {
        MockServerBuilder::new()
    }

    /// The address actually bound. With port `0` this is the ephemeral port
    /// the OS chose.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Base URL for the running server, e.g. `http://127.0.0.1:54321`.
    ///
    /// Append the provider path your client expects — `/v1` for OpenAI-style
    /// clients, the bare URL for Anthropic.
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Stop the server and wait for it to finish.
    ///
    /// Prefer this over dropping when the test needs the port released before
    /// it continues.
    pub async fn shutdown(mut self) -> Result<(), Error> {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.handle.take() {
            handle.await.map_err(|e| Error::Runtime(e.to_string()))?;
        }
        Ok(())
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        // Best-effort: signal shutdown and abort the task. We cannot await in
        // Drop, so callers who need to observe termination use shutdown().
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

impl std::fmt::Debug for MockServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockServer").field("addr", &self.addr).finish()
    }
}
