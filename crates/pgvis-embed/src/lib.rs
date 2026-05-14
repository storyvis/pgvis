//! `pgvis-embed` — One-liner to get an axum Router from a database DSN.
//!
//! ```rust,no_run
//! use pgvis_embed::Builder;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let router = Builder::new("postgres://localhost/mydb")
//!     .schemas(vec!["public"])
//!     .build()
//!     .await?;
//! # Ok(())
//! # }
//! ```

pub struct Builder {
    dsn: String,
    schemas: Vec<String>,
}

impl Builder {
    /// Create a new builder with the given DSN.
    pub fn new(dsn: impl Into<String>) -> Self {
        Self {
            dsn: dsn.into(),
            schemas: vec!["public".to_string()],
        }
    }

    /// Set which schemas to expose.
    pub fn schemas(mut self, schemas: Vec<impl Into<String>>) -> Self {
        self.schemas = schemas.into_iter().map(Into::into).collect();
        self
    }

    /// Build the axum Router by connecting to the database and introspecting the schema.
    ///
    /// # Errors
    /// Returns an error if the database connection or introspection fails.
    pub async fn build(self) -> Result<axum::Router, pgvis_core::error::Error> {
        let _backend = pgvis_postgres::PgBackend::new(&self.dsn)?;

        // TODO: introspect + build_app (Phase 8 of implementation plan)
        Ok(axum::Router::new())
    }
}
