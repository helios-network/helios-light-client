use std::time::Duration;

use axum::{
    extract::{Query, State},
    Json,
};
use tokio::sync::{broadcast, watch};
use tracing::info;

use crate::state::{RootResponse, SharedState, StatusResponse};

pub type AppStateType = (
    SharedState,
    broadcast::Sender<()>,
    watch::Receiver<()>,
);

pub async fn root_handler() -> Json<RootResponse> {
    let response = RootResponse {
        name: "helios-light-client",
        version: env!("CARGO_PKG_VERSION"),
    };
    Json(response)
}

pub async fn status_handler(
    State((state, sync_trigger, mut sync_done)): State<AppStateType>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<StatusResponse>, http::StatusCode> {
    let freshness_threshold = {
        let lock = state.read().await;
        params
            .get("freshness")
            .and_then(|s| s.parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or(lock.config.freshness_threshold)
    };

    let needs_sync = {
        let mut lock = state.write().await;
        let elapsed = lock.last_sync.elapsed();

        if elapsed > freshness_threshold {
            if !lock.syncing {
                lock.syncing = true;
                // Send a sync request, ignore error if no receivers
                let _ = sync_trigger.send(());
                true
            } else {
                info!("Sync already in progress, waiting for it to complete...");
                true // A sync is in progress, so we need to wait
            }
        } else {
            false // Data is fresh enough
        }
    };

    if needs_sync {
        // Wait for the sync to complete with timeout
        let timeout_duration = { state.read().await.config.api_timeout };
        let res = tokio::time::timeout(timeout_duration, sync_done.changed()).await;
        match res {
            Ok(Ok(_)) => {},
            Ok(Err(_)) => return Err(http::StatusCode::INTERNAL_SERVER_ERROR),
            Err(_) => return Err(http::StatusCode::GATEWAY_TIMEOUT),
        }
    }

    let lock = state.read().await;
    if let Some(light_block) = &lock.light_block {
        let response = StatusResponse {
            block_height: light_block.height(),
            block_hash: light_block.signed_header.header.hash(),
            block_timestamp: light_block.signed_header.header.time,
        };
        Ok(Json(response))
    } else {
        Err(http::StatusCode::SERVICE_UNAVAILABLE)
    }
}
