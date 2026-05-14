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
    pub statement_timeout_ms: Option<u64>,

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
            statement_timeout_ms: None,
            pre_request: None,
            openapi_title: None,
            openapi_server_url: None,
            openapi_mode: OpenApiMode::default(),
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
