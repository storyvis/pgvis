//! `pgvis-router` — Embeddable REST + OpenAPI router for pgvis.
//!
//! Provides [`build_app`] which takes a [`SchemaCache`](pgvis_core::SchemaCache) and produces an
//! axum Router with schema-driven routes. Mount it into any axum application.
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use std::sync::Arc;
//! use arc_swap::ArcSwap;
//! use pgvis_core::{Config, SchemaCache, dialect::POSTGRES};
//! use pgvis_router::build_app;
//!
//! let cache = Arc::new(ArcSwap::new(Arc::new(SchemaCache::default())));
//! let config = Arc::new(Config::default());
//! let dialect = Arc::new(POSTGRES.clone());
//!
//! let app = build_app(cache, config, dialect);
//! // app is ready to serve with axum::serve(...)
//! ```
//!
//! ## Embedding in an existing app
//!
//! ```rust,no_run
//! use axum::Router;
//! use axum::routing::get;
//! # use std::sync::Arc;
//! # use arc_swap::ArcSwap;
//! # use pgvis_core::{Config, SchemaCache, dialect::POSTGRES};
//! # use pgvis_router::build_app;
//! # let cache = Arc::new(ArcSwap::new(Arc::new(SchemaCache::default())));
//! # let config = Arc::new(Config::default());
//! # let dialect = Arc::new(POSTGRES.clone());
//!
//! let pgvis_api = build_app(cache, config, dialect);
//! let my_app = Router::new()
//!     .nest("/db", pgvis_api)
//!     .route("/health", get(|| async { "ok" }));
//! ```

pub mod openapi;
pub mod response;
pub mod routing;

pub use routing::{build_app, AppState};
