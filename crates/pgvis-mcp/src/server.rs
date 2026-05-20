//! MCP server implementation using the `rmcp` crate.
//!
//! [`McpServer`] implements `rmcp::ServerHandler`, bridging the existing
//! [`build_mcp_tools`](crate::tools::build_mcp_tools) and
//! [`handle_tool_call`](crate::tools::handle_tool_call) into the MCP protocol.
//!
//! # Example
//!
//! ```rust,no_run
//! use std::sync::Arc;
//! use arc_swap::ArcSwap;
//! use pgvis_core::{Config, SchemaCache, Backend, dialect::POSTGRES};
//! use pgvis_mcp::McpServer;
//!
//! // Assumes you have an Arc<dyn Backend> from pgvis-postgres or pgvis-sqlite
//! # fn example(backend: Arc<dyn Backend>) {
//! let cache = Arc::new(ArcSwap::new(Arc::new(SchemaCache::default())));
//! let config = Arc::new(Config::default());
//! let dialect = Arc::new(POSTGRES.clone());
//!
//! let server = McpServer::new(cache, config, dialect, backend);
//! // Pass `server` to a transport (stdio, streamable HTTP, etc.)
//! # }
//! ```

use std::sync::Arc;

use arc_swap::ArcSwap;
use rmcp::model::{
    Annotated, CallToolRequestParams, CallToolResult, Content, Implementation, InitializeResult,
    ListResourcesResult, ListToolsResult, PaginatedRequestParams, RawResource,
    ReadResourceRequestParams, ReadResourceResult, ResourceContents, ServerCapabilities, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::ServerHandler;

use pgvis_core::backend::Backend;
use pgvis_core::{Config, Dialect, SchemaCache};

use crate::tools::{build_mcp_resources, build_mcp_tools, handle_tool_call};
use crate::types::McpToolCall;

// ---------------------------------------------------------------------------
// McpServer
// ---------------------------------------------------------------------------

/// The pgvis MCP server — embeddable in any application.
///
/// Implements [`rmcp::ServerHandler`] by delegating to the existing
/// tool generation and execution logic in [`crate::tools`].
///
/// Create one with [`McpServer::new`], then attach a transport:
/// - [`crate::transport::serve_stdio`] for CLI / Claude Desktop
/// - [`crate::transport::streamable_http_service`] for HTTP/SSE
#[derive(Clone)]
pub struct McpServer {
    /// The schema cache — hot-swappable via `ArcSwap`.
    pub cache: Arc<ArcSwap<SchemaCache>>,
    /// Shared configuration.
    pub config: Arc<Config>,
    /// SQL dialect (Postgres capability flags).
    pub dialect: Arc<Dialect>,
    /// The database backend for query execution.
    pub backend: Arc<dyn Backend>,
}

impl McpServer {
    /// Create a new MCP server instance.
    ///
    /// The server reads the current `SchemaCache` snapshot on each request,
    /// so it automatically reflects schema changes when the cache is updated.
    pub fn new(
        cache: Arc<ArcSwap<SchemaCache>>,
        config: Arc<Config>,
        dialect: Arc<Dialect>,
        backend: Arc<dyn Backend>,
    ) -> Self {
        Self {
            cache,
            config,
            dialect,
            backend,
        }
    }
}

// ---------------------------------------------------------------------------
// ServerHandler implementation
// ---------------------------------------------------------------------------

impl ServerHandler for McpServer {
    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(
            Implementation::new("pgvis", env!("CARGO_PKG_VERSION"))
                .with_description("pgvis MCP server — database as MCP tools".to_string()),
        )
        .with_instructions(
            "This server exposes database tables and functions as MCP tools. \
             Use list_tools to discover available operations, then call them \
             with appropriate filters and parameters."
                .to_string(),
        )
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, rmcp::ErrorData>>
           + Send
           + '_ {
        async move {
            let cache = self.cache.load();
            let tools = build_mcp_tools(&cache, &self.config);

            let rmcp_tools: Vec<Tool> = tools
                .into_iter()
                .map(|t| {
                    let input_schema: serde_json::Map<String, serde_json::Value> =
                        t.input_schema.as_object().cloned().unwrap_or_default();

                    Tool::new(t.name, t.description, input_schema)
                })
                .collect();

            Ok(ListToolsResult {
                meta: None,
                tools: rmcp_tools,
                next_cursor: None,
            })
        }
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<CallToolResult, rmcp::ErrorData>>
           + Send
           + '_ {
        async move {
            let cache = self.cache.load();

            // Convert rmcp arguments to our McpToolCall format
            let arguments = request
                .arguments
                .map(serde_json::Value::Object)
                .unwrap_or(serde_json::Value::Null);

            let call = McpToolCall {
                name: request.name.to_string(),
                arguments,
            };

            let result = handle_tool_call(
                &call,
                &cache,
                &self.dialect,
                &self.config,
                &*self.backend,
            )
            .await;

            // Convert our McpToolResult to rmcp's CallToolResult
            let content: Vec<Content> = result
                .content
                .into_iter()
                .map(|c| match c {
                    crate::types::McpContent::Text { text } => Content::text(text),
                })
                .collect();

            if result.is_error {
                Ok(CallToolResult::error(content))
            } else {
                Ok(CallToolResult::success(content))
            }
        }
    }

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListResourcesResult, rmcp::ErrorData>>
           + Send
           + '_ {
        async move {
            let cache = self.cache.load();
            let resources = build_mcp_resources(&cache, &self.config);

            let rmcp_resources = resources
                .into_iter()
                .map(|r| {
                    Annotated::new(
                        RawResource {
                            uri: r.uri,
                            name: r.name,
                            title: None,
                            description: Some(r.description),
                            mime_type: r.mime_type,
                            size: None,
                            icons: None,
                            meta: None,
                        },
                        None,
                    )
                })
                .collect();

            Ok(ListResourcesResult {
                meta: None,
                resources: rmcp_resources,
                next_cursor: None,
            })
        }
    }

    fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ReadResourceResult, rmcp::ErrorData>>
           + Send
           + '_ {
        async move {
            let cache = self.cache.load();
            let uri = request.uri.as_str();

            let content = match uri {
                "pgvis://schemas" => {
                    serde_json::to_string_pretty(&self.config.schemas).unwrap_or_default()
                }
                _ if uri.starts_with("pgvis://") && uri.ends_with("/schema") => {
                    // Extract schema name: pgvis://{schema}/schema
                    let schema = uri
                        .strip_prefix("pgvis://")
                        .and_then(|s| s.strip_suffix("/schema"))
                        .unwrap_or("public");

                    let tables: Vec<&str> = cache
                        .tables
                        .values()
                        .filter(|t| t.schema() == schema)
                        .map(|t| t.name())
                        .collect();

                    serde_json::to_string_pretty(&serde_json::json!({
                        "schema": schema,
                        "tables": tables,
                    }))
                    .unwrap_or_default()
                }
                _ if uri.starts_with("pgvis://") && uri.ends_with("/columns") => {
                    // Extract schema.table: pgvis://{schema}/{table}/columns
                    let path = uri
                        .strip_prefix("pgvis://")
                        .and_then(|s| s.strip_suffix("/columns"))
                        .unwrap_or("");

                    let parts: Vec<&str> = path.splitn(2, '/').collect();
                    let (schema, table) = match parts.as_slice() {
                        [s, t] => (*s, *t),
                        _ => ("public", path),
                    };

                    let columns: Vec<serde_json::Value> = cache
                        .tables
                        .values()
                        .find(|t| t.schema() == schema && t.name() == table)
                        .map(|t| {
                            t.columns
                                .values()
                                .map(|c| {
                                    serde_json::json!({
                                        "name": c.name,
                                        "type": c.typ,
                                        "nullable": c.nullable,
                                        "has_default": c.default.is_some(),
                                    })
                                })
                                .collect()
                        })
                        .unwrap_or_default();

                    serde_json::to_string_pretty(&columns).unwrap_or_default()
                }
                _ => {
                    return Err(rmcp::ErrorData::new(
                        rmcp::model::ErrorCode::INVALID_PARAMS,
                        format!("Unknown resource URI: {uri}"),
                        None,
                    ));
                }
            };

            Ok(ReadResourceResult::new(vec![ResourceContents::text(
                content, uri,
            )]))
        }
    }
}
