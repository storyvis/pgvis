//! MCP type definitions for tool and resource descriptions.
//!
//! These types represent the MCP protocol structures. They are kept as simple
//! serde-compatible structs that can be serialized to the MCP JSON-RPC format.

use serde::{Deserialize, Serialize};

/// An MCP tool definition — describes a callable operation.
///
/// Corresponds to MCP's `Tool` type in the protocol spec.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolDefinition {
    /// Tool name — schema-namespaced (e.g., `public/list_users`).
    pub name: String,
    /// Human-readable description for LLM discovery.
    pub description: String,
    /// JSON Schema describing the tool's input parameters.
    #[serde(rename = "inputSchema")]
    pub input_schema: serde_json::Value,
}

/// An MCP resource — read-only data for schema discovery.
///
/// Corresponds to MCP's `Resource` type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpResource {
    /// Resource URI (e.g., `pgvis://public/schema`).
    pub uri: String,
    /// Human-readable name.
    pub name: String,
    /// Description of what this resource provides.
    pub description: String,
    /// Optional MIME type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

/// An MCP tool call — incoming request to invoke a tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolCall {
    /// The tool name being invoked.
    pub name: String,
    /// The arguments provided by the caller.
    #[serde(default)]
    pub arguments: serde_json::Value,
}

/// The result of an MCP tool invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolResult {
    /// Content items in the result.
    pub content: Vec<McpContent>,
    /// Whether the tool execution resulted in an error.
    #[serde(default, rename = "isError")]
    pub is_error: bool,
}

/// A content item in an MCP tool result.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum McpContent {
    /// JSON data content.
    #[serde(rename = "text")]
    Text {
        /// The text/JSON content.
        text: String,
    },
}

impl McpToolResult {
    /// Create a successful result with JSON content.
    pub fn success(data: serde_json::Value) -> Self {
        Self {
            content: vec![McpContent::Text {
                text: serde_json::to_string_pretty(&data).unwrap_or_default(),
            }],
            is_error: false,
        }
    }

    /// Create a successful result with a pre-formatted text string.
    pub fn success_text(text: String) -> Self {
        Self {
            content: vec![McpContent::Text { text }],
            is_error: false,
        }
    }

    /// Create an error result.
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            content: vec![McpContent::Text {
                text: message.into(),
            }],
            is_error: true,
        }
    }

    /// Create a structured error result mirroring the REST surface's
    /// PostgREST-compatible error shape.
    ///
    /// Emits a single text content item containing a JSON object:
    ///
    /// ```json
    /// {
    ///   "error": {
    ///     "code": "PGRST200",
    ///     "message": "...",
    ///     "details": "...",
    ///     "hint": "..."
    ///   }
    /// }
    /// ```
    ///
    /// Tool clients can parse this and surface code/details/hint to the
    /// model, instead of getting a stringified `[PGRST200] ...` blob.
    pub fn error_structured(
        code: &str,
        message: impl Into<String>,
        details: Option<String>,
        hint: Option<String>,
    ) -> Self {
        let body = serde_json::json!({
            "error": {
                "code": code,
                "message": message.into(),
                "details": details,
                "hint": hint,
            }
        });
        Self {
            content: vec![McpContent::Text {
                text: serde_json::to_string_pretty(&body).unwrap_or_else(|_| body.to_string()),
            }],
            is_error: true,
        }
    }

    /// Convert a [`pgvis_core::error::Error`] into a structured MCP error
    /// result, preserving the PGRST* code and any database-supplied details.
    pub fn from_core_error(err: &pgvis_core::error::Error) -> Self {
        match err {
            pgvis_core::error::Error::Execution {
                message,
                db_code,
                detail,
                hint,
            } => Self::error_structured(
                db_code.as_deref().unwrap_or(err.code().as_str()),
                message.clone(),
                detail.clone(),
                hint.clone(),
            ),
            pgvis_core::error::Error::Plan {
                message,
                detail,
                hint,
                ..
            } => Self::error_structured(
                err.code().as_str(),
                message.clone(),
                detail.clone(),
                hint.clone(),
            ),
            pgvis_core::error::Error::Parse {
                message, detail, ..
            } => Self::error_structured(
                err.code().as_str(),
                message.clone(),
                detail.clone(),
                None,
            ),
            other => Self::error_structured(
                other.code().as_str(),
                other.to_string(),
                None,
                None,
            ),
        }
    }
}
