//! Schema-driven routing — builds axum routes from the schema cache.
//!
//! The [`build_app`] function is the primary entry point. It takes a [`SchemaCache`],
//! [`Config`], [`Dialect`], and [`Backend`] and produces an `axum::Router` with all routes
//! registered for the exposed schemas.
//!
//! ## Routing Modes
//!
//! Three routing modes controlled by [`RoutingConfig`](pgvis_core::config::RoutingConfig):
//! 1. **Full path** (`schema_in_path=true`): `/{prefix}/{schema}/{table}` and `/{prefix}/{schema}/rpc/{fn}`
//! 2. **Prefix only** (`schema_in_path=false`, `prefix="api"`): `/{prefix}/{table}` (schema from `Accept-Profile` header or default)
//! 3. **PostgREST compat** (`schema_in_path=false`, `prefix=""`): `/{table}` (PostgREST drop-in)

use std::collections::HashMap;
use std::sync::Arc;

use arc_swap::ArcSwap;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use pgvis_core::backend::{Backend, ExecContext, TxEnd};
use pgvis_core::plan::{plan_request, ActionPlan, ApiRequest, RequestBody, RequestMethod};
use pgvis_core::preferences::{PreferTx, Preferences};
use pgvis_core::query;
use pgvis_core::query_params::{self, OrderItem};
use pgvis_core::select_ast::SelectItem;
use pgvis_core::config::OpenApiMode;
use pgvis_core::{Config, Dialect, SchemaCache};

use crate::openapi;
use crate::response;

// ---------------------------------------------------------------------------
// AppState
// ---------------------------------------------------------------------------

/// Shared application state, hot-swappable via `ArcSwap`.
///
/// The [`SchemaCache`] is stored behind `ArcSwap` so it can be atomically
/// updated without rebuilding routes. Handlers load the latest snapshot on
/// each request.
#[derive(Clone)]
pub struct AppState {
    /// The schema cache — hot-swappable for live schema reloads.
    pub cache: Arc<ArcSwap<SchemaCache>>,
    /// The shared configuration (routing, auth, feature gates).
    pub config: Arc<Config>,
    /// The SQL dialect (Postgres or SQLite capability flags).
    pub dialect: Arc<Dialect>,
    /// The database backend for query execution.
    pub backend: Arc<dyn Backend>,
}

// ---------------------------------------------------------------------------
// build_app — the main entry point
// ---------------------------------------------------------------------------

/// Build an axum Router from the [`SchemaCache`], configuration, and backend.
///
/// Routes are generated based on `config.routing`:
/// - `schema_in_path = true`: `/{prefix}/{schema}/{table}` and `/{prefix}/{schema}/rpc/{fn}`
/// - `schema_in_path = false`: `/{prefix}/{table}` and `/{prefix}/rpc/{fn}`
///
/// # Hot Reload
///
/// The returned router uses `ArcSwap<SchemaCache>` so handlers always reference the
/// latest schema cache snapshot. Call `ArcSwap::store` to atomically update the cache
/// without rebuilding routes.
///
/// # Approach
///
/// Rather than generating one route per table (which would require rebuilding the router
/// on schema changes), we use wildcard path parameters and resolve the target at
/// request time against the current schema cache snapshot.
pub fn build_app(
    cache: Arc<ArcSwap<SchemaCache>>,
    config: Arc<Config>,
    dialect: Arc<Dialect>,
    backend: Arc<dyn Backend>,
) -> Router {
    let state = AppState {
        cache,
        config: config.clone(),
        dialect,
        backend,
    };

    let routing = &config.routing;
    let prefix = routing.normalized_prefix();

    let mut router = Router::new();

    if routing.schema_in_path {
        // Mode 1: /{prefix}/{schema}/{table} and /{prefix}/{schema}/rpc/{fn}
        if prefix.is_empty() {
            router = router
                .route("/{schema}/rpc/{function}", get(handle_rpc_with_schema).post(handle_rpc_with_schema))
                .route("/{schema}/{target}", get(handle_table_with_schema)
                    .head(handle_table_with_schema)
                    .post(handle_table_with_schema)
                    .put(handle_table_with_schema)
                    .patch(handle_table_with_schema)
                    .delete(handle_table_with_schema))
                .route("/", get(handle_root));
        } else {
            let rpc_path = format!("/{prefix}/{{schema}}/rpc/{{function}}");
            let table_path = format!("/{prefix}/{{schema}}/{{target}}");
            let root_path = format!("/{prefix}/");

            router = router
                .route(&rpc_path, get(handle_rpc_with_schema).post(handle_rpc_with_schema))
                .route(&table_path, get(handle_table_with_schema)
                    .head(handle_table_with_schema)
                    .post(handle_table_with_schema)
                    .put(handle_table_with_schema)
                    .patch(handle_table_with_schema)
                    .delete(handle_table_with_schema))
                .route(&root_path, get(handle_root));
        }
    } else {
        // Mode 2/3: /{prefix}/{table} or /{table} (schema from header/default)
        if prefix.is_empty() {
            router = router
                .route("/rpc/{function}", get(handle_rpc_no_schema).post(handle_rpc_no_schema))
                .route("/{target}", get(handle_table_no_schema)
                    .head(handle_table_no_schema)
                    .post(handle_table_no_schema)
                    .put(handle_table_no_schema)
                    .patch(handle_table_no_schema)
                    .delete(handle_table_no_schema))
                .route("/", get(handle_root));
        } else {
            let rpc_path = format!("/{prefix}/rpc/{{function}}");
            let table_path = format!("/{prefix}/{{target}}");
            let root_path = format!("/{prefix}/");

            router = router
                .route(&rpc_path, get(handle_rpc_no_schema).post(handle_rpc_no_schema))
                .route(&table_path, get(handle_table_no_schema)
                    .head(handle_table_no_schema)
                    .post(handle_table_no_schema)
                    .put(handle_table_no_schema)
                    .patch(handle_table_no_schema)
                    .delete(handle_table_no_schema))
                .route(&root_path, get(handle_root));
        }
    }

    router.with_state(state)
}

