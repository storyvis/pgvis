//! `pgvis-rest` — REST + OpenAPI adapter for pgvis.
//!
//! Provides [`build_app`] which takes a [`SchemaCache`] and produces a paired
//! `(axum::Router, OpenAPI)` — routes and their documentation in a single pass.

pub mod openapi;
pub mod routing;
