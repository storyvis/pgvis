//! # Query execution — parameter binding, transaction management, and result extraction.
//!
//! Bridges the gap between the SQL builder's output (`String` + `Vec<serde_json::Value>`)
//! and tokio-postgres's execution interface.
//!
//! ## Design: Text-Protocol Approach
//!
//! Like PostgREST, all parameter values are sent as text strings. Postgres will coerce
//! them to the correct type based on the query context. This avoids needing to match
//! Rust types to Postgres OIDs at the driver level.

use pgvis_core::backend::{ExecContext, QueryResult, TxEnd};
use pgvis_core::error::Error;
use serde_json::Value;
use tokio_postgres::types::{IsNull, ToSql, Type};
use tokio_postgres::Client;

// ---------------------------------------------------------------------------
// TextParam — sends all values as text for Postgres to coerce
// ---------------------------------------------------------------------------

/// A wrapper that sends `serde_json::Value` as text to Postgres.
///
/// Postgres will coerce the text representation to the correct column type
/// based on query context (same approach PostgREST uses).
///
/// Mapping:
/// - `Value::Null` → SQL NULL
/// - `Value::String(s)` → text `s`
/// - `Value::Number(n)` → text representation of the number
/// - `Value::Bool(b)` → `"true"` / `"false"`
/// - `Value::Array(...)` → JSON text (Postgres parses as array literal or json)
/// - `Value::Object(...)` → JSON text
#[derive(Debug)]
pub struct TextParam<'a>(pub &'a Value);

impl ToSql for TextParam<'_> {
    fn to_sql(
        &self,
        ty: &Type,
        out: &mut bytes::BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        match &self.0 {
            Value::Null => Ok(IsNull::Yes),
            Value::String(s) => encode_str_as_type(s, ty, out),
            Value::Number(n) => {
                // Try native numeric encoding first
                if *ty == Type::INT4 || *ty == Type::INT2 {
                    if let Some(i) = n.as_i64() {
                        return (i as i32).to_sql(ty, out);
                    }
                }
                if *ty == Type::INT8 {
                    if let Some(i) = n.as_i64() {
                        return i.to_sql(ty, out);
                    }
                }
                if *ty == Type::FLOAT8 {
                    if let Some(f) = n.as_f64() {
                        return f.to_sql(ty, out);
                    }
                }
                if *ty == Type::FLOAT4 {
                    if let Some(f) = n.as_f64() {
                        return (f as f32).to_sql(ty, out);
                    }
                }
                // Fallback: send as text
                let s = n.to_string();
                s.as_str().to_sql(&Type::TEXT, out)
            }
            Value::Bool(b) => {
                if *ty == Type::BOOL {
                    b.to_sql(ty, out)
                } else {
                    let s = if *b { "true" } else { "false" };
                    s.to_sql(&Type::TEXT, out)
                }
            }
            // Arrays and objects → serialize as JSON text
            other => {
                let s = serde_json::to_string(other)
                    .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Sync + Send>)?;
                s.as_str().to_sql(&Type::TEXT, out)
            }
        }
    }

    fn accepts(_ty: &Type) -> bool {
        true
    }

    tokio_postgres::types::to_sql_checked!();
}

/// Encode a string value as the target Postgres type.
/// Parses the string to the appropriate Rust type based on what Postgres expects.
fn encode_str_as_type(
    s: &str,
    ty: &Type,
    out: &mut bytes::BytesMut,
) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
    match *ty {
        Type::INT4 => {
            let v: i32 = s.parse()?;
            v.to_sql(ty, out)
        }
        Type::INT8 => {
            let v: i64 = s.parse()?;
            v.to_sql(ty, out)
        }
        Type::INT2 => {
            let v: i16 = s.parse()?;
            v.to_sql(ty, out)
        }
        Type::FLOAT4 => {
            let v: f32 = s.parse()?;
            v.to_sql(ty, out)
        }
        Type::FLOAT8 => {
            let v: f64 = s.parse()?;
            v.to_sql(ty, out)
        }
        Type::BOOL => {
            let v = matches!(s, "true" | "t" | "yes" | "1");
            v.to_sql(ty, out)
        }
        // TEXT, VARCHAR, NAME, UNKNOWN, and everything else: send as text
        _ => s.to_sql(&Type::TEXT, out),
    }
}

