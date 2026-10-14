//! `teresad` — the TERESA server binary.
//!
//! A thin shell: parse arguments, load configuration, assemble the
//! application, serve until told to stop. Everything with behaviour worth
//! testing lives in the library.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use axum_server::tls_rustls::RustlsConfig;
use axum_server::Handle;
use clap::{Parser, Subcommand};
use teresa_server::repo::sqlite::SqliteStore;
use teresa_server::services::Core;
use teresa_server::state::AppState;
use teresa_server::{api, ServerConfig};
use tracing_subscriber::EnvFilter;

/// How long a graceful shutdown waits for connections to drain.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);
/// Cadence of the housekeeping sweep.
const HOUSEKEEPING_INTERVAL: Duration = Duration::from_secs(60);

/// The TERESA server.
#[derive(Debug, Parser)]
#[command(name = "teresad", version, about)]
struct Args {
    /// Path to server.toml. Defaults apply if omitted.
    #[arg(short, long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Make an existing account an admin (works while the server runs).
    Promote {
        /// The account's username.
        username: String,
    },
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let config = ServerConfig::load(args.config.as_deref())?;

    // RUST_LOG wins over the file, because the person typing it is closer to
    // the problem than the person who wrote the config.
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(config.log.clone()));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    // One CSPRNG/TLS backend for the whole process, chosen explicitly rather
    // than by whichever dependency initialises first.
    rustls::crypto::ring::default_provider()
        .install_default()
        .ok();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("tokio runtime failed to start")?;
    match args.command {
        Some(Command::Promote { username }) => runtime.block_on(promote(config, &username)),
        None => runtime.block_on(serve(config)),
    }
}

/// `teresad promote <user>`: the operator's door into the admin role.
async fn promote(config: ServerConfig, username: &str) -> anyhow::Result<()> {
    let store = SqliteStore::open(&config.database)
        .await
        .with_context(|| format!("opening database {}", config.database.display()))?;
    let user = Core::new(store, config)
        .promote(username)
        .await
        .with_context(|| format!("promoting {username}"))?;
    println!("✦ {} (@{}) is now an admin", user.display_name, user.username);
    Ok(())
}

async fn serve(config: ServerConfig) -> anyhow::Result<()> {
    banner(&config);

    if let Some(parent) = config
        .database
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating database directory {}", parent.display()))?;
    }

    let store = SqliteStore::open(&config.database)
        .await
        .with_context(|| format!("opening database {}", config.database.display()))?;

    let tls = match (&config.tls.cert, &config.tls.key) {
        (Some(cert), Some(key)) => Some(
            RustlsConfig::from_pem_file(cert, key)
                .await
                .context("loading TLS certificate and key")?,
        ),
        _ => None,
    };

    let core = Core::new(store, config);
    core.bootstrap().await.context("seeding initial rooms")?;
    let state = AppState::new(core);

    // Housekeeping heartbeat: session expiry, channel pruning, limiter decay.
    let housekeeper = {
        let state = state.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(HOUSEKEEPING_INTERVAL);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                state.housekeeping().await;
            }
        })
    };

    // Ctrl-C → warn the rooms, then drain.
    let handle = Handle::new();
    {
        let handle = handle.clone();
        let state = state.clone();
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                tracing::info!("shutdown signal received");
                state
                    .core
                    .hub()
                    .publish_global(teresa_proto::ServerFrame::Notice {
                        text: "server is shutting down".to_owned(),
                    });
                handle.graceful_shutdown(Some(SHUTDOWN_GRACE));
            }
        });
    }

    let addr = state.config().addr();
    let app = api::router(state.clone()).into_make_service_with_connect_info::<SocketAddr>();

    match tls {
        Some(tls) => {
            tracing::info!(%addr, tls = true, "listening");
            axum_server::bind_rustls(addr, tls)
                .handle(handle)
                .serve(app)
                .await?;
        }
        None => {
            tracing::info!(%addr, tls = false, "listening");
            axum_server::bind(addr).handle(handle).serve(app).await?;
        }
    }

    housekeeper.abort();
    tracing::info!("goodbye");
    Ok(())
}

/// A little ceremony on startup; operators deserve some atmosphere too.
fn banner(config: &ServerConfig) {
    eprintln!(
        "\n  ✦ TERESA server v{}\n  ├ node: {}\n  ├ db:   {}\n  ╰ no system is safe\n",
        teresa_server::VERSION,
        config.name,
        config.database.display(),
    );
}
