//! pgvis — narrate a Postgres (or SQLite) database over REST, OpenAPI, and MCP.
//!
//! This binary uses [`pgvis_lib`] as its library, ensuring the same code path
//! that end-users get when embedding pgvis in their own applications.

use clap::Parser;
use pgvis_core::Config;

/// Storyvis AI / pgvis — narrate a database over REST, OpenAPI, and MCP.
#[derive(Parser)]
#[command(name = "pgvis", version, about)]
struct Cli {
    /// Database DSN (`postgres://...` or `sqlite:///path.db`).
    #[arg(short, long, env = "PGVIS_DSN")]
    dsn: String,

    /// Path to config file (TOML). Falls back to PGVIS_* env vars.
    #[arg(short, long, env = "PGVIS_CONFIG")]
    config: Option<std::path::PathBuf>,

    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(clap::Subcommand)]
enum Cmd {
    /// Start the HTTP server (REST + optional MCP over Streamable HTTP).
    Serve {
        /// Bind address for the HTTP server.
        #[arg(short, long, default_value = "0.0.0.0:3000", env = "PGVIS_BIND")]
        bind: String,

        /// Which database schemas to expose (comma-separated or repeated).
        /// Defaults to "public".
        #[arg(short, long, env = "PGVIS_SCHEMAS", value_delimiter = ',')]
        schema: Vec<String>,

        /// Also serve MCP over Streamable HTTP at /mcp endpoint.
        #[arg(long, default_value = "false")]
        mcp_http: bool,
    },
    /// Run MCP server over stdio (for Claude Desktop / agent integrations).
    #[cfg(feature = "mcp")]
    Mcp,
    /// Print the OpenAPI 3.0 document and exit.
    Openapi,
    /// Dump the introspected schema cache as JSON.
    Inspect,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "pgvis=info,tower_http=info".into()),
        )
        .json()
        .init();

    let cli = Cli::parse();
    let mut config = load_config(cli.config.as_deref())?;

    match cli.cmd.unwrap_or(Cmd::Serve {
        bind: "0.0.0.0:3000".into(),
        schema: vec![],
        mcp_http: false,
    }) {
        Cmd::Serve {
            bind,
            schema,
            mcp_http,
        } => {
            // Override schemas from CLI if provided
            if !schema.is_empty() {
                config.schemas = schema;
            }

            tracing::info!(dsn = %cli.dsn, bind = %bind, schemas = ?config.schemas, mcp_http, "starting pgvis server");

            let mut builder = pgvis_lib::Builder::new(&cli.dsn).config(config);

            #[cfg(feature = "mcp")]
            if mcp_http {
                builder = builder.with_mcp_http();
            }

            let components = builder.build_components().await?;

            let listener = tokio::net::TcpListener::bind(&bind).await?;
            tracing::info!("listening on {bind}");
            axum::serve(listener, components.router).await?;
        }

        #[cfg(feature = "mcp")]
        Cmd::Mcp => {
            tracing::info!(dsn = %cli.dsn, "starting pgvis MCP server (stdio)");

            let mcp_server = pgvis_lib::Builder::new(&cli.dsn)
                .config(config)
                .build_mcp_server()
                .await?;

            pgvis_lib::pgvis_mcp::serve_stdio(mcp_server)
                .await
                .map_err(|e| anyhow::anyhow!(e))?;
        }

        Cmd::Openapi => {
            let components = pgvis_lib::Builder::new(&cli.dsn)
                .config(config)
                .build_components()
                .await?;

            let cache = components.cache.load();
            let spec = pgvis_lib::pgvis_router::openapi::generate_spec(&cache, &components.config);
            let json = serde_json::to_string_pretty(&spec)?;
            println!("{json}");
        }

        Cmd::Inspect => {
            let components = pgvis_lib::Builder::new(&cli.dsn)
                .config(config)
                .build_components()
                .await?;

            let cache = components.cache.load();
            let json = serde_json::to_string_pretty(&*cache)?;
            println!("{json}");
        }
    }

    Ok(())
}

/// Load configuration from file and/or environment variables.
///
/// Configuration is layered (later sources override earlier):
/// 1. Defaults from [`Config::default()`]
/// 2. TOML config file (if `--config` flag or `PGVIS_CONFIG` env var is set)
/// 3. Environment variables prefixed with `PGVIS_`
fn load_config(path: Option<&std::path::Path>) -> anyhow::Result<Config> {
    use figment::Figment;
    use figment::providers::{Env, Format, Serialized, Toml};

    let mut figment = Figment::from(Serialized::defaults(Config::default()));

    // Layer 2: TOML config file (if provided)
    if let Some(path) = path {
        figment = figment.merge(Toml::file(path));
    }

    // Layer 3: Environment variables (PGVIS_SCHEMAS, PGVIS_JWT_SECRET, etc.)
    // Uses lowercase field names with underscore splitting:
    // PGVIS_JWT_SECRET → jwt_secret, PGVIS_MAX_ROWS → max_rows
    figment = figment.merge(Env::prefixed("PGVIS_").lowercase(true));

    let config: Config = figment.extract()?;
    Ok(config)
}