// ---------------------------------------------------------------------------
// Handlers — schema_in_path = true
// ---------------------------------------------------------------------------

/// Handle table requests when the schema is in the URL path.
async fn handle_table_with_schema(
    State(state): State<AppState>,
    method: axum::http::Method,
    Path(params): Path<HashMap<String, String>>,
    headers: HeaderMap,
    Query(query_params): Query<HashMap<String, String>>,
    body: Option<Json<serde_json::Value>>,
) -> Response {
    let schema = params.get("schema").cloned().unwrap_or_default();
    let target = params.get("target").cloned().unwrap_or_default();
    let request_method = http_method_to_request_method(&method);

    dispatch_request(&state, schema, target, request_method, false, &headers, &query_params, body.map(|b| b.0)).await
}

/// Handle RPC requests when the schema is in the URL path.
async fn handle_rpc_with_schema(
    State(state): State<AppState>,
    method: axum::http::Method,
    Path(params): Path<HashMap<String, String>>,
    headers: HeaderMap,
    Query(query_params): Query<HashMap<String, String>>,
    body: Option<Json<serde_json::Value>>,
) -> Response {
    let schema = params.get("schema").cloned().unwrap_or_default();
    let function = params.get("function").cloned().unwrap_or_default();

    // RPC accepts both GET and POST — always plan as Post for function call
    let _ = method;
    dispatch_request(&state, schema, function, RequestMethod::Post, true, &headers, &query_params, body.map(|b| b.0)).await
}

// ---------------------------------------------------------------------------
// Handlers — schema_in_path = false
// ---------------------------------------------------------------------------

/// Handle table requests when the schema comes from headers/config.
async fn handle_table_no_schema(
    State(state): State<AppState>,
    method: axum::http::Method,
    Path(params): Path<HashMap<String, String>>,
    headers: HeaderMap,
    Query(query_params): Query<HashMap<String, String>>,
    body: Option<Json<serde_json::Value>>,
) -> Response {
    let target = params.get("target").cloned().unwrap_or_default();
    let schema = resolve_schema_from_headers(&headers, &state.config);
    let request_method = http_method_to_request_method(&method);

    dispatch_request(&state, schema, target, request_method, false, &headers, &query_params, body.map(|b| b.0)).await
}

/// Handle RPC requests when the schema comes from headers/config.
async fn handle_rpc_no_schema(
    State(state): State<AppState>,
    method: axum::http::Method,
    Path(params): Path<HashMap<String, String>>,
    headers: HeaderMap,
    Query(query_params): Query<HashMap<String, String>>,
    body: Option<Json<serde_json::Value>>,
) -> Response {
    let function = params.get("function").cloned().unwrap_or_default();
    let schema = resolve_schema_from_headers(&headers, &state.config);

    let _ = method;
    dispatch_request(&state, schema, function, RequestMethod::Post, true, &headers, &query_params, body.map(|b| b.0)).await
}

// ---------------------------------------------------------------------------
// Root handler
// ---------------------------------------------------------------------------

