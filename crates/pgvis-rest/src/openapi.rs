//! OpenAPI 3.0 spec generation from the schema cache.
//!
//! Generates a minimal but functional OpenAPI spec documenting all exposed
//! tables, views, and RPC functions based on the current [`SchemaCache`].

use indexmap::IndexMap;
use openapiv3::{Info, OpenAPI, Operation, PathItem, Paths, ReferenceOr, Server};
use pgvis_core::{Config, SchemaCache};

/// Generate an OpenAPI 3.0 spec for the exposed schemas.
///
/// The spec includes:
/// - One path per table/view with GET, POST, PATCH, DELETE operations
/// - One path per RPC function with POST (and GET for stable/immutable)
///
/// # Arguments
///
/// * `cache` — The current schema cache snapshot
/// * `config` — Configuration (routing, schemas, OpenAPI settings)
pub fn generate_spec(cache: &SchemaCache, config: &Config) -> OpenAPI {
    let mut paths: IndexMap<String, ReferenceOr<PathItem>> = IndexMap::new();
    let routing = &config.routing;
    let prefix = routing.normalized_prefix();

    for schema in &config.schemas {
        // Generate paths for tables/views
        for (ident, table) in &cache.tables {
            if ident.schema != *schema {
                continue;
            }

            let path = build_path(prefix, routing.schema_in_path, schema, &ident.name, false);

            let get_op = Operation {
                summary: Some(format!("List rows from {}.{}", schema, ident.name)),
                operation_id: Some(format!("{schema}_list_{}", ident.name)),
                tags: vec![schema.clone()],
                description: table.description.clone(),
                ..Default::default()
            };

            let post_op = if table.insertable {
                Some(Operation {
                    summary: Some(format!("Insert into {}.{}", schema, ident.name)),
                    operation_id: Some(format!("{schema}_insert_{}", ident.name)),
                    tags: vec![schema.clone()],
                    ..Default::default()
                })
            } else {
                None
            };

            let patch_op = if table.updatable {
                Some(Operation {
                    summary: Some(format!("Update rows in {}.{}", schema, ident.name)),
                    operation_id: Some(format!("{schema}_update_{}", ident.name)),
                    tags: vec![schema.clone()],
                    ..Default::default()
                })
            } else {
                None
            };

            let delete_op = if table.deletable {
                Some(Operation {
                    summary: Some(format!("Delete rows from {}.{}", schema, ident.name)),
                    operation_id: Some(format!("{schema}_delete_{}", ident.name)),
                    tags: vec![schema.clone()],
                    ..Default::default()
                })
            } else {
                None
            };

            let path_item = PathItem {
                get: Some(get_op),
                post: post_op,
                patch: patch_op,
                delete: delete_op,
                ..Default::default()
            };

            paths.insert(path, ReferenceOr::Item(path_item));
        }

        // Generate paths for RPC functions
        for (ident, routines) in &cache.routines {
            if ident.schema != *schema {
                continue;
            }

            let path = build_path(prefix, routing.schema_in_path, schema, &ident.name, true);

            let description = routines.first().and_then(|r| r.description.clone());

            let post_op = Operation {
                summary: Some(format!("Call {}.{}", schema, ident.name)),
                operation_id: Some(format!("{schema}_call_{}", ident.name)),
                tags: vec![schema.clone()],
                description,
                ..Default::default()
            };

            // Stable/immutable functions also accept GET
            let get_op = routines.first().and_then(|r| {
                use pgvis_core::cache::Volatility;
                match r.volatility {
                    Volatility::Immutable | Volatility::Stable => Some(Operation {
                        summary: Some(format!("Call {}.{} (read-only)", schema, ident.name)),
                        operation_id: Some(format!("{schema}_get_{}", ident.name)),
                        tags: vec![schema.clone()],
                        ..Default::default()
                    }),
                    Volatility::Volatile => None,
                }
            });

            let path_item = PathItem {
                post: Some(post_op),
                get: get_op,
                ..Default::default()
            };

            paths.insert(path, ReferenceOr::Item(path_item));
        }
    }

    let servers = config
        .openapi_server_url
        .as_ref()
        .map(|url| {
            vec![Server {
                url: url.clone(),
                ..Default::default()
            }]
        })
        .unwrap_or_default();

    OpenAPI {
        openapi: "3.0.3".to_string(),
        info: Info {
            title: config
                .openapi_title
                .clone()
                .unwrap_or_else(|| "pgvis API".to_string()),
            version: "0.1.0".to_string(),
            ..Default::default()
        },
        servers,
        paths: Paths {
            paths,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Build a URL path for a given resource.
fn build_path(
    prefix: &str,
    schema_in_path: bool,
    schema: &str,
    name: &str,
    is_rpc: bool,
) -> String {
    let resource = if is_rpc {
        format!("rpc/{name}")
    } else {
        name.to_string()
    };

    match (schema_in_path, prefix.is_empty()) {
        (true, true) => format!("/{schema}/{resource}"),
        (true, false) => format!("/{prefix}/{schema}/{resource}"),
        (false, true) => format!("/{resource}"),
        (false, false) => format!("/{prefix}/{resource}"),
    }
}
