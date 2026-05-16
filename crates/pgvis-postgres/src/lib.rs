//! # `pgvis-postgres` — Postgres backend for pgvis.
//!
//! Implements [`pgvis_core::Backend`] using `tokio-postgres` + `deadpool-postgres`.
//!
//! ## Responsibilities
//!
//! - **Connection pooling** via `deadpool-postgres`
//! - **Schema introspection** from `pg_catalog` (tables, columns, FKs, functions)
//! - **Query execution** within role-switched transactions
//! - **Schema change notifications** via `LISTEN/NOTIFY` (planned)
//!
//! ## Example
//!
//! ```rust,ignore
//! use pgvis_postgres::PgBackend;
//! use pgvis_core::{Backend, IntrospectConfig};
//!
//! let backend = PgBackend::new("postgres://user:pass@localhost/db")?;
//! let cache = backend.introspect(&IntrospectConfig::default()).await?;
//! println!("Found {} tables", cache.tables.len());
//! ```

pub mod execute;
pub mod introspect;

use deadpool_postgres::{Config as PoolConfig, Pool, Runtime};
use futures::future::BoxFuture;
use pgvis_core::backend::{Backend, ExecContext, IntrospectConfig, QueryResult, SchemaChangeStream};
use pgvis_core::cache::SchemaCache;
use pgvis_core::dialect::{self, Dialect};
use pgvis_core::error::Error;
use serde_json::Value;
use tokio_postgres::NoTls;

/// The Postgres backend — implements [`Backend`] for PostgreSQL databases.
///
/// Holds a connection pool (`deadpool-postgres`) and provides:
/// - `introspect()` — loads schema metadata from `pg_catalog`
/// - `execute()` — runs CTE-wrapped SQL within a transaction
/// - `watch_schema()` — LISTEN/NOTIFY for schema changes (planned)
/// - `dialect()` — returns [`POSTGRES`](pgvis_core::dialect::POSTGRES)
pub struct PgBackend {
    pool: Pool,
}

impl PgBackend {
    /// Create a new Postgres backend from a DSN.
    ///
    /// Initialises the connection pool but does NOT connect immediately —
    /// connections are created lazily on first use.
    ///
    /// # Arguments
    ///
    /// * `dsn` — A PostgreSQL connection string (e.g. `postgres://user:pass@host/db`)
    ///
    /// # Errors
    ///
    /// Returns [`Error::Introspection`] if the pool configuration is invalid.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let backend = PgBackend::new("postgres://localhost/mydb")?;
    /// ```
    pub fn new(dsn: &str) -> Result<Self, Error> {
        let mut cfg = PoolConfig::new();
        cfg.url = Some(dsn.to_string());

        let pool = cfg
            .create_pool(Some(Runtime::Tokio1), NoTls)
            .map_err(|e| Error::Introspection(format!("failed to create pool: {e}")))?;

        Ok(Self { pool })
    }

    /// Get a reference to the underlying connection pool.
    ///
    /// Useful for advanced use cases (custom queries, health checks, metrics).
    pub fn pool(&self) -> &Pool {
        &self.pool
    }
}

impl Backend for PgBackend {
    fn introspect(&self, cfg: &IntrospectConfig) -> BoxFuture<'_, Result<SchemaCache, Error>> {
        let cfg = cfg.clone();
        Box::pin(async move {
            let client = self
                .pool
                .get()
                .await
                .map_err(|e| Error::Introspection(format!("pool error: {e}")))?;

            introspect::load_schema_cache(&client, &cfg).await
        })
    }

    fn execute(
        &self,
        ctx: &ExecContext,
        sql: &str,
        params: &[Value],
    ) -> BoxFuture<'_, Result<QueryResult, Error>> {
        let sql = sql.to_string();
        let params = params.to_vec();
        let ctx = ctx.clone();
        Box::pin(async move {
            let client = self
                .pool
                .get()
                .await
                .map_err(|e| Error::Execution {
                    message: format!("pool error: {e}"),
                    db_code: None,
                    detail: None,
                    hint: None,
                })?;

            execute::execute_query(&client, &ctx, &sql, &params).await
        })
    }

    fn watch_schema(&self) -> BoxFuture<'_, Option<SchemaChangeStream>> {
        Box::pin(async {
            // TODO: Implement LISTEN/NOTIFY for schema change detection
            None
        })
    }

    fn dialect(&self) -> &'static Dialect {
        &dialect::POSTGRES
    }
}
