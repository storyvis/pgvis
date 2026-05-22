//! # Shared configuration types used by backends and adapters.
//!
//! These types define the **common configuration surface** that all pgvis crates
//! agree on. Backend-specific config (pool size, connection timeouts) lives in
//! the driver crates. Adapter-specific config (bind address, CORS) lives in
//! the adapter crates.
//!
//! ## Configuration Hierarchy
//!
//! ```text
//! pgvis-core::Config         — shared by all (schemas, auth, features)
//! pgvis-postgres::PgConfig   — pool settings, LISTEN channel name
//! pgvis-rest::RestConfig     — bind addr, CORS, OpenAPI mode
//! pgvis-server::ServerConfig — CLI args, figment layering
//! ```
//!
//! ## PostgREST Comparison
//!
//! PostgREST has a single flat `AppConfig` with ~60 fields. pgvis splits this
//! across crates so library consumers only see what's relevant to them.

use serde::{Deserialize, Serialize};

/// Configuration for URL routing and namespace mapping.
///
/// Controls how REST routes are structured and how MCP tools are named.
/// Both adapters read this to determine their namespace hierarchy.
///
/// # Routing Modes
///
/// | Mode | Example URL | Config |
/// |------|-------------|--------|
/// | Full path | `/api/public/users` | `schema_in_path = true` |
/// | Prefix only | `/api/users` | `schema_in_path = false` |
/// | Compat | `/users` | `schema_in_path = false, prefix = ""` |
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingConfig {
    /// URL prefix for all API routes.
    ///
    /// Examples: `"api"`, `"v1"`, `""` (empty for PostgREST compat).
    /// Leading/trailing slashes are stripped automatically.
    ///
    /// REST: routes are mounted under `/{prefix}/...`
    /// MCP: prefix is not used in tool names.
    #[serde(default = "default_routing_prefix")]
    pub prefix: String,

    /// Whether the schema name appears in the URL path.
    ///
    /// When `true`: `/{prefix}/{schema}/{table}` (recommended)
    /// When `false`: `/{prefix}/{table}` with schema from header/default (PostgREST compat)
    ///
    /// Defaults to `true` for new projects.
    #[serde(default = "default_schema_in_path")]
    pub schema_in_path: bool,

    /// The default schema used when `schema_in_path = false`.
    ///
    /// Requests without an explicit schema header use this schema.
    /// Must be one of the schemas listed in `Config::schemas`.
    ///
    /// Defaults to `"public"`.
    #[serde(default = "default_default_schema")]
    pub default_schema: String,

    /// Separator character for MCP tool names.
    ///
    /// Tool names are formatted as `{schema}{separator}{verb}_{table}`.
    /// `/` maps cleanly to hierarchical tool discovery.
    /// `.` is an alternative for flat tool lists.
    ///
    /// Defaults to `'/'`.
    #[serde(default = "default_mcp_separator")]
    pub mcp_separator: char,
}

impl RoutingConfig {
    /// Generate an MCP tool name from schema, verb, and target.
    ///
    /// When `schema_in_path = true`, always includes the schema prefix.
    /// When `schema_in_path = false`, omits the prefix for the default schema.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgvis_core::config::RoutingConfig;
    ///
    /// let config = RoutingConfig::default();
    /// assert_eq!(config.mcp_tool_name("public", "list", "users"), "public/list_users");
    /// assert_eq!(config.mcp_tool_name("internal", "call", "rotate"), "internal/call_rotate");
    /// ```
    pub fn mcp_tool_name(&self, schema: &str, verb: &str, target: &str) -> String {
        if self.schema_in_path || schema != self.default_schema {
            format!("{schema}{sep}{verb}_{target}", sep = self.mcp_separator)
        } else {
            format!("{verb}_{target}")
        }
    }

    /// Build the URL path prefix for a given schema.
    ///
    /// Returns the path segment(s) that precede the table/rpc name.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgvis_core::config::RoutingConfig;
    ///
    /// let config = RoutingConfig::default();
    /// assert_eq!(config.schema_path_prefix("public"), "/api/public");
    ///
    /// let compat = RoutingConfig { prefix: String::new(), schema_in_path: false, ..Default::default() };
    /// assert_eq!(compat.schema_path_prefix("public"), "");
    /// ```
    pub fn schema_path_prefix(&self, schema: &str) -> String {
        let prefix_part = if self.prefix.is_empty() {
            String::new()
        } else {
            format!("/{}", self.prefix)
        };

        if self.schema_in_path {
            format!("{prefix_part}/{schema}")
        } else {
            prefix_part
        }
    }

    /// Normalize the prefix by stripping leading/trailing slashes.
    pub fn normalized_prefix(&self) -> &str {
        self.prefix.trim_matches('/')
    }
}

