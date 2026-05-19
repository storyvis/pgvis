//! Schema introspection for Postgres.
//!
//! Queries `pg_catalog` system tables to discover tables, columns, relationships,
//! routines, and data representations. Assembles results into a [`SchemaCache`].
//!
//! Ported from PostgREST's `SchemaCache.hs`.

pub mod tables;
pub mod relationships;
pub mod routines;
pub mod representations;
pub mod post_process;

use pgvis_core::backend::IntrospectConfig;
use pgvis_core::cache::SchemaCache;
use pgvis_core::error::Error;
use tokio_postgres::Client;
use tracing::info;

/// Load the full schema cache by running introspection queries against Postgres.
///
/// Executes all introspection queries within the scope of a voided `search_path`
/// (ensuring fully-qualified names), then applies post-processing to infer
/// M2M relationships and inverse relationships.
///
/// # Parameters
///
/// - `client` — A connected `tokio_postgres::Client`
/// - `cfg` — Specifies which schemas to introspect
///
/// # Errors
///
/// Returns [`Error::Introspection`] if any introspection query fails.
pub async fn load_schema_cache(
    client: &Client,
    cfg: &IntrospectConfig,
) -> Result<SchemaCache, Error> {
    // Wrap in a transaction so SET LOCAL takes effect for all introspection queries.
    // Without a transaction, SET LOCAL is a no-op on pooled connections.
    client
        .batch_execute("BEGIN; SET LOCAL search_path = ''")
        .await
        .map_err(|e| Error::Introspection(format!("failed to begin introspection tx: {e}")))?;

    let result = run_introspection(client, cfg).await;

    // Always end the transaction (COMMIT on success, ROLLBACK on error).
    // Either way, SET LOCAL is reverted when the transaction ends.
    let end_sql = if result.is_ok() { "COMMIT" } else { "ROLLBACK" };
    client
        .batch_execute(end_sql)
        .await
        .map_err(|e| Error::Introspection(format!("failed to end introspection tx: {e}")))?;

    result
}

/// Inner implementation that runs inside the transaction opened by [`load_schema_cache`].
async fn run_introspection(
    client: &Client,
    cfg: &IntrospectConfig,
) -> Result<SchemaCache, Error> {
    let schemas: &[String] = &cfg.schemas;

    info!(schemas = ?cfg.schemas, "introspecting database schema");

    let tables = tables::query_tables(client, schemas).await?;
    let rels = relationships::query_relationships(client, schemas).await?;
    let routines = routines::query_routines(client, schemas).await?;
    let representations = representations::query_representations(client).await?;

    // Assemble the cache
    let mut cache = SchemaCache {
        built_at: Some(std::time::SystemTime::now()),
        schema_version: None,
        tables,
        relationships: rels,
        computed_relationships: Vec::new(), // TODO: allComputedRels
        routines,
        representations,
        media_handlers: std::collections::HashMap::new(), // TODO: mediaHandlers
    };

    // Post-processing order matches PostgREST: M2M inference first, then inverse rels.
    // In Haskell: `addInverseRels $ addM2MRels` — right-to-left application.
    post_process::infer_m2m_relationships(&mut cache);
    post_process::add_inverse_relationships(&mut cache);
    post_process::mark_fk_columns(&mut cache);

    let table_count = cache.tables.len();
    let rel_count = cache.relationships.len();
    let routine_count = cache.routines.len();
    info!(
        tables = table_count,
        relationships = rel_count,
        routines = routine_count,
        "schema cache loaded"
    );

    Ok(cache)
}
