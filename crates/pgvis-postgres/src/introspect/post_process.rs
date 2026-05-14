//! Post-processing of introspection results.
//!
//! These algorithms are backend-agnostic — they operate on the already-populated
//! `SchemaCache` and infer additional relationships that can't be discovered
//! from a single query.

use std::collections::HashSet;

use pgvis_core::cache::{Cardinality, QualifiedIdentifier, Relationship, SchemaCache};

/// Add inverse relationships for every M2O and O2O relationship.
///
/// - For each M2O (A → B), adds O2M (B → A) with swapped columns
/// - For each O2O (A → B), adds O2O (B → A) with swapped columns
///
/// This matches PostgREST's `addInverseRels` behaviour.
pub fn add_inverse_relationships(cache: &mut SchemaCache) {
    let existing: Vec<Relationship> = cache.relationships.clone();
    let mut inverse_rels = Vec::new();

    for rel in &existing {
        match &rel.cardinality {
            Cardinality::M2O => {
                // M2O(A→B) becomes O2M(B→A) with swapped columns
                inverse_rels.push(Relationship {
                    source_table: rel.target_table.clone(),
                    target_table: rel.source_table.clone(),
                    source_columns: rel.target_columns.clone(),
                    target_columns: rel.source_columns.clone(),
                    cardinality: Cardinality::O2M,
                    constraint_name: rel.constraint_name.clone(),
                    is_self: rel.is_self,
                });
            }
            Cardinality::O2O => {
                // O2O(A→B) becomes O2O(B→A) with swapped columns
                inverse_rels.push(Relationship {
                    source_table: rel.target_table.clone(),
                    target_table: rel.source_table.clone(),
                    source_columns: rel.target_columns.clone(),
                    target_columns: rel.source_columns.clone(),
                    cardinality: Cardinality::O2O,
                    constraint_name: rel.constraint_name.clone(),
                    is_self: rel.is_self,
                });
            }
            // O2M and M2M are already derived — don't double-invert
            _ => {}
        }
    }

    cache.relationships.extend(inverse_rels);
}

/// Infer Many-to-Many relationships from pairs of M2O relationships.
///
/// A junction table is identified when:
/// 1. It has at least two M2O relationships (two different FKs)
/// 2. The union of FK source columns is a subset of the junction table's PK columns
///
/// For each qualifying pair, a M2M relationship is created between the two
/// target tables, via the junction table.
///
/// This matches PostgREST's `addM2MRels` behaviour.
pub fn infer_m2m_relationships(cache: &mut SchemaCache) {
    let existing: Vec<Relationship> = cache.relationships.clone();
    let mut m2m_rels = Vec::new();

    // Index M2O rels by their source table (potential junction tables)
    let mut m2o_by_source: std::collections::HashMap<&QualifiedIdentifier, Vec<&Relationship>> =
        std::collections::HashMap::new();

    for rel in &existing {
        if matches!(rel.cardinality, Cardinality::M2O) {
            m2o_by_source
                .entry(&rel.source_table)
                .or_default()
                .push(rel);
        }
    }

    // For each potential junction table with 2+ M2O rels
    for (junction_ident, rels) in &m2o_by_source {
        if rels.len() < 2 {
            continue;
        }

        // Get the junction table's PK columns
        let pk_cols: HashSet<&str> = match cache.tables.get(*junction_ident) {
            Some(table) => table.pk_cols.iter().map(String::as_str).collect(),
            None => continue,
        };

        if pk_cols.is_empty() {
            continue;
        }

        // Try all pairs of M2O rels on this junction
        for (i, rel1) in rels.iter().enumerate() {
            for rel2 in rels.iter().skip(i + 1) {
                // Skip if both point to the same constraint
                if rel1.constraint_name == rel2.constraint_name {
                    continue;
                }

                // Check: union of FK source columns ⊆ PK columns
                let fk_cols: HashSet<&str> = rel1
                    .source_columns
                    .iter()
                    .chain(rel2.source_columns.iter())
                    .map(String::as_str)
                    .collect();

                if !fk_cols.is_subset(&pk_cols) {
                    continue;
                }

                // This is a valid M2M relationship!
                let is_self = rel1.target_table == rel2.target_table;

                m2m_rels.push(Relationship {
                    source_table: rel1.target_table.clone(),
                    target_table: rel2.target_table.clone(),
                    // For M2M, source_columns are the FK cols in junction pointing to source
                    source_columns: rel1.target_columns.clone(),
                    target_columns: rel2.target_columns.clone(),
                    cardinality: Cardinality::M2M {
                        junction_table: (*junction_ident).clone(),
                        junction_cols_source: rel1.source_columns.clone(),
                        junction_cols_target: rel2.source_columns.clone(),
                    },
                    constraint_name: format!(
                        "{}_{}", rel1.constraint_name, rel2.constraint_name
                    ),
                    is_self,
                });
            }
        }
    }

    cache.relationships.extend(m2m_rels);
}

/// Mark columns that participate in foreign keys.
///
/// Sets `column.is_fk = true` for all columns that appear as source columns
/// in any relationship.
pub fn mark_fk_columns(cache: &mut SchemaCache) {
    // Collect all (table, column) pairs that are FK sources
    let fk_cols: HashSet<(QualifiedIdentifier, String)> = cache
        .relationships
        .iter()
        .flat_map(|rel| {
            rel.source_columns
                .iter()
                .map(|col| (rel.source_table.clone(), col.clone()))
        })
        .collect();

    // Update the columns
    for (ident, table) in &mut cache.tables {
        for (col_name, col) in &mut table.columns {
            if fk_cols.contains(&(ident.clone(), col_name.clone())) {
                col.is_fk = true;
            }
        }
    }
}