impl Default for RoutingConfig {
    fn default() -> Self {
        Self {
            prefix: default_routing_prefix(),
            schema_in_path: default_schema_in_path(),
            default_schema: default_default_schema(),
            mcp_separator: default_mcp_separator(),
        }
    }
}

/// Shared configuration consumed by backends and adapters.
///
/// This is the "inner config" that both the REST adapter and the Backend
/// implementation need to agree on. It contains:
/// - Schema selection (which schemas to expose)
/// - Authentication settings (JWT secret, anonymous role)
/// - Feature gates (aggregates, plan endpoint)
/// - Pre-request hook
///
/// # PostgREST equivalents
///
/// | pgvis field | PostgREST config key |
/// |---|---|
/// | `schemas` | `db-schemas` |
/// | `extra_search_path` | `db-extra-search-path` |
/// | `anon_role` | `db-anon-role` |
/// | `jwt_secret` | `jwt-secret` |
/// | `pre_request` | `db-pre-request` |
/// | `aggregates_enabled` | `db-aggregates-enabled` |
/// | `max_rows` | `max-rows` |
/// | `statement_timeout` | `db-plan-enabled` (partially) |
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    // --- Schema selection ---

    /// Which schemas to expose as API endpoints.
    ///
    /// Tables, views, and functions in these schemas become REST routes and
    /// MCP tools. Defaults to `["public"]`.
    ///
    /// PostgREST equivalent: `db-schemas`.
    #[serde(default = "default_schemas")]
    pub schemas: Vec<String>,

    /// Additional schemas for type/function resolution (not exposed as endpoints).
    ///
    /// PostgREST equivalent: `db-extra-search-path`.
    #[serde(default)]
    pub extra_search_path: Vec<String>,

    // --- Authentication ---

    /// JWT secret for token verification.
    ///
    /// If `None`, JWT verification is disabled (all requests are anonymous).
    /// Can be a symmetric secret (HS256) or a path to a public key (RS256).
    ///
    /// PostgREST equivalent: `jwt-secret`.
    pub jwt_secret: Option<String>,

    /// JWT signing algorithm. Defaults to `HS256`.
    ///
    /// PostgREST equivalent: `jwt-secret-is-base64` + implicit algorithm detection.
    #[serde(default = "default_jwt_algo")]
    pub jwt_algo: JwtAlgorithm,

    /// The role used for unauthenticated requests.
    ///
    /// On Postgres, this role's permissions define what anonymous users can access.
    /// On SQLite, this is informational only (no role system).
    ///
    /// PostgREST equivalent: `db-anon-role`.
    pub anon_role: Option<String>,

    /// The JWT claim key that specifies the database role.
    ///
    /// Defaults to `"role"`. The value of this claim in the JWT becomes the
    /// `SET LOCAL role` value for the transaction.
    ///
    /// PostgREST equivalent: `jwt-role-claim-key`.
    #[serde(default = "default_role_claim_key")]
    pub role_claim_key: String,

    // --- Feature gates ---

    /// Whether aggregate functions (`sum`, `avg`, etc.) are enabled in `select`.
    ///
    /// Disabled by default (matching PostgREST) because aggregates can be
    /// expensive on large tables without proper indexes.
    ///
    /// PostgREST equivalent: `db-aggregates-enabled`.
    #[serde(default)]
    pub aggregates_enabled: bool,

    /// Whether the `EXPLAIN` plan media type is enabled.
    ///
    /// When true, clients can request `Accept: application/vnd.pgrst.plan+json`
    /// to get the query execution plan instead of results.
    ///
    /// PostgREST equivalent: `db-plan-enabled`.
    #[serde(default)]
    pub plan_enabled: bool,

    /// Whether `Prefer: tx=rollback` is allowed.
    ///
    /// When true, clients can force transaction rollback (useful for testing).
    /// When false, the `tx` preference is silently ignored.
    ///
    /// PostgREST equivalent: `db-tx-allow-override`.
    #[serde(default)]
    pub tx_allow_override: bool,

    /// Roll back ALL transactions (testing/dry-run mode).
    ///
    /// When true, no transaction is ever committed regardless of client preference.
    ///
    /// PostgREST equivalent: `db-tx-rollback-all`.
    #[serde(default)]
    pub tx_rollback_all: bool,

    // --- Query limits ---

    /// Maximum number of rows returned per request (0 = unlimited).
    ///
    /// Acts as a server-side cap on `limit`. When a client requests more rows
    /// than this (or no limit), the response is capped at `max_rows`.
    ///
    /// PostgREST equivalent: `max-rows`.
    #[serde(default)]
    pub max_rows: Option<u64>,

    /// Default statement timeout in milliseconds (0 = no timeout).
    ///
    /// Applied to every query unless overridden per-function.
    /// Defaults to 30000 (30 seconds) to prevent runaway queries.
    #[serde(default = "default_statement_timeout_ms")]
    pub statement_timeout_ms: Option<u64>,

    /// Reject every mutation (POST/PATCH/PUT/DELETE on tables, RPC calls
    /// returning into mutation paths) at the surface layer.
    ///
    /// The REST router still enforces per-route privileges via the database
    /// role; this flag is an upstream cut so MCP tool catalogues (and any
    /// other surfaces that read it) omit write tools entirely when running in
    /// a read-only deployment (e.g. `pgvis mcp --read-only` for an LLM that
    /// should only browse).
    #[serde(default)]
    pub read_only: bool,

    // --- Connection pool ---

    /// Maximum number of database connections in the pool.
    ///
    /// Each concurrent request holds one connection for its transaction duration.
    /// Defaults to 16.
    #[serde(default = "default_pool_size")]
    pub pool_size: u32,

    /// Pool checkout timeout in milliseconds.
    ///
    /// If all connections are busy and a new request arrives, it will wait
    /// up to this duration before returning a 503 Service Unavailable error.
    /// Defaults to 5000 (5 seconds). Set to 0 for no timeout (not recommended).
    #[serde(default = "default_pool_timeout_ms")]
    pub pool_timeout_ms: u64,

    // --- Hooks ---

    /// Pre-request function to call before every query.
    ///
    /// Called after JWT verification and role switching, before the main query.
    /// Can raise exceptions to abort the request (e.g. rate limiting, audit).
    ///
    /// PostgREST equivalent: `db-pre-request`.
    pub pre_request: Option<String>,

    // --- OpenAPI ---

    /// OpenAPI document title.
    pub openapi_title: Option<String>,

    /// OpenAPI server URL override (for proxied deployments).
    ///
    /// PostgREST equivalent: `openapi-server-proxy-uri`.
    pub openapi_server_url: Option<String>,

    /// OpenAPI generation mode.
    ///
    /// PostgREST equivalent: `openapi-mode`.
    #[serde(default)]
    pub openapi_mode: OpenApiMode,

    // --- Routing ---

    /// URL routing and namespace configuration.
    ///
    /// Controls route URL structure and MCP tool naming.
    #[serde(default)]
    pub routing: RoutingConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schemas: default_schemas(),
            extra_search_path: Vec::new(),
            jwt_secret: None,
            jwt_algo: default_jwt_algo(),
            anon_role: None,
            role_claim_key: default_role_claim_key(),
            aggregates_enabled: false,
            plan_enabled: false,
            tx_allow_override: false,
            tx_rollback_all: false,
            max_rows: None,
            statement_timeout_ms: default_statement_timeout_ms(),
            read_only: false,
            pool_size: default_pool_size(),
            pool_timeout_ms: default_pool_timeout_ms(),
            pre_request: None,
            openapi_title: None,
            openapi_server_url: None,
            openapi_mode: OpenApiMode::default(),
            routing: RoutingConfig::default(),
        }
    }
}

