#![allow(unused)]

use std::sync::Arc;
use std::time::Instant;

use axum::{routing::get, Router};
use clap::Parser;
use color_eyre::eyre::Result;
use tokio::sync::{broadcast, watch};
use tower_http::cors::{Any, CorsLayer};
use tracing::info;
use tracing_subscriber::{util::SubscriberInitExt, EnvFilter};

mod api;
mod cli;
mod state;
mod sync;

use crate::{
    api::{root_handler, status_handler},
    cli::Args,
    state::{AppState, Config, SharedState},
    sync::run_sync,
};

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;

    let args = Args::parse();

    let env_filter = EnvFilter::builder()
        .with_default_directive(args.verbose.to_level_filter().into())
        .from_env_lossy();

    tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter(env_filter)
        .finish()
        .init();

    info!("Starting daemon...");
    run_server(args).await
}

async fn run_server(args: Args) -> Result<()> {
    let state = Arc::new(tokio::sync::RwLock::new(AppState {
        config: Config {
            freshness_threshold: std::time::Duration::from_secs(args.freshness_threshold),
            keep_warm_interval: std::time::Duration::from_secs(args.keep_warm_interval),
            halt_duration_on_fork: std::time::Duration::from_secs(args.halt_duration_on_fork),
            api_timeout: std::time::Duration::from_secs(args.api_timeout),
        },
        light_block: None,
        last_sync: Instant::now(),
        syncing: true,
        last_sync_success: false,
    }));

    let (sync_trigger_tx, sync_trigger_rx) = broadcast::channel(1);
    let (sync_done_tx, sync_done_rx) = watch::channel(());

    // Spawn the background syncing task
    let sync_task_state = state.clone();
    let sync_task_args = args.clone();
    tokio::spawn(async move {
        run_sync(
            sync_task_args,
            sync_task_state,
            sync_trigger_rx,
            sync_done_tx,
        )
        .await;
    });

    // Create the Axum app
    let sync_trigger_tx_for_state = sync_trigger_tx.clone();
    let app = Router::new()
        .route("/", get(root_handler))
        .route("/v1/status", get(status_handler))
        .with_state((state, sync_trigger_tx_for_state, sync_done_rx))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        );

    info!("Listening on http://{}", args.listen_addr);
    let listener = tokio::net::TcpListener::bind(args.listen_addr).await?;
    // Trigger initial sync immediately on startup
    let _ = sync_trigger_tx.send(());
    axum::serve(listener, app).await?;

    Ok(())
}
