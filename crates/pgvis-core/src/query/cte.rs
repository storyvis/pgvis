//! # CTE wrapping — the unified response shape for all queries.
//!
//! Every query pgvis generates is wrapped in a CTE that produces the
//! [`QueryResult`] shape: `body` (JSON array), `page_total`, and optionally
//! `response_status` / `response_headers` (Postgres GUC readback).
//!
//! This is the PostgREST pattern — a single result row regardless of query type.

use crate::plan::types::CountStrategy;

use super::RenderContext;

/// Wrap an inner SQL query in the standard CTE envelope.
///
/// Produces:
/// ```sql
/// WITH pgrst_source AS (
///   <inner_sql>
/// )
/// SELECT
///   COALESCE(json_agg(_pgvis_t), '[]') AS body,
///   (SELECT count(*) FROM pgrst_source) AS page_total,
///   current_setting('response.status', true) AS response_status,
///   current_setting('response.headers', true) AS response_headers
/// FROM (SELECT * FROM pgrst_source) _pgvis_t
/// ```
///
/// On SQLite (no GUC support), `response_status` and `response_headers` are omitted.
pub fn wrap_cte(
    inner_sql: &str,
    count: Option<&CountStrategy>,
    ctx: &mut RenderContext<'_>,
) {
    let json_agg = ctx.dialect.json_array_agg;

    // Open CTE
    ctx.push_sql("WITH pgrst_source AS (\n  ");
    ctx.push_sql(inner_sql);
    ctx.push_sql("\n)\nSELECT\n");

    // Body — aggregate all rows into JSON array
    ctx.push_sql("  COALESCE(");
    ctx.push_sql(json_agg);
    ctx.push_sql("(_pgvis_t), '[]') AS body");

    // Page total — count of rows in this page
    ctx.push_sql(",\n  (SELECT count(*) FROM pgrst_source) AS page_total");

    // Total count (exact) — only if requested
    if let Some(CountStrategy::Exact) = count {
        // For exact count, the page_total from the CTE IS the total
        // (pre-LIMIT count would need a separate CTE — simplified for now)
        ctx.push_sql(",\n  (SELECT count(*) FROM pgrst_source) AS total_count");
    }

    // GUC readback — Postgres only
    if ctx.dialect.supports_set_local {
        ctx.push_sql(",\n  current_setting('response.status', true) AS response_status");
        ctx.push_sql(",\n  current_setting('response.headers', true) AS response_headers");
    }

    // FROM clause
    ctx.push_sql("\nFROM (SELECT * FROM pgrst_source) _pgvis_t");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dialect::{POSTGRES, SQLITE};

    #[test]
    fn test_cte_postgres() {
        let mut ctx = RenderContext::new(&POSTGRES);
        let inner = "SELECT \"users\".\"id\", \"users\".\"name\" FROM \"public\".\"users\" AS \"users\"";

        wrap_cte(inner, None, &mut ctx);

        let sql = ctx.sql();
        assert!(sql.contains("WITH pgrst_source AS ("));
        assert!(sql.contains("json_agg(_pgvis_t)"));
        assert!(sql.contains("page_total"));
        assert!(sql.contains("response_status"));
        assert!(sql.contains("response_headers"));
        assert!(sql.contains("current_setting('response.status', true)"));
    }

    #[test]
    fn test_cte_sqlite() {
        let mut ctx = RenderContext::new(&SQLITE);
        let inner = "SELECT \"users\".\"id\" FROM \"users\" AS \"users\"";

        wrap_cte(inner, None, &mut ctx);

        let sql = ctx.sql();
        assert!(sql.contains("WITH pgrst_source AS ("));
        assert!(sql.contains("json_group_array(_pgvis_t)"));
        assert!(sql.contains("page_total"));
        // SQLite should NOT have GUC readback
        assert!(!sql.contains("response_status"));
        assert!(!sql.contains("current_setting"));
    }

    #[test]
    fn test_cte_with_exact_count() {
        let mut ctx = RenderContext::new(&POSTGRES);
        let inner = "SELECT * FROM \"users\"";

        wrap_cte(inner, Some(&CountStrategy::Exact), &mut ctx);

        let sql = ctx.sql();
        assert!(sql.contains("total_count"));
    }
}