// ---------------------------------------------------------------------------
// Execute a CTE-wrapped query
// ---------------------------------------------------------------------------

/// Execute a CTE-wrapped SQL statement within a transaction.
///
/// This is the full execution pipeline:
/// 1. BEGIN transaction
/// 2. SET LOCAL role (if provided)
/// 3. SET LOCAL claims GUCs (if provided)
/// 4. SET LOCAL statement_timeout (if provided)
/// 5. Call pre-request function (if configured)
/// 6. Execute the main SQL with parameters
/// 7. Extract result from the CTE row
/// 8. COMMIT or ROLLBACK based on preference
pub async fn execute_query(
    client: &Client,
    ctx: &ExecContext,
    sql: &str,
    params: &[Value],
) -> Result<QueryResult, Error> {
    // 1. Begin transaction
    client
        .batch_execute("BEGIN ISOLATION LEVEL READ COMMITTED")
        .await
        .map_err(|e| execution_error("BEGIN failed", &e))?;

    // Run the inner execution; if it fails, rollback
    let result = execute_inner(client, ctx, sql, params).await;

    // Determine transaction end
    let should_rollback = match result {
        Err(_) => true,
        Ok(_) => matches!(ctx.tx_end, Some(TxEnd::Rollback)),
    };

    let end_sql = if should_rollback { "ROLLBACK" } else { "COMMIT" };
    client
        .batch_execute(end_sql)
        .await
        .map_err(|e| execution_error(&format!("{end_sql} failed"), &e))?;

    result
}