/// Root endpoint handler — returns available schemas or the OpenAPI spec.
async fn handle_root(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    // Check if the client accepts OpenAPI JSON
    let accept = headers
        .get("accept")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if accept.contains("application/openapi+json") || accept.contains("application/vnd.pgrst.object") {
        // Check if OpenAPI is disabled
        if state.config.openapi_mode == OpenApiMode::Disabled {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "code": "PGRST404",
                    "message": "OpenAPI spec is disabled",
                    "details": null,
                    "hint": "Set openapi_mode to IgnorePrivileges or FollowPrivileges to enable.",
                })),
            ).into_response();
        }

        // Generate OpenAPI spec
        let cache = state.cache.load();
        let spec = openapi::generate_spec(&cache, &state.config);
        match serde_json::to_value(&spec) {
            Ok(val) => (StatusCode::OK, Json(val)).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "code": "PGV500",
                    "message": format!("Failed to serialize OpenAPI spec: {e}"),
                })),
            ).into_response(),
        }
    } else {
        let resp = serde_json::json!({
            "schemas": state.config.schemas,
            "hint": "Append a table/view name to query it. Use Accept: application/openapi+json for the OpenAPI spec.",
        });
        (StatusCode::OK, Json(resp)).into_response()
    }
}

// ---------------------------------------------------------------------------
// Core dispatch logic — the full pipeline
// ---------------------------------------------------------------------------

/// Core request dispatch — plan → render SQL → execute → format response.
///
/// This is the heart of the pgvis pipeline:
/// 1. Parse HTTP concerns into an [`ApiRequest`]
/// 2. Plan the request against the schema cache → [`ActionPlan`]
/// 3. Render the plan to parameterised SQL via [`query::render`]
/// 4. Execute via [`Backend::execute`] with transaction/role/claims
/// 5. Format the [`QueryResult`] into an HTTP response
async fn dispatch_request(
    state: &AppState,
    schema: String,
    target: String,
    method: RequestMethod,
    is_rpc: bool,
    headers: &HeaderMap,
    params: &HashMap<String, String>,
    body: Option<serde_json::Value>,
) -> Response {
    let cache = state.cache.load();

    // Parse preferences early — needed for ExecContext and response formatting
    let preferences = headers
        .get("prefer")
        .and_then(|v| v.to_str().ok())
        .map(|s| Preferences::parse(s).0)
        .unwrap_or_default();

    // 1. Build the adapter-agnostic ApiRequest
    let api_request = build_api_request(
        schema, target, method.clone(), is_rpc, headers, params, body, &preferences,
    );

    // 2. Plan the request against the schema cache
    let plan = match plan_request(&api_request, &cache, &state.dialect, &state.config) {
        Ok(plan) => plan,
        Err(err) => return response::format_error(&err),
    };

    // For Inspect plans, return the inspection result directly
    if let ActionPlan::Inspect(_) = &plan {
        let resp = serde_json::json!({"status": "inspect", "message": "not yet implemented"});
        return (StatusCode::OK, Json(resp)).into_response();
    }

    // 3. Render the plan to SQL + parameters
    let (sql, params_vec) = match query::render(&plan, &state.dialect) {
        Ok(rendered) => rendered,
        Err(err) => return response::format_error(&err),
    };

    tracing::debug!(sql = %sql, params = ?params_vec, "executing query");

    // 4. Build ExecContext from config + preferences
    let exec_ctx = build_exec_context(&state.config, &preferences);

    // 5. Execute via backend
    let result = match state.backend.execute(&exec_ctx, &sql, &params_vec).await {
        Ok(result) => result,
        Err(err) => return response::format_error(&err),
    };

    // 6. Format the QueryResult into an HTTP response
    let is_singular = headers
        .get("accept")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.contains("application/vnd.pgrst.object"))
        .unwrap_or(false);

    let request_offset = params.get("offset").and_then(|s| s.parse::<u64>().ok());

    response::format_response(&result, &method, &preferences, is_singular, request_offset)
}

// ---------------------------------------------------------------------------
// build_api_request — parse HTTP concerns into the adapter-agnostic request
// ---------------------------------------------------------------------------

