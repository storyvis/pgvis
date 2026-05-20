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
                // Try native numeric encoding first, with bounds checking
                if *ty == Type::INT2 {
                    if let Some(i) = n.as_i64() {
                        let v = i16::try_from(i).map_err(|_| {
                            format!("value {i} out of range for type smallint")
                        })?;
                        return v.to_sql(ty, out);
                    }
                }
                if *ty == Type::INT4 {
                    if let Some(i) = n.as_i64() {
                        let v = i32::try_from(i).map_err(|_| {
                            format!("value {i} out of range for type integer")
                        })?;
                        return v.to_sql(ty, out);
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
                if *ty == Type::NUMERIC {
                    let s = n.to_string();
                    encode_numeric_str(&s, out)?;
                    return Ok(IsNull::No);
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
            // Arrays and objects → serialize as JSON
            other => {
                let s = serde_json::to_string(other)
                    .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Sync + Send>)?;
                if *ty == Type::JSONB {
                    // JSONB binary format: version byte (1) + JSON text
                    use bytes::BufMut;
                    out.put_u8(1); // JSONB version
                    out.extend_from_slice(s.as_bytes());
                    Ok(IsNull::No)
                } else if *ty == Type::JSON {
                    // JSON binary format is just the text
                    s.as_str().to_sql(&Type::TEXT, out)
                } else {
                    // Unknown type — send as text (Postgres will try to coerce)
                    s.as_str().to_sql(&Type::TEXT, out)
                }
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
        Type::INT4 | Type::OID => {
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
        Type::NUMERIC => {
            // Postgres NUMERIC binary wire format: encode digits in base-10000.
            // ndigits(i16), weight(i16), sign(u16), dscale(u16), digits(i16[])
            encode_numeric_str(s, out)?;
            Ok(IsNull::No)
        }
        Type::BOOL => {
            let v = matches!(s, "true" | "t" | "yes" | "1");
            v.to_sql(ty, out)
        }
        // TEXT, VARCHAR, NAME, UNKNOWN, and everything else: send as text
        _ => s.to_sql(&Type::TEXT, out),
    }
}

/// Encode a decimal string into Postgres NUMERIC binary format.
///
/// Wire format: ndigits(i16) weight(i16) sign(u16) dscale(u16) digits(i16[])
/// Each digit is a base-10000 digit (0–9999).
/// Weight is the exponent of the first digit: value = sum(digit[i] * 10000^(weight - i))
fn encode_numeric_str(
    s: &str,
    out: &mut bytes::BytesMut,
) -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
    use bytes::BufMut;

    let s = s.trim();
    if s.eq_ignore_ascii_case("nan") {
        out.put_i16(0); // ndigits
        out.put_i16(0); // weight
        out.put_u16(0xC000); // sign = NaN
        out.put_u16(0); // dscale
        return Ok(());
    }

    let (sign, s) = if let Some(rest) = s.strip_prefix('-') {
        (0x4000u16, rest)
    } else if let Some(rest) = s.strip_prefix('+') {
        (0x0000u16, rest)
    } else {
        (0x0000u16, s)
    };

    // Split integer and fractional parts
    let (int_part, frac_part) = match s.split_once('.') {
        Some((i, f)) => (i, f),
        None => (s, ""),
    };

    let dscale = frac_part.len() as u16;

    // Remove leading zeros from integer part (but keep track of it being empty)
    let int_part = int_part.trim_start_matches('0');
    let int_len = int_part.len();

    // === Build base-10000 digit groups ===
    //
    // Strategy: Pad integer part on the LEFT to a multiple of 4,
    // pad fractional part on the RIGHT to a multiple of 4,
    // then chunk into groups of 4.

    // Integer groups
    let int_pad = if int_len == 0 { 0 } else { (4 - int_len % 4) % 4 };
    let int_groups: Vec<i16> = if int_len == 0 {
        vec![]
    } else {
        let mut int_str = "0".repeat(int_pad);
        int_str.push_str(int_part);
        int_str
            .as_bytes()
            .chunks(4)
            .map(|chunk| std::str::from_utf8(chunk).unwrap().parse::<i16>().unwrap())
            .collect()
    };

    // Fractional groups
    let frac_groups: Vec<i16> = if frac_part.is_empty() {
        vec![]
    } else {
        let frac_pad = (4 - frac_part.len() % 4) % 4;
        let mut frac_str = frac_part.to_string();
        for _ in 0..frac_pad {
            frac_str.push('0');
        }
        frac_str
            .as_bytes()
            .chunks(4)
            .map(|chunk| std::str::from_utf8(chunk).unwrap().parse::<i16>().unwrap())
            .collect()
    };

    // Combine all groups
    let mut digits: Vec<i16> = Vec::with_capacity(int_groups.len() + frac_groups.len());
    digits.extend_from_slice(&int_groups);
    digits.extend_from_slice(&frac_groups);

    // Handle pure zero
    if digits.is_empty() || digits.iter().all(|&d| d == 0) {
        out.put_i16(0); // ndigits
        out.put_i16(0); // weight
        out.put_u16(sign);
        out.put_u16(dscale);
        return Ok(());
    }

    // Calculate weight of the FIRST group in the combined array (before trimming).
    // For integers: weight = number_of_integer_groups - 1
    // For pure fractional: weight = -1 (first frac group is at 10000^(-1) position)
    // Leading-zero trimming then adjusts the weight downward.
    let weight: i16 = if int_len > 0 {
        (int_groups.len() as i16) - 1
    } else {
        // First fractional group is at position -1 (i.e., 10000^(-1))
        // Leading zeros will be trimmed and weight adjusted below.
        -1
    };

    // Trim leading zero groups and adjust weight
    let leading_zeros = digits.iter().take_while(|&&d| d == 0).count();
    // Trim trailing zero groups
    let trailing_zeros = digits.iter().rev().take_while(|&&d| d == 0).count();

    let start = leading_zeros;
    let end = digits.len() - trailing_zeros;
    let trimmed_digits = &digits[start..end];
    let adjusted_weight = weight - leading_zeros as i16;

    let ndigits = trimmed_digits.len() as i16;

    out.put_i16(ndigits);
    out.put_i16(adjusted_weight);
    out.put_u16(sign);
    out.put_u16(dscale);
    for d in trimmed_digits {
        out.put_i16(*d);
    }

    Ok(())
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
    let should_rollback = match &result {
        Err(_) => true,
        Ok(_) => matches!(ctx.tx_end, Some(TxEnd::Rollback)),
    };

    let end_sql = if should_rollback { "ROLLBACK" } else { "COMMIT" };
    if let Err(tx_err) = client.batch_execute(end_sql).await {
        tracing::error!(error = %tx_err, command = end_sql, "transaction end failed");
        // If the original result was Ok but COMMIT failed, return the commit error
        if result.is_ok() {
            return Err(execution_error(&format!("{end_sql} failed"), &tx_err));
        }
        // If original was already Err, preserve it (don't lose the real error)
    }

    result
}

/// Inner execution logic (within the transaction).
///
/// Batches all SET LOCAL statements into a single `batch_execute` call to minimize
/// round-trips, then executes the main query with a prepared statement.
async fn execute_inner(
    client: &Client,
    ctx: &ExecContext,
    sql: &str,
    params: &[Value],
) -> Result<QueryResult, Error> {
    // Build and execute all session setup in a single batch (saves 4-9 round-trips)
    let setup_sql = build_session_setup(ctx)?;
    if !setup_sql.is_empty() {
        client
            .batch_execute(&setup_sql)
            .await
            .map_err(|e| execution_error("session setup failed", &e))?;
    }

    // Execute the main query with parameters.
    // Using prepare() enables per-connection statement caching in tokio-postgres,
    // avoiding repeated parse cycles for identical SQL on the same connection.
    let text_params: Vec<TextParam> = params.iter().map(TextParam).collect();
    let param_refs: Vec<&(dyn ToSql + Sync)> = text_params
        .iter()
        .map(|p| p as &(dyn ToSql + Sync))
        .collect();

    let stmt = client
        .prepare(sql)
        .await
        .map_err(|e| execution_error("prepare failed", &e))?;
    let rows = client
        .query(&stmt, &param_refs)
        .await
        .map_err(|e| execution_error("query execution failed", &e))?;

    // Extract result from the CTE row
    extract_cte_result(&rows)
}

/// Build a single SQL string containing all session setup statements.
///
/// Combines SET LOCAL role, claims, statement_timeout, and pre-request function
/// into one semicolon-separated batch. This reduces per-request round-trips from
/// 5-9 down to 1.
fn build_session_setup(ctx: &ExecContext) -> Result<String, Error> {
    use std::fmt::Write;
    let mut sql = String::with_capacity(256);

    // SET LOCAL role
    if let Some(role) = &ctx.role {
        write!(sql, "SET LOCAL role = {};", quote_ident(role)).unwrap();
    }

    // SET LOCAL claims (bulk JSON + individual GUCs)
    if let Some(claims) = &ctx.claims {
        let claims_str = claims.to_string();
        let escaped = escape_literal(&claims_str)?;
        write!(sql, "SET LOCAL request.jwt.claims = '{escaped}';").unwrap();

        // Also set individual claim GUCs (request.jwt.claim.sub, etc.)
        if let Value::Object(map) = claims {
            for (key, val) in map {
                // Sanitize key: only allow safe GUC name characters
                if !is_safe_guc_key(key) {
                    tracing::debug!(key = %key, "skipping JWT claim with unsafe GUC key");
                    continue;
                }
                let val_str = match val {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                if let Ok(escaped_val) = escape_literal(&val_str) {
                    write!(
                        sql,
                        "SET LOCAL \"request.jwt.claim.{key}\" = '{escaped_val}';"
                    )
                    .unwrap();
                }
            }
        }
    }

    // SET LOCAL statement_timeout
    if let Some(timeout_ms) = ctx.statement_timeout {
        write!(sql, "SET LOCAL statement_timeout = '{timeout_ms}ms';").unwrap();
    }

    // Pre-request function call
    // pre_request is a qualified function name like "auth.check_request"
    // Quote each identifier part to prevent SQL injection via config values.
    if let Some(pre_req) = &ctx.pre_request {
        let quoted = pre_req
            .split('.')
            .map(|part| quote_ident(part))
            .collect::<Vec<_>>()
            .join(".");
        write!(sql, "SELECT {quoted}();").unwrap();
    }

    Ok(sql)
}

/// Check if a JWT claim key is safe to use as a GUC name component.
///
/// Rejects keys with characters that could break SET LOCAL syntax or
/// create invalid GUC names. Only allows alphanumeric, underscore, hyphen, and dot.
fn is_safe_guc_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 128
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
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

/// Quote a Postgres identifier (role name, schema name) using double-quote escaping.
/// Prevents SQL injection through crafted identifiers.
fn quote_ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// Escape a string literal for use in SET LOCAL statements.
/// Doubles single quotes and rejects null bytes to prevent SQL injection.
fn escape_literal(s: &str) -> Result<String, Error> {
    if s.contains('\0') {
        return Err(Error::Execution {
            message: "null byte in literal value".to_string(),
            db_code: None,
            detail: None,
            hint: Some("Remove null bytes from the value".to_string()),
        });
    }
    Ok(s.replace('\'', "''"))
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

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BytesMut;
    use serde_json::json;

    // -----------------------------------------------------------------------
    // NUMERIC encoding tests
    // -----------------------------------------------------------------------

    /// Decode the NUMERIC wire format from a BytesMut for test assertions.
    fn decode_numeric(buf: &[u8]) -> (i16, i16, u16, u16, Vec<i16>) {
        assert!(buf.len() >= 8, "buffer too short for NUMERIC header");
        let ndigits = i16::from_be_bytes([buf[0], buf[1]]);
        let weight = i16::from_be_bytes([buf[2], buf[3]]);
        let sign = u16::from_be_bytes([buf[4], buf[5]]);
        let dscale = u16::from_be_bytes([buf[6], buf[7]]);
        let mut digits = Vec::new();
        for i in 0..ndigits {
            let offset = 8 + (i as usize * 2);
            digits.push(i16::from_be_bytes([buf[offset], buf[offset + 1]]));
        }
        (ndigits, weight, sign, dscale, digits)
    }

    fn encode_numeric(s: &str) -> (i16, i16, u16, u16, Vec<i16>) {
        let mut buf = BytesMut::new();
        encode_numeric_str(s, &mut buf).unwrap();
        decode_numeric(&buf)
    }

    #[test]
    fn numeric_zero() {
        let (n, w, _s, d, digits) = encode_numeric("0");
        assert_eq!((n, w, d), (0, 0, 0));
        assert!(digits.is_empty());
    }

    #[test]
    fn numeric_zero_with_decimal() {
        let (n, w, _s, d, digits) = encode_numeric("0.00");
        assert_eq!((n, w, d), (0, 0, 2));
        assert!(digits.is_empty());
    }

    #[test]
    fn numeric_simple_integer() {
        // 1 => digits=[1], weight=0
        let (n, w, _s, d, digits) = encode_numeric("1");
        assert_eq!((n, w, d), (1, 0, 0));
        assert_eq!(digits, vec![1]);
    }

    #[test]
    fn numeric_large_integer() {
        // 12345 = 1*10000 + 2345 => digits=[1, 2345], weight=1
        let (n, w, _s, d, digits) = encode_numeric("12345");
        assert_eq!((n, w, d), (2, 1, 0));
        assert_eq!(digits, vec![1, 2345]);
    }

    #[test]
    fn numeric_ten_thousand() {
        // 10000 => digits=[1], weight=1 (trailing zero group trimmed)
        let (n, w, _s, d, digits) = encode_numeric("10000");
        assert_eq!((n, w, d), (1, 1, 0));
        assert_eq!(digits, vec![1]);
    }

    #[test]
    fn numeric_simple_fraction() {
        // 0.1 => frac "1" padded to "1000" => digit [1000], weight=-1, dscale=1
        let (n, w, _s, d, digits) = encode_numeric("0.1");
        assert_eq!((n, w, d), (1, -1, 1));
        assert_eq!(digits, vec![1000]);
    }

    #[test]
    fn numeric_0001() {
        // 0.0001 => frac "0001" => digit [1], weight=-1, dscale=4
        let (n, w, _s, d, digits) = encode_numeric("0.0001");
        assert_eq!((n, w, d), (1, -1, 4));
        assert_eq!(digits, vec![1]);
    }

    #[test]
    fn numeric_very_small_fraction() {
        // 0.00000001 => frac "00000001" => groups [0000, 0001] => [0, 1]
        // After trimming leading zero: [1], weight = -1 - 1(leading zeros) = -2
        let (n, w, _s, d, digits) = encode_numeric("0.00000001");
        assert_eq!((n, w, d), (1, -2, 8));
        assert_eq!(digits, vec![1]);
    }

    #[test]
    fn numeric_mixed() {
        // 123.456 => int "123" pad to "0123" => [123], frac "456" pad to "4560" => [4560]
        // digits=[123, 4560], weight=0, dscale=3
        let (n, w, _s, d, digits) = encode_numeric("123.456");
        assert_eq!((n, w, d), (2, 0, 3));
        assert_eq!(digits, vec![123, 4560]);
    }

    #[test]
    fn numeric_negative() {
        let (_, _, sign, _, _) = encode_numeric("-42");
        assert_eq!(sign, 0x4000);
    }

    #[test]
    fn numeric_positive_sign() {
        let (_, _, sign, _, _) = encode_numeric("+42");
        assert_eq!(sign, 0x0000);
    }

    #[test]
    fn numeric_nan() {
        let (n, w, sign, d, _) = encode_numeric("NaN");
        assert_eq!((n, w, sign, d), (0, 0, 0xC000, 0));
    }

    #[test]
    fn numeric_large_with_fraction() {
        // 99999999.99 => int "99999999" pad to "99999999" => [9999, 9999]
        //               frac "99" pad to "9900" => [9900]
        // digits=[9999, 9999, 9900], weight=1, dscale=2
        let (n, w, _s, d, digits) = encode_numeric("99999999.99");
        assert_eq!((n, w, d), (3, 1, 2));
        assert_eq!(digits, vec![9999, 9999, 9900]);
    }

    #[test]
    fn numeric_leading_zeros_in_integer() {
        // "007" should be same as "7"
        let (n, w, _s, d, digits) = encode_numeric("007");
        assert_eq!((n, w, d), (1, 0, 0));
        assert_eq!(digits, vec![7]);
    }

    // -----------------------------------------------------------------------
    // TextParam INT2/INT4 bounds tests
    // -----------------------------------------------------------------------

    #[test]
    fn text_param_int2_in_range() {
        let val = json!(32767); // i16::MAX
        let param = TextParam(&val);
        let mut buf = BytesMut::new();
        let result = param.to_sql(&Type::INT2, &mut buf);
        assert!(result.is_ok());
    }

    #[test]
    fn text_param_int2_out_of_range() {
        let val = json!(100000); // > i16::MAX
        let param = TextParam(&val);
        let mut buf = BytesMut::new();
        let result = param.to_sql(&Type::INT2, &mut buf);
        assert!(result.is_err());
    }

    #[test]
    fn text_param_int4_in_range() {
        let val = json!(2147483647); // i32::MAX
        let param = TextParam(&val);
        let mut buf = BytesMut::new();
        let result = param.to_sql(&Type::INT4, &mut buf);
        assert!(result.is_ok());
    }

    #[test]
    fn text_param_int4_out_of_range() {
        let val = json!(3000000000i64); // > i32::MAX
        let param = TextParam(&val);
        let mut buf = BytesMut::new();
        let result = param.to_sql(&Type::INT4, &mut buf);
        assert!(result.is_err());
    }

    #[test]
    fn text_param_null() {
        let val = json!(null);
        let param = TextParam(&val);
        let mut buf = BytesMut::new();
        let result = param.to_sql(&Type::TEXT, &mut buf);
        assert!(matches!(result, Ok(IsNull::Yes)));
    }

    #[test]
    fn text_param_bool_native() {
        let val = json!(true);
        let param = TextParam(&val);
        let mut buf = BytesMut::new();
        let result = param.to_sql(&Type::BOOL, &mut buf);
        assert!(result.is_ok());
    }

    // -----------------------------------------------------------------------
    // Session setup batch tests
    // -----------------------------------------------------------------------

    #[test]
    fn build_session_setup_empty() {
        let ctx = ExecContext::default();
        let sql = build_session_setup(&ctx).unwrap();
        assert!(sql.is_empty());
    }

    #[test]
    fn build_session_setup_with_role() {
        let ctx = ExecContext {
            role: Some("web_user".to_string()),
            ..Default::default()
        };
        let sql = build_session_setup(&ctx).unwrap();
        assert!(sql.contains("SET LOCAL role = \"web_user\""));
    }

    #[test]
    fn build_session_setup_role_with_quotes() {
        let ctx = ExecContext {
            role: Some("user\"name".to_string()),
            ..Default::default()
        };
        let sql = build_session_setup(&ctx).unwrap();
        // Double-quote escaping should produce: "user""name"
        assert!(sql.contains("\"user\"\"name\""));
    }

    #[test]
    fn build_session_setup_with_claims() {
        let ctx = ExecContext {
            claims: Some(json!({"sub": "user123", "role": "admin"})),
            ..Default::default()
        };
        let sql = build_session_setup(&ctx).unwrap();
        assert!(sql.contains("SET LOCAL request.jwt.claims"));
        assert!(sql.contains("request.jwt.claim.sub"));
        assert!(sql.contains("request.jwt.claim.role"));
    }

    #[test]
    fn build_session_setup_skips_null_byte_in_individual_claims() {
        // Null bytes in claim values cause individual GUC settings to be skipped
        // (the bulk JSON is escaped by serde_json so it's safe).
        // Construct a Value with a literal null byte in the string.
        let mut map = serde_json::Map::new();
        map.insert("safe".to_string(), Value::String("good".to_string()));
        map.insert("bad".to_string(), Value::String("user\x00evil".to_string()));
        let ctx = ExecContext {
            claims: Some(Value::Object(map)),
            ..Default::default()
        };
        let sql = build_session_setup(&ctx).unwrap();
        // The bulk claims JSON should still be set
        assert!(sql.contains("SET LOCAL request.jwt.claims"));
        // "safe" key should get its individual GUC
        assert!(sql.contains("request.jwt.claim.safe"));
        // "bad" key should be skipped (null byte in value fails escape_literal)
        assert!(!sql.contains("request.jwt.claim.bad"));
    }

    #[test]
    fn build_session_setup_with_timeout() {
        let ctx = ExecContext {
            statement_timeout: Some(5000),
            ..Default::default()
        };
        let sql = build_session_setup(&ctx).unwrap();
        assert!(sql.contains("SET LOCAL statement_timeout = '5000ms'"));
    }

    #[test]
    fn build_session_setup_with_pre_request() {
        let ctx = ExecContext {
            pre_request: Some("auth.check_request".to_string()),
            ..Default::default()
        };
        let sql = build_session_setup(&ctx).unwrap();
        // Each part of the qualified name is quoted
        assert!(sql.contains("SELECT \"auth\".\"check_request\"()"));
    }

    #[test]
    fn build_session_setup_full() {
        let ctx = ExecContext {
            role: Some("api_user".to_string()),
            claims: Some(json!({"sub": "abc"})),
            pre_request: Some("auth.pre".to_string()),
            statement_timeout: Some(30000),
            tx_end: None,
            is_mutation: false,
        };
        let sql = build_session_setup(&ctx).unwrap();
        // Should be a single string with all statements separated by semicolons
        assert!(sql.contains("SET LOCAL role"));
        assert!(sql.contains("SET LOCAL request.jwt.claims"));
        assert!(sql.contains("SET LOCAL statement_timeout"));
        assert!(sql.contains("SELECT \"auth\".\"pre\"()"));
        // Count semicolons to verify batching (at least 4 statements)
        assert!(sql.matches(';').count() >= 4);
    }

    // -----------------------------------------------------------------------
    // GUC key safety tests
    // -----------------------------------------------------------------------

    #[test]
    fn is_safe_guc_key_normal() {
        assert!(is_safe_guc_key("sub"));
        assert!(is_safe_guc_key("user_id"));
        assert!(is_safe_guc_key("org.name"));
        assert!(is_safe_guc_key("my-claim"));
    }

    #[test]
    fn is_safe_guc_key_unsafe() {
        assert!(!is_safe_guc_key("")); // empty
        assert!(!is_safe_guc_key("foo bar")); // space
        assert!(!is_safe_guc_key("foo'bar")); // quote
        assert!(!is_safe_guc_key("foo;bar")); // semicolon
        assert!(!is_safe_guc_key(&"a".repeat(200))); // too long
    }

    #[test]
    fn build_session_setup_skips_unsafe_claim_keys() {
        let ctx = ExecContext {
            claims: Some(json!({
                "safe_key": "value1",
                "unsafe key": "value2",
                "also;bad": "value3"
            })),
            ..Default::default()
        };
        let sql = build_session_setup(&ctx).unwrap();
        // The safe key should have its individual GUC set
        assert!(sql.contains("request.jwt.claim.safe_key"));
        // Unsafe keys should NOT get individual GUC settings
        // (but they still appear in the bulk request.jwt.claims JSON — that's fine)
        assert!(!sql.contains("request.jwt.claim.unsafe key"));
        assert!(!sql.contains("request.jwt.claim.also;bad"));
    }

    // -----------------------------------------------------------------------
    // Helper tests
    // -----------------------------------------------------------------------

    #[test]
    fn quote_ident_simple() {
        assert_eq!(quote_ident("my_role"), "\"my_role\"");
    }

    #[test]
    fn quote_ident_with_double_quotes() {
        assert_eq!(quote_ident("my\"role"), "\"my\"\"role\"");
    }

    #[test]
    fn escape_literal_simple() {
        assert_eq!(escape_literal("hello").unwrap(), "hello");
    }

    #[test]
    fn escape_literal_with_quotes() {
        assert_eq!(escape_literal("it's").unwrap(), "it''s");
    }

    #[test]
    fn escape_literal_rejects_null_byte() {
        assert!(escape_literal("hello\x00world").is_err());
    }

    // -----------------------------------------------------------------------
    // GUC header parsing tests
    // -----------------------------------------------------------------------

    #[test]
    fn parse_guc_headers_valid() {
        let raw = r#"[{"X-Custom": "value"}, {"Cache-Control": "no-cache"}]"#;
        let headers = parse_guc_headers(raw).unwrap();
        assert_eq!(headers.len(), 2);
        assert_eq!(headers[0], ("X-Custom".to_string(), "value".to_string()));
        assert_eq!(
            headers[1],
            ("Cache-Control".to_string(), "no-cache".to_string())
        );
    }

    #[test]
    fn parse_guc_headers_empty_array() {
        let raw = "[]";
        let headers = parse_guc_headers(raw).unwrap();
        assert!(headers.is_empty());
    }

    #[test]
    fn parse_guc_headers_invalid_json() {
        let raw = "not json";
        assert!(parse_guc_headers(raw).is_none());
    }
}
