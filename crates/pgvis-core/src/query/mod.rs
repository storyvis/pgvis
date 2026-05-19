//! # SQL Builder — renders `ActionPlan` into parameterised SQL.
//!
//! This module is the final step of the pgvis pipeline. It takes the fully-resolved
//! [`ActionPlan`] tree (from the plan layer) and emits a SQL string with positional
//! parameters, ready to pass to [`Backend::execute`].
//!
//! ## Design Principles
//!
//! 1. **Single-pass string assembly** — no intermediate SQL AST, no double rendering.
//! 2. **Zero external dependencies** — uses only `std::fmt::Write` and `serde_json::Value`.
//! 3. **Dialect-aware via `Dialect` struct** — pattern-matches on ~5 fields for syntax.
//! 4. **Never re-checks capabilities** — the plan layer already gated/annotated everything.
//! 5. **Deterministic output** — same plan + dialect always produces same SQL (snapshot-testable).

pub mod call;
pub mod cte;
pub mod fragment;
pub mod mutate;
pub mod read;

use serde_json::Value;

use crate::dialect::Dialect;
use crate::error::Error;
use crate::plan::ActionPlan;

pub use cte::wrap_cte;

// ---------------------------------------------------------------------------
// RenderContext — the stateful SQL assembly context
// ---------------------------------------------------------------------------

/// Stateful context for SQL rendering.
///
/// Tracks the SQL buffer, parameter values, and placeholder counter.
/// Passed mutably through all rendering functions.
pub struct RenderContext<'d> {
    /// The dialect controlling syntax differences.
    pub dialect: &'d Dialect,
    /// The SQL string being assembled.
    buf: String,
    /// Positional parameter values (in order).
    params: Vec<Value>,
    /// 1-based placeholder counter.
    param_count: u32,
    /// Alias counter for generating unique subquery aliases.
    alias_counter: u32,
}

impl<'d> RenderContext<'d> {
    /// Create a new render context for the given dialect.
    pub fn new(dialect: &'d Dialect) -> Self {
        Self {
            dialect,
            buf: String::with_capacity(512),
            params: Vec::with_capacity(8),
            param_count: 0,
            alias_counter: 0,
        }
    }

    /// Push a parameter value and return the placeholder string.
    ///
    /// - Postgres: `"$1"`, `"$2"`, …
    /// - SQLite: `"?"`, `"?"`, …
    pub fn push_param(&mut self, value: Value) -> String {
        self.param_count += 1;
        self.params.push(value);
        self.dialect.placeholder.render(self.param_count)
    }

    /// Quote an identifier (table/column name) using the dialect's quote character.
    pub fn quote_ident(&self, name: &str) -> String {
        let q = self.dialect.identifier_quote;
        format!("{q}{name}{q}")
    }

    /// Emit a qualified table reference: `"schema"."table"` or just `"table"`.
    ///
    /// Uses schema qualification only when `dialect.schema_namespacing` is true.
    pub fn qualified_table(&self, schema: &str, name: &str) -> String {
        if self.dialect.schema_namespacing {
            format!(
                "{}.{}",
                self.quote_ident(schema),
                self.quote_ident(name)
            )
        } else {
            self.quote_ident(name)
        }
    }

    /// Write raw SQL text to the buffer.
    pub fn push_sql(&mut self, sql: &str) {
        self.buf.push_str(sql);
    }

    /// Write a single character to the buffer.
    pub fn push_char(&mut self, c: char) {
        self.buf.push(c);
    }

    /// Generate a unique alias for a subquery.
    pub fn next_alias(&mut self, prefix: &str) -> String {
        self.alias_counter += 1;
        format!("_{prefix}_{}", self.alias_counter)
    }

    /// Consume the context and return the final SQL string and parameter values.
    pub fn finish(self) -> (String, Vec<Value>) {
        (self.buf, self.params)
    }

    /// Get a reference to the current SQL buffer (for debugging/testing).
    pub fn sql(&self) -> &str {
        &self.buf
    }

    /// Get a reference to the current parameters.
    pub fn params(&self) -> &[Value] {
        &self.params
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Render an [`ActionPlan`] into parameterised SQL.
///
/// This is the **only public function** of the SQL builder module.
/// Returns `(sql_string, parameter_values)` ready for [`Backend::execute`].
///
/// # Panics
///
/// Panics if called with `ActionPlan::Inspect` — those plans do not generate SQL.
///
/// # Errors
///
/// Returns [`Error::Internal`] if the plan contains inconsistencies that prevent
/// SQL generation (should not happen if the plan layer is correct).
pub fn render(plan: &ActionPlan, dialect: &Dialect) -> Result<(String, Vec<Value>), Error> {
    let mut ctx = RenderContext::new(dialect);

    match plan {
        ActionPlan::Read(read_plan) => {
            let inner_sql = read::render_read(read_plan, &mut ctx)?;
            cte::wrap_cte(&inner_sql, read_plan.count.as_ref(), &mut ctx);
        }
        ActionPlan::Mutate(mutate_plan) => {
            let inner_sql = mutate::render_mutate(mutate_plan, &mut ctx)?;
            cte::wrap_cte(&inner_sql, mutate_plan.count.as_ref(), &mut ctx);
        }
        ActionPlan::Call(call_plan) => {
            let inner_sql = call::render_call(call_plan, &mut ctx)?;
            cte::wrap_cte(&inner_sql, None, &mut ctx);
        }
        ActionPlan::Inspect(_) => {
            return Err(Error::Internal(
                "Inspect plans do not generate SQL".to_string(),
            ));
        }
    }

    Ok(ctx.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dialect::{POSTGRES, SQLITE};

    #[test]
    fn render_context_placeholder_postgres() {
        let mut ctx = RenderContext::new(&POSTGRES);
        let p1 = ctx.push_param(Value::from(42));
        let p2 = ctx.push_param(Value::from("hello"));
        assert_eq!(p1, "$1");
        assert_eq!(p2, "$2");
        assert_eq!(ctx.params().len(), 2);
    }

    #[test]
    fn render_context_placeholder_sqlite() {
        let mut ctx = RenderContext::new(&SQLITE);
        let p1 = ctx.push_param(Value::from(42));
        let p2 = ctx.push_param(Value::from("hello"));
        assert_eq!(p1, "?");
        assert_eq!(p2, "?");
    }

    #[test]
    fn render_context_qualified_table_postgres() {
        let ctx = RenderContext::new(&POSTGRES);
        assert_eq!(ctx.qualified_table("public", "users"), "\"public\".\"users\"");
    }

    #[test]
    fn render_context_qualified_table_sqlite() {
        let ctx = RenderContext::new(&SQLITE);
        // SQLite has no schema namespacing
        assert_eq!(ctx.qualified_table("main", "users"), "\"users\"");
    }
}
