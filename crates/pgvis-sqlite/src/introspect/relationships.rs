//! Foreign key relationships introspection via `PRAGMA foreign_key_list`.
//!
//! Discovers M2O and O2O relationships. The O2O determination checks whether
//! the FK source columns have a unique constraint on them.

use indexmap::IndexMap;
use pgvis_core::cache::{Cardinality, QualifiedIdentifier, Relationship, Table, UniqueConstraint};
use pgvis_core::error::Error;
use tokio_rusqlite::Connection;

use crate::util::{escape_ident, SqliteInternalError};

/// Query all foreign key relationships from the introspected tables.
///
/// For each table, runs `PRAGMA foreign_key_list(table)` to discover FK constraints.
/// Determines O2O vs M2O by checking whether the FK source columns have a unique
/// constraint covering exactly those columns.
///
/// Returns M2O and O2O relationships. Inverse (O2M) and M2M relationships
/// are added during post-processing.
pub async fn query_relationships(
    conn: &Connection,
    tables: &IndexMap<QualifiedIdentifier, Table>,
) -> Result<Vec<Relationship>, Error> {
    let table_names: Vec<String> = tables.keys().map(|k| k.name.clone()).collect();
    let tables_clone = tables.clone();

    conn.call(move |conn| {
        let mut rels = Vec::new();

        for table_name in &table_names {
            let table_rels = query_table_fks(conn, table_name, &tables_clone)
                .map_err(|e| tokio_rusqlite::Error::Other(Box::new(e)))?;
            rels.extend(table_rels);
        }

        Ok(rels)
    })
    .await
    .map_err(|e| Error::Introspection(format!("SQLite relationships introspection failed: {e}")))
}

/// Query foreign keys for a single table using `PRAGMA foreign_key_list`.
fn query_table_fks(
    conn: &rusqlite::Connection,
    table_name: &str,
    tables: &IndexMap<QualifiedIdentifier, Table>,
) -> Result<Vec<Relationship>, SqliteInternalError> {
    let sql = format!("PRAGMA foreign_key_list(\"{}\")", escape_ident(table_name));
    let mut stmt = conn.prepare(&sql).map_err(|e| {
        SqliteInternalError(format!("failed to prepare foreign_key_list for {table_name}: {e}"))
    })?;

    // foreign_key_list columns: id, seq, table, from, to, on_update, on_delete, match
    // Multiple rows with same `id` form a composite FK (multi-column)
    let mut fk_map: std::collections::BTreeMap<i32, FkEntry> = std::collections::BTreeMap::new();

    let mut rows = stmt.query([]).map_err(|e| {
        SqliteInternalError(format!("failed to query foreign_key_list for {table_name}: {e}"))
    })?;

    while let Some(row) = rows.next().map_err(|e| {
        SqliteInternalError(format!("foreign_key_list iteration failed for {table_name}: {e}"))
    })? {
        let id: i32 = row.get(0).unwrap_or(0);
        let _seq: i32 = row.get(1).unwrap_or(0);
        let target_table: String = row.get(2).unwrap_or_default();
        let from_col: String = row.get(3).unwrap_or_default();
        let to_col: String = row.get(4).unwrap_or_default();

        let entry = fk_map.entry(id).or_insert_with(|| FkEntry {
            target_table: target_table.clone(),
            source_columns: Vec::new(),
            target_columns: Vec::new(),
        });

        entry.source_columns.push(from_col);
        entry.target_columns.push(to_col);
    }

    // Convert FK entries to Relationship structs
    let source_ident = QualifiedIdentifier::new("main", table_name);
    let source_table_meta = tables.get(&source_ident);

    let mut rels = Vec::new();
    for (id, entry) in fk_map {
        let target_ident = QualifiedIdentifier::new("main", &entry.target_table);
        let is_self = source_ident == target_ident;

        // Determine cardinality: O2O if source columns have a unique constraint
        let is_one_to_one = source_table_meta
            .map(|t| has_unique_on_columns(&t.unique_constraints, &entry.source_columns))
            .unwrap_or(false);

        let cardinality = if is_one_to_one {
            Cardinality::O2O
        } else {
            Cardinality::M2O
        };

        // Synthesize a constraint name (SQLite doesn't name FK constraints)
        let constraint_name = format!(
            "{table_name}_{}_fkey_{id}",
            entry.source_columns.join("_")
        );

        rels.push(Relationship {
            source_table: source_ident.clone(),
            target_table: target_ident,
            source_columns: entry.source_columns,
            target_columns: entry.target_columns,
            cardinality,
            constraint_name,
            is_self,
        });
    }

    Ok(rels)
}

/// Check if there's a unique constraint that covers exactly the given columns.
fn has_unique_on_columns(constraints: &[UniqueConstraint], columns: &[String]) -> bool {
    constraints.iter().any(|uc| {
        let mut uc_sorted = uc.columns.clone();
        uc_sorted.sort();
        let mut cols_sorted = columns.to_vec();
        cols_sorted.sort();
        uc_sorted == cols_sorted
    })
}

/// Intermediate FK entry for grouping composite foreign keys.
struct FkEntry {
    target_table: String,
    source_columns: Vec<String>,
    target_columns: Vec<String>,
}