/// JWT signing algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JwtAlgorithm {
    /// HMAC-SHA256 (symmetric secret).
    HS256,
    /// HMAC-SHA384 (symmetric secret).
    HS384,
    /// HMAC-SHA512 (symmetric secret).
    HS512,
    /// RSA-SHA256 (asymmetric — public key verification).
    RS256,
    /// Ed25519 (asymmetric — public key verification).
    EdDSA,
}

/// OpenAPI generation mode.
///
/// Controls how the OpenAPI spec is generated and what it exposes.
///
/// # PostgREST equivalent
///
/// `openapi-mode` config key: `follow-privileges`, `ignore-privileges`, `disabled`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum OpenApiMode {
    /// Show all tables/functions regardless of role privileges.
    ///
    /// The spec shows everything the anonymous role can see.
    /// Fastest — no per-role filtering needed.
    #[default]
    IgnorePrivileges,

    /// Filter the spec based on the requesting role's `GRANT`ed privileges.
    ///
    /// Requires an additional query per unique role to determine visible tables.
    /// Cached per role, invalidated on schema reload.
    FollowPrivileges,

    /// Disable the OpenAPI endpoint entirely (`GET /` returns 404).
    Disabled,
}

// --- Default value helpers ---

fn default_schemas() -> Vec<String> {
    vec!["public".to_string()]
}

fn default_jwt_algo() -> JwtAlgorithm {
    JwtAlgorithm::HS256
}

fn default_role_claim_key() -> String {
    "role".to_string()
}

fn default_routing_prefix() -> String {
    "api".to_string()
}

fn default_schema_in_path() -> bool {
    true
}

fn default_default_schema() -> String {
    "public".to_string()
}

fn default_mcp_separator() -> char {
    '/'
}

fn default_statement_timeout_ms() -> Option<u64> {
    Some(30_000) // 30 seconds
}

fn default_pool_size() -> u32 {
    16
}

fn default_pool_timeout_ms() -> u64 {
    5_000 // 5 seconds
}
