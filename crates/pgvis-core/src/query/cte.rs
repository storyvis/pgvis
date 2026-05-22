//! # CTE wrapping — the unified response shape for all queries.
//!
//! Every query pgvis generates is wrapped in a CTE that produces the
//! [`QueryResult`] shape: `body` (JSON array), `page_total`, and optionally
//! `total_count`, `response_status` / `response_headers` (Postgres GUC readback).
//!
//! This is the PostgREST pattern — a single result row regardless of query type.

use crate::plan::types::CountStrategy;

use super::RenderContext;

/// Wrap an inner SQL query in the standard CTE envelope.
///
/// ## Parameters
///
/// - `inner_sql` — The paginated query (with LIMIT/OFFSET) for the response body.
/// - `count_source_sql` — Optional SQL without LIMIT/OFFSET for exact total count.
///   When provided with `count=exact`, a separate `pgrst_count` CTE counts all
///   matching rows regardless of pagination.
/// - `count` — The counting strategy from `Prefer: count=`.
/// - `ctx` — The render context.
///
/// ## Exact count
///
/// When `count=exact` and `count_source_sql` is provided:
/// ```sql
/// WITH pgrst_count AS (
///   SELECT count(*) AS total FROM (<count_source_sql>) _count_src
/// ),
/// pgrst_source AS (
///   <inner_sql with LIMIT/OFFSET>
/// )
/// SELECT
///   COALESCE(json_agg(_pgvis_t), '[]') AS body,
///   (SELECT count(*) FROM pgrst_source) AS page_total,
///   (SELECT total FROM pgrst_count) AS total_count,
///   ...
/// ```
///
/// When `count_source_sql` is `None` (e.g. for mutations), falls back to counting
/// from `pgrst_source` directly.
///
/// On SQLite (no GUC support), `response_status` and `response_headers` are omitted.
pub fn wrap_cte(
    inner_sql: &str,
    count_source_sql: Option<&str>,
    count: Option<&CountStrategy>,
    ctx: &mut RenderContext<'_>,
) {
    let json_agg = ctx.dialect.json_array_agg;
    let has_exact_count = matches!(count, Some(CountStrategy::Exact));
    let has_separate_count_source = count_source_sql.is_some() && has_exact_count;

    // Open CTE chain
    ctx.push_sql("WITH ");

    // If exact count is requested and we have a separate counting source,
    // emit pgrst_count CTE first
    if has_separate_count_source {
        ctx.push_sql("pgrst_count AS (\n  SELECT count(*) AS total FROM (");
        ctx.push_sql(count_source_sql.unwrap());
        ctx.push_sql(") _count_src\n),\n");
    }

    ctx.push_sql("pgrst_source AS (\n  ");
    ctx.push_sql(inner_sql);
    ctx.push_sql("\n)\nSELECT\n");

    // Body — aggregate all rows into JSON array
    ctx.push_sql("  COALESCE(");
    ctx.push_sql(json_agg);
    ctx.push_sql("(_pgvis_t), '[]') AS body");

    // Page total — count of rows in this page (post-LIMIT)
    ctx.push_sql(",\n  (SELECT count(*) FROM pgrst_source) AS page_total");

    // Total count (exact) — counts all matching rows regardless of pagination
    if has_exact_count {
        if has_separate_count_source {
            // Use the separate counting CTE (pre-LIMIT count)
            ctx.push_sql(",\n  (SELECT total FROM pgrst_count) AS total_count");
        } else {
            // Fallback: count from pgrst_source (e.g. for mutations without pagination)
            ctx.push_sql(",\n  (SELECT count(*) FROM pgrst_source) AS total_count");
        }
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
        let inner =
            "SELECT \"users\".\"id\", \"users\".\"name\" FROM \"public\".\"users\" AS \"users\"";

        wrap_cte(inner, None, None, &mut ctx);

        let sql = ctx.sql();
        assert!(sql.contains("WITH pgrst_source AS ("));
        assert!(sql.contains("json_agg(_pgvis_t)"));
        assert!(sql.contains("page_total"));
        assert!(sql.contains("response_status"));
        assert!(sql.contains("response_headers"));
        assert!(sql.contains("current_setting('response.status', true)"));
        // Should NOT have total_count when no count requested
        assert!(!sql.contains("total_count"));
    }

    #[test]
    fn test_cte_sqlite() {
        let mut ctx = RenderContext::new(&SQLITE);
        let inner = "SELECT \"users\".\"id\" FROM \"users\" AS \"users\"";

        wrap_cte(inner, None, None, &mut ctx);

        let sql = ctx.sql();
        assert!(sql.contains("WITH pgrst_source AS ("));
        assert!(sql.contains("json_group_array(_pgvis_t)"));
        assert!(sql.contains("page_total"));
        // SQLite should NOT have GUC readback
        assert!(!sql.contains("response_status"));
        assert!(!sql.contains("current_setting"));
    }

    #[test]
    fn test_cte_with_exact_count_separate_source() {
        let mut ctx = RenderContext::new(&POSTGRES);
        let inner = "SELECT * FROM \"users\" LIMIT 10 OFFSET 0";
        let count_src = "SELECT * FROM \"users\"";

        wrap_cte(
            inner,
            Some(count_src),
            Some(&CountStrategy::Exact),
            &mut ctx,
        );

        let sql = ctx.sql();
        assert!(sql.contains("pgrst_count AS ("));
        assert!(sql.contains("SELECT count(*) AS total FROM (SELECT * FROM \"users\") _count_src"));
        assert!(sql.contains("(SELECT total FROM pgrst_count) AS total_count"));
        assert!(sql.contains("pgrst_source AS ("));
        assert!(sql.contains("LIMIT 10 OFFSET 0"));
    }

    #[test]
    fn test_cte_with_exact_count_no_separate_source() {
        let mut ctx = RenderContext::new(&POSTGRES);
        let inner = "INSERT INTO \"users\" (\"name\") VALUES ($1) RETURNING *";

        wrap_cte(inner, None, Some(&CountStrategy::Exact), &mut ctx);

        let sql = ctx.sql();
        // Falls back to counting from pgrst_source
        assert!(sql.contains("(SELECT count(*) FROM pgrst_source) AS total_count"));
        assert!(!sql.contains("pgrst_count"));
    }
}
