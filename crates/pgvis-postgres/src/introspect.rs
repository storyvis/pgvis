//! Schema introspection queries for Postgres.
//!
//! Ports the SQL from PostgREST's `SchemaCache.hs`.

use pgvis_core::backend::IntrospectConfig;
use pgvis_core::cache::SchemaCache;
use pgvis_core::error::Error;
use tokio_postgres::Client;

/// Load the full schema cache by running introspection queries.
pub async fn load_schema_cache(
    _client: &Client,
    _cfg: &IntrospectConfig,
) -> Result<SchemaCache, Error> {
    // TODO: Port introspection SQL from PostgREST SchemaCache.hs
    // Phase 2 of implementation plan
    Ok(SchemaCache::default())
}
