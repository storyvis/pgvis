//! `pgvis-rest` — REST + OpenAPI adapter for pgvis.
//!
//! Provides [`build_app`] which takes a [`SchemaCache`](pgvis_core::SchemaCache) and produces an
//! axum Router with schema-driven routes.
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use std::sync::Arc;
//! use arc_swap::ArcSwap;
//! use pgvis_core::{Config, SchemaCache, dialect::POSTGRES};
//! use pgvis_rest::build_app;
//!
//! let cache = Arc::new(ArcSwap::new(Arc::new(SchemaCache::default())));
//! let config = Arc::new(Config::default());
//! let dialect = Arc::new(POSTGRES.clone());
//!
//! let app = build_app(cache, config, dialect);
//! // app is ready to serve with axum::serve(...)
//! ```

pub mod openapi;
pub mod routing;

pub use routing::{build_app, AppState};
