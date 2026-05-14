//! # `pgvis-core` — Backend-agnostic database narration primitives
//!
//! This crate is the **I/O-free foundation** of the pgvis workspace. It defines:
//!
//! - The [`Backend`] trait that database drivers (`pgvis-postgres`, `pgvis-sqlite`) implement
//! - The [`SchemaCache`] and related types describing introspected database metadata
//! - The [`Dialect`] struct parametrising SQL generation for different databases
//! - The [`Error`] type used throughout the stack
//! - Shared [`Config`] types consumed by backends and adapters
//!
//! ## Architectural Role
//!
//! ```text
//! pgvis-core (this crate)
//!   │
//!   ├── defines trait Backend ──► implemented by pgvis-postgres, pgvis-sqlite
//!   ├── defines SchemaCache   ──► consumed by pgvis-rest, pgvis-mcp
//!   ├── defines Dialect       ──► used by SQL builder (planned: pgvis-core::query)
//!   └── defines Error/Config  ──► shared across all crates
//! ```
//!
//! ## No I/O Guarantee
//!
//! This crate has **zero runtime I/O dependencies**:
//! - No `tokio-postgres`, no `sqlx`, no `rusqlite`
//! - No `axum`, no `hyper`, no HTTP framework
//! - No filesystem access, no network calls
//!
//! All async operations are defined as trait methods returning [`futures::future::BoxFuture`],
//! making the trait **object-safe** (`dyn Backend` works). The actual I/O happens in the
//! implementing crates.
//!
//! ## Extensibility for Multiple Backends
//!
//! Every type is designed to work with both Postgres and SQLite from day one:
//! - [`Dialect`] captures capability differences as boolean flags
//! - [`SchemaCache`] types use string-based type names (not Postgres OIDs)
//! - [`Backend`] methods accept generic [`serde_json::Value`] params
//! - Optional fields (e.g. schema namespacing) gracefully degrade

pub mod backend;
pub mod cache;
pub mod config;
pub mod dialect;
pub mod error;

// Re-export primary types for ergonomic use
pub use backend::{Backend, ExecContext, IntrospectConfig, QueryResult, SchemaChangeStream};
pub use cache::{
    Cardinality, Column, ComputedRelationship, DataRepresentation, MediaHandler,
    QualifiedIdentifier, Relationship, Routine, RoutineParam, SchemaCache, Table, Volatility,
};
pub use config::Config;
pub use dialect::{Dialect, Placeholder, POSTGRES, SQLITE};
pub use error::{Error, ErrorCode};
