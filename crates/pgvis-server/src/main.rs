//! pgvis — narrate a Postgres (or SQLite) database over REST, OpenAPI, and MCP.

use clap::Parser;

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
    /// Start the HTTP server (default).
    Serve,
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

    match cli.cmd.unwrap_or(Cmd::Serve) {
        Cmd::Serve => {
            tracing::info!(dsn = %cli.dsn, "starting pgvis server");
            // TODO: Phase 7 — full serve implementation
            println!("pgvis serve not yet implemented — scaffolding only");
        }
        Cmd::Openapi => {
            println!("pgvis openapi not yet implemented — scaffolding only");
        }
        Cmd::Inspect => {
            println!("pgvis inspect not yet implemented — scaffolding only");
        }
    }

    Ok(())
}
