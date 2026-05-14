//! Foreign key relationships introspection query and row decoder.

use pgvis_core::cache::{Cardinality, QualifiedIdentifier, Relationship};
use pgvis_core::error::Error;
use serde::Deserialize;
use tokio_postgres::Client;

/// SQL query for relationships introspection (loaded at compile time).
const RELATIONSHIPS_SQL: &str = include_str!("../sql/relationships.sql");

/// Intermediate struct for deserialising the JSON column pairs from Postgres.
#[derive(Debug, Deserialize)]
struct ColumnPair {
    source: String,
    target: String,
}

/// Query all foreign key relationships across all schemas.
///
/// Returns M2O and O2O relationships. Inverse (O2M) and M2M relationships
/// are added during post-processing.
pub async fn query_relationships(client: &Client) -> Result<Vec<Relationship>, Error> {
    let rows = client
        .query(RELATIONSHIPS_SQL, &[])
        .await
        .map_err(|e| Error::Introspection(format!("relationships query failed: {e}")))?;

    let mut rels = Vec::new();

    for row in &rows {
        let table_schema: String = row.get("table_schema");
        let table_name: String = row.get("table_name");
        let foreign_table_schema: String = row.get("foreign_table_schema");
        let foreign_table_name: String = row.get("foreign_table_name");
        let is_self: bool = row.get("is_self");
        let constraint_name: String = row.get("constraint_name");
        let is_one_to_one: bool = row.get("is_one_to_one");

        // Decode column pairs from JSON
        let columns_json: serde_json::Value = row.get("columns");
        let col_pairs: Vec<ColumnPair> = serde_json::from_value(columns_json)
            .map_err(|e| Error::Introspection(format!("failed to decode relationship columns for {constraint_name}: {e}")))?;

        let source_columns: Vec<String> = col_pairs.iter().map(|p| p.source.clone()).collect();
        let target_columns: Vec<String> = col_pairs.iter().map(|p| p.target.clone()).collect();

        let source_table = QualifiedIdentifier::new(&table_schema, &table_name);
        let target_table = QualifiedIdentifier::new(&foreign_table_schema, &foreign_table_name);

        let cardinality = if is_one_to_one {
            Cardinality::O2O
        } else {
            Cardinality::M2O
        };

        rels.push(Relationship {
            source_table,
            target_table,
            source_columns,
            target_columns,
            cardinality,
            constraint_name,
            is_self,
        });
    }

    Ok(rels)
}
