//! Transport builders for the pgvis MCP server.
//!
//! Provides ready-to-use functions that wire [`McpServer`](crate::McpServer)
//! to MCP transports provided by the `rmcp` crate.
//!
//! ## Stdio Transport (Claude Desktop / CLI agents)
//!
//! ```rust,no_run
//! # use std::sync::Arc;
//! # use arc_swap::ArcSwap;
//! # use pgvis_core::{Config, SchemaCache, Backend, dialect::POSTGRES};
//! use pgvis_mcp::{McpServer, transport::serve_stdio};
//!
//! # async fn example(backend: Arc<dyn Backend>) -> Result<(), Box<dyn std::error::Error>> {
//! # let cache = Arc::new(ArcSwap::new(Arc::new(SchemaCache::default())));
//! # let config = Arc::new(Config::default());
//! # let dialect = Arc::new(POSTGRES.clone());
//! let server = McpServer::new(cache, config, dialect, backend);
//! serve_stdio(server).await?;
//! # Ok(())
//! # }
//! ```
//!
//! ## Streamable HTTP Transport (hosted deployments)
//!
//! ```rust,no_run
//! # use std::sync::Arc;
//! # use arc_swap::ArcSwap;
//! # use pgvis_core::{Config, SchemaCache, Backend, dialect::POSTGRES};
//! use pgvis_mcp::{McpServer, transport::streamable_http_service};
//!
//! # fn example(backend: Arc<dyn Backend>) {
//! # let cache = Arc::new(ArcSwap::new(Arc::new(SchemaCache::default())));
//! # let config = Arc::new(Config::default());
//! # let dialect = Arc::new(POSTGRES.clone());
//! let server = McpServer::new(cache, config, dialect, backend);
//! let mcp_service = streamable_http_service(server);
//! // Mount `mcp_service` at e.g. `/mcp` alongside your REST routes
//! # }
//! ```

use std::sync::Arc;

use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};

use crate::McpServer;

/// Start the MCP server over stdio (stdin/stdout).
///
/// This blocks the current task until the transport closes (client disconnects
/// or EOF on stdin). Typically called from `pgvis mcp` CLI subcommand.
///
/// # Errors
///
/// Returns an error if the MCP initialization handshake fails or the transport
/// encounters an I/O error.
pub async fn serve_stdio(server: McpServer) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let transport = rmcp::transport::io::stdio();
    let service = rmcp::serve_server(server, transport).await?;
    service.waiting().await?;
    Ok(())
}

/// Create a tower `Service` that serves MCP over the Streamable HTTP transport.
///
/// The returned service handles the MCP JSON-RPC protocol over HTTP POST/GET
/// with SSE streaming. Mount it at a path (e.g., `/mcp`) using axum or any
/// tower-compatible router.
///
/// Uses a [`LocalSessionManager`] for in-process session management with
/// sensible defaults. Each incoming session spawns a new handler backed by
/// a clone of the provided [`McpServer`].
///
/// # Example
///
/// ```rust,no_run
/// use axum::Router;
/// # use std::sync::Arc;
/// # use arc_swap::ArcSwap;
/// # use pgvis_core::{Config, SchemaCache, Backend, dialect::POSTGRES};
/// # use pgvis_mcp::{McpServer, transport::streamable_http_service};
///
/// # fn example(backend: Arc<dyn Backend>) {
/// # let cache = Arc::new(ArcSwap::new(Arc::new(SchemaCache::default())));
/// # let config = Arc::new(Config::default());
/// # let dialect = Arc::new(POSTGRES.clone());
/// let server = McpServer::new(cache, config, dialect, backend);
/// let mcp_svc = streamable_http_service(server);
/// // Use with hyper/axum at a specific path
/// # }
/// ```
pub fn streamable_http_service(
    server: McpServer,
) -> StreamableHttpService<McpServer, LocalSessionManager> {
    let config = StreamableHttpServerConfig::default();
    let session_manager = Arc::new(LocalSessionManager::default());

    StreamableHttpService::new(
        move || Ok(server.clone()),
        session_manager,
        config,
    )
}