/// Inner execution logic (within the transaction).
async fn execute_inner(
    client: &Client,
    ctx: &ExecContext,
    sql: &str,
    params: &[Value],
) -> Result<QueryResult, Error> {
    // 2. SET LOCAL role
    if let Some(role) = &ctx.role {
        let set_role = format!("SET LOCAL role = '{}'", escape_literal(role));
        client
            .batch_execute(&set_role)
            .await
            .map_err(|e| execution_error("SET LOCAL role failed", &e))?;
    }

    // 3. SET LOCAL claims
    if let Some(claims) = &ctx.claims {
        let claims_str = claims.to_string();
        let set_claims = format!(
            "SET LOCAL request.jwt.claims = '{}'",
            escape_literal(&claims_str)
        );
        client
            .batch_execute(&set_claims)
            .await
            .map_err(|e| execution_error("SET LOCAL claims failed", &e))?;

        // Also set individual claim GUCs (request.jwt.claim.sub, etc.)
        if let Value::Object(map) = claims {
            for (key, val) in map {
                let val_str = match val {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                let set_claim = format!(
                    "SET LOCAL \"request.jwt.claim.{key}\" = '{}'",
                    escape_literal(&val_str)
                );
                // Individual claim failures are non-fatal
                let _ = client.batch_execute(&set_claim).await;
            }
        }
    }

    // 4. SET LOCAL statement_timeout
    if let Some(timeout_ms) = ctx.statement_timeout {
        let set_timeout = format!("SET LOCAL statement_timeout = '{timeout_ms}ms'");
        client
            .batch_execute(&set_timeout)
            .await
            .map_err(|e| execution_error("SET LOCAL statement_timeout failed", &e))?;
    }

    // 5. Pre-request function
    if let Some(pre_req) = &ctx.pre_request {
        let call_pre = format!("SELECT {pre_req}()");
        client
            .batch_execute(&call_pre)
            .await
            .map_err(|e| execution_error("pre-request function failed", &e))?;
    }

    // 6. Execute the main query with parameters.
    // TextParam implements ToSql and encodes values based on the type Postgres infers
    // for each parameter position (e.g., INT4 for integer columns, TEXT for text columns).
    let text_params: Vec<TextParam> = params.iter().map(TextParam).collect();
    let param_refs: Vec<&(dyn ToSql + Sync)> = text_params
        .iter()
        .map(|p| p as &(dyn ToSql + Sync))
        .collect();

    let rows = client
        .query(sql, &param_refs)
        .await
        .map_err(|e| execution_error("query execution failed", &e))?;

    // 7. Extract result from the CTE row
    extract_cte_result(&rows)
}

// ---------------------------------------------------------------------------
// CTE result extraction
// ---------------------------------------------------------------------------

/// Extract a [`QueryResult`] from the CTE-wrapped result rows.
///
/// The CTE produces a single row with columns:
/// - `body` — JSON array (coalesced to '[]')
/// - `page_total` — count of rows on this page
/// - `total_count` — total count (only when Prefer: count=exact)
/// - `response_status` — GUC override (Postgres only)
/// - `response_headers` — GUC override (Postgres only)
fn extract_cte_result(rows: &[tokio_postgres::Row]) -> Result<QueryResult, Error> {
    if rows.is_empty() {
        // No rows from CTE means something went wrong, but we handle gracefully
        return Ok(QueryResult {
            body: Value::Array(vec![]),
            total_count: None,
            page_total: Some(0),
            response_status: None,
            response_headers: None,
            was_insert: None,
        });
    }

    let row = &rows[0];

    // body — json_agg result. With `with-serde_json-1` feature, tokio-postgres
    // can directly deserialize json/jsonb columns to serde_json::Value.
    let body: Value = try_get_column(row, "body")
        .unwrap_or(Value::Array(vec![]));

    // page_total
    let page_total: Option<i64> = try_get_column(row, "page_total");

    // total_count (only present when count preference was requested)
    let total_count: Option<i64> = try_get_column(row, "total_count");

    // response_status — from GUC current_setting('response.status', true)
    let response_status_str: Option<String> = try_get_column(row, "response_status");
    let response_status = response_status_str
        .as_deref()
        .and_then(|s| s.parse::<u16>().ok());

    // response_headers — from GUC current_setting('response.headers', true)
    let response_headers_str: Option<String> = try_get_column(row, "response_headers");
    let response_headers = response_headers_str
        .as_deref()
        .and_then(parse_guc_headers);

    Ok(QueryResult {
        body,
        total_count,
        page_total,
        response_status,
        response_headers,
        was_insert: None,
    })
}

/// Try to get a column value, returning None if the column doesn't exist or is NULL.
fn try_get_column<'a, T: tokio_postgres::types::FromSql<'a>>(
    row: &'a tokio_postgres::Row,
    name: &str,
) -> Option<T> {
    row.try_get(name).ok()
}

// ---------------------------------------------------------------------------
// GUC header parsing
// ---------------------------------------------------------------------------

/// Parse response headers from the GUC value.
///
/// PostgREST format: `[{"Header-Name": "value"}, ...]`
fn parse_guc_headers(raw: &str) -> Option<Vec<(String, String)>> {
    let parsed: Vec<serde_json::Map<String, Value>> = serde_json::from_str(raw).ok()?;
    let mut headers = Vec::new();
    for obj in parsed {
        for (key, val) in obj {
            let value = match val {
                Value::String(s) => s,
                other => other.to_string(),
            };
            headers.push((key, value));
        }
    }
    Some(headers)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Escape a string literal for use in SET LOCAL statements.
/// Doubles single quotes to prevent SQL injection.
fn escape_literal(s: &str) -> String {
    s.replace('\'', "''")
}

/// Create an execution error from a tokio-postgres error.
fn execution_error(context: &str, e: &tokio_postgres::Error) -> Error {
    Error::Execution {
        message: format!("{context}: {e}"),
        db_code: e.code().map(|c| c.code().to_string()),
        detail: e
            .as_db_error()
            .and_then(|db| db.detail().map(String::from)),
        hint: e.as_db_error().and_then(|db| db.hint().map(String::from)),
    }
}
