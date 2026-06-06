//! Beacon binary entry — wires env loading, tracing, state, schedulers,
//! and the axum server. Handles SIGINT / SIGTERM for graceful shutdown.

use std::net::SocketAddr;

use beacon_api::{app, dotenv, scheduler, state::AppState, tracing_setup};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenv::dotenv();
    tracing_setup::init();

    // One-off migration mode for the blue/green pre-deploy task:
    //   `beacon migrate` connects, applies pending migrations, and exits.
    // The long-running service tasks then boot with MIGRATE_ON_BOOT=verify so
    // two task sets never race to run DDL during a cutover.
    if std::env::args().nth(1).as_deref() == Some("migrate") {
        let database_url = std::env::var("DATABASE_URL")
            .map_err(|_| anyhow::anyhow!("DATABASE_URL must be set"))?;
        let pool =
            beacon_db::connect_with_retry(&database_url, std::time::Duration::from_secs(30)).await?;
        let n = beacon_db::migrate(&pool).await?;
        tracing::info!(applied = n, "migration task complete");
        return Ok(());
    }

    let bind: SocketAddr = std::env::var("BEACON_BIND")
        .unwrap_or_else(|_| "127.0.0.1:8080".into())
        .parse()?;
    tracing::info!(%bind, "Beacon starting");

    let state = AppState::from_env().await?;
    scheduler::spawn_all(state.clone());

    let app = app::build_router(state);
    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!(%bind, "Beacon listening");

    // `into_make_service_with_connect_info` makes the client SocketAddr
    // available to the rate-limit middleware.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    tracing::info!("Beacon shutting down");
    Ok(())
}

/// Resolve when either SIGINT (Ctrl-C) or SIGTERM is received.
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl-C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => { tracing::info!("Ctrl-C received"); },
        _ = terminate => { tracing::info!("SIGTERM received"); },
    }
}
