//! Schema-driven routing — builds axum routes from the schema cache.
//!
//! The [`build_app`] function is the primary entry point. It takes a [`SchemaCache`],
//! [`Config`], and [`Dialect`] and produces an `axum::Router` with all routes
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
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use pgvis_core::plan::{plan_request, ApiRequest, RequestBody, RequestMethod};
use pgvis_core::preferences::Preferences;
use pgvis_core::query_params::{self, OrderItem};
use pgvis_core::select_ast::SelectItem;
use pgvis_core::{Config, Dialect, SchemaCache};

use crate::openapi;

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
}

// ---------------------------------------------------------------------------
// build_app — the main entry point
// ---------------------------------------------------------------------------

/// Build an axum Router from the [`SchemaCache`] and configuration.
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
) -> Router {
    let state = AppState {
        cache,
        config: config.clone(),
        dialect,
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
    Query(query): Query<HashMap<String, String>>,
    body: Option<Json<serde_json::Value>>,
) -> impl IntoResponse {
    let schema = params.get("schema").cloned().unwrap_or_default();
    let target = params.get("target").cloned().unwrap_or_default();
    let request_method = http_method_to_request_method(&method);

    dispatch_request(&state, schema, target, request_method, false, &headers, &query, body.map(|b| b.0))
}

/// Handle RPC requests when the schema is in the URL path.
async fn handle_rpc_with_schema(
    State(state): State<AppState>,
    method: axum::http::Method,
    Path(params): Path<HashMap<String, String>>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    body: Option<Json<serde_json::Value>>,
) -> impl IntoResponse {
    let schema = params.get("schema").cloned().unwrap_or_default();
    let function = params.get("function").cloned().unwrap_or_default();

    // RPC requests are always planned as POST (function call)
    let _ = method; // RPC accepts both GET and POST
    dispatch_request(&state, schema, function, RequestMethod::Post, true, &headers, &query, body.map(|b| b.0))
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
    Query(query): Query<HashMap<String, String>>,
    body: Option<Json<serde_json::Value>>,
) -> impl IntoResponse {
    let target = params.get("target").cloned().unwrap_or_default();
    let schema = resolve_schema_from_headers(&headers, &state.config);
    let request_method = http_method_to_request_method(&method);

    dispatch_request(&state, schema, target, request_method, false, &headers, &query, body.map(|b| b.0))
}

/// Handle RPC requests when the schema comes from headers/config.
async fn handle_rpc_no_schema(
    State(state): State<AppState>,
    method: axum::http::Method,
    Path(params): Path<HashMap<String, String>>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    body: Option<Json<serde_json::Value>>,
) -> impl IntoResponse {
    let function = params.get("function").cloned().unwrap_or_default();
    let schema = resolve_schema_from_headers(&headers, &state.config);

    let _ = method;
    dispatch_request(&state, schema, function, RequestMethod::Post, true, &headers, &query, body.map(|b| b.0))
}

// ---------------------------------------------------------------------------
// Root handler
// ---------------------------------------------------------------------------

/// Root endpoint handler — returns available schemas or the OpenAPI spec.
async fn handle_root(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    // Check if the client accepts OpenAPI JSON
    let accept = headers
        .get("accept")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if accept.contains("application/openapi+json") || accept.contains("application/vnd.pgrst.object") {
        // Generate OpenAPI spec
        let cache = state.cache.load();
        let spec = openapi::generate_spec(&cache, &state.config);
        match serde_json::to_value(&spec) {
            Ok(val) => (StatusCode::OK, Json(val)),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "code": "PGV500",
                    "message": format!("Failed to serialize OpenAPI spec: {e}"),
                })),
            ),
        }
    } else {
        let response = serde_json::json!({
            "schemas": state.config.schemas,
            "hint": "Append a table/view name to query it. Use Accept: application/openapi+json for the OpenAPI spec.",
        });
        (StatusCode::OK, Json(response))
    }
}

// ---------------------------------------------------------------------------
// Core dispatch logic
// ---------------------------------------------------------------------------

/// Core request dispatch — builds an [`ApiRequest`] and runs it through the planner.
fn dispatch_request(
    state: &AppState,
    schema: String,
    target: String,
    method: RequestMethod,
    is_rpc: bool,
    headers: &HeaderMap,
    params: &HashMap<String, String>,
    body: Option<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let cache = state.cache.load();

    // Build the adapter-agnostic ApiRequest
    let api_request = build_api_request(schema, target, method, is_rpc, headers, params, body);

    // Plan the request against the schema cache
    match plan_request(&api_request, &cache, &state.dialect, &state.config) {
        Ok(plan) => {
            // TODO: SQL builder + execution — for now return the plan summary as JSON
            let plan_type = match &plan {
                pgvis_core::plan::ActionPlan::Read(_) => "Read",
                pgvis_core::plan::ActionPlan::Mutate(_) => "Mutate",
                pgvis_core::plan::ActionPlan::Call(_) => "Call",
                pgvis_core::plan::ActionPlan::Inspect(_) => "Inspect",
            };
            let response = serde_json::json!({
                "status": "planned",
                "plan_type": plan_type,
            });
            (StatusCode::OK, Json(response))
        }
        Err(err) => {
            let status = StatusCode::from_u16(err.http_status())
                .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            let body = serde_json::json!({
                "code": err.code().as_str(),
                "message": err.to_string(),
            });
            (status, Json(body))
        }
    }
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
    headers: &HeaderMap,
    params: &HashMap<String, String>,
    body: Option<serde_json::Value>,
) -> ApiRequest {
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

    // Parse preferences from Prefer header
    let preferences = headers
        .get("prefer")
        .and_then(|v| v.to_str().ok())
        .map(|s| Preferences::parse(s).0)
        .unwrap_or_default();

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
        preferences,
        body: request_body,
        on_conflict,
        columns,
        logic_filters: Vec::new(), // TODO: parse and/or logic from query params
    }
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Parse filter expressions from query parameters.
///
/// Any parameter whose key is not a reserved keyword (`select`, `order`, `limit`,
/// `offset`, `on_conflict`, `columns`) is treated as a column filter.
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
