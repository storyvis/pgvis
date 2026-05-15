//! `pgvis-mcp` — MCP (Model Context Protocol) adapter for pgvis.
//!
//! Exposes database tables and functions as MCP tools that mirror the REST
//! route structure. Both adapters share the same `plan_request()` pipeline.
//!
//! # Architecture
//!
//! ```text
//! SchemaCache + Config
//!       │
//!       ├──► pgvis-rest::build_app()      → axum Router (REST routes)
//!       │
//!       └──► pgvis-mcp::build_mcp_tools() → Vec<McpToolDefinition> (MCP tools)
//!                    │
//!                    └──► handle_tool_call() → ApiRequest → plan_request() → execute
//! ```

pub mod tools;
pub mod types;

pub use tools::{build_mcp_resources, build_mcp_tools, handle_tool_call};
pub use types::*;