/// Build an [`ApiRequest`] from raw HTTP query parameters, headers, and body.
///
/// This is where the REST adapter converts HTTP-level concerns into the
/// adapter-agnostic `ApiRequest` that the plan layer consumes.
fn build_api_request(
    schema: String,
    target: String,
    method: RequestMethod,
    is_rpc: bool,
    _headers: &HeaderMap,
    params: &HashMap<String, String>,
    body: Option<serde_json::Value>,
    preferences: &Preferences,
) -> ApiRequest {
    let _ = preferences; // Will be used for count strategy, etc.

    // Parse select parameter
    let select = params
        .get("select")
        .and_then(|s| query_params::parse_select(s).ok())
        .unwrap_or_default();

    // If select is empty, default to star
    let select = if select.is_empty() {
        vec![SelectItem::Star]
    } else {
        select
    };

    // Parse filters from query params (columns not named select/order/limit/offset)
    let filters = parse_filters_from_params(params);

    // Parse order — extract only direct OrderTerms (skip relation terms for now)
    let order = params
        .get("order")
        .and_then(|s| query_params::parse_order(s).ok())
        .map(|items| {
            items
                .into_iter()
                .filter_map(|item| match item {
                    OrderItem::Term(t) => Some(t),
                    OrderItem::Relation(_) => None,
                })
                .collect()
        })
        .unwrap_or_default();

    // Parse range (limit/offset)
    let range = parse_range_from_params(params);

    // Parse body into RequestBody
    let request_body = body.map(|v| {
        if v.is_array() {
            RequestBody::Bulk(v.as_array().cloned().unwrap_or_default())
        } else {
            RequestBody::Single(v)
        }
    });

    // On-conflict
    let on_conflict = params.get("on_conflict").cloned();

    // Columns
    let columns = params.get("columns").map(|s| {
        s.split(',').map(|c| c.trim().to_string()).collect()
    });

    ApiRequest {
        schema,
        target,
        method,
        is_rpc,
        select,
        filters,
        order,
        range,
        preferences: preferences.clone(),
        body: request_body,
        on_conflict,
        columns,
        logic_filters: Vec::new(), // TODO: parse and/or logic from query params
    }
}

/// Build an [`ExecContext`] from configuration and request preferences.
///
/// In a full implementation, this would also extract the JWT role and claims.
/// For now, we use the anonymous role from config.
fn build_exec_context(config: &Config, preferences: &Preferences) -> ExecContext {
    let tx_end = preferences.tx.and_then(|tx| match tx {
        PreferTx::Commit => Some(TxEnd::Commit),
        PreferTx::Rollback => Some(TxEnd::Rollback),
    });

    ExecContext {
        role: config.anon_role.clone(),
        claims: None, // TODO: extract from JWT when auth is implemented
        pre_request: config.pre_request.clone(),
        // Always enforce a statement timeout to prevent runaway queries
        statement_timeout: config.statement_timeout_ms,
        tx_end,
    }
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Parse filter expressions from query parameters.
///
/// Any parameter whose key is not a reserved keyword (`select`, `order`, `limit`,
/// `offset`, `on_conflict`, `columns`) is treated as a column filter.
///
/// Filters are sorted by column name for deterministic SQL output,
/// which improves Postgres prepared-statement cache hit rates and
/// makes debugging/logging reproducible.
fn parse_filters_from_params(
    params: &HashMap<String, String>,
) -> Vec<pgvis_core::query_params::Filter> {
    const RESERVED: &[&str] = &["select", "order", "limit", "offset", "on_conflict", "columns"];
    let mut filters = Vec::new();

    for (key, value) in params {
        if RESERVED.contains(&key.as_str()) {
            continue;
        }
        // Try to parse as a filter: column=operator.value
        if let Ok(filter) = query_params::parse_filter(key, value) {
            filters.push(filter);
        }
    }

    // Sort by column name for deterministic SQL output
    filters.sort_by(|a, b| a.field.cmp(&b.field));

    filters
}

/// Parse limit/offset from query parameters into a `RangeSpec`.
fn parse_range_from_params(
    params: &HashMap<String, String>,
) -> Option<pgvis_core::query_params::RangeSpec> {
    let limit = params.get("limit").and_then(|s| s.parse().ok());
    let offset = params.get("offset").and_then(|s| s.parse().ok());

    if limit.is_some() || offset.is_some() {
        Some(pgvis_core::query_params::RangeSpec { limit, offset })
    } else {
        None
    }
}

/// Resolve the schema name from headers (when `schema_in_path = false`).
///
/// Checks `Accept-Profile` header first, falls back to `config.routing.default_schema`.
fn resolve_schema_from_headers(headers: &HeaderMap, config: &Config) -> String {
    headers
        .get("accept-profile")
        .or_else(|| headers.get("content-profile"))
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| config.routing.default_schema.clone())
}

/// Convert an axum HTTP method to our [`RequestMethod`].
fn http_method_to_request_method(method: &axum::http::Method) -> RequestMethod {
    match *method {
        axum::http::Method::GET => RequestMethod::Get,
        axum::http::Method::HEAD => RequestMethod::Head,
        axum::http::Method::POST => RequestMethod::Post,
        axum::http::Method::PATCH => RequestMethod::Patch,
        axum::http::Method::PUT => RequestMethod::Put,
        axum::http::Method::DELETE => RequestMethod::Delete,
        _ => RequestMethod::Get,
    }
}
