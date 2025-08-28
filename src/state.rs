use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use tendermint::Time;
use tendermint_light_client::types::{Hash, Height, LightBlock};
use tokio::sync::RwLock;

#[derive(Debug, Serialize, Clone)]
pub struct StatusResponse {
    pub block_height: Height,
    pub block_hash: Hash,
    pub block_timestamp: Time,
}

#[derive(Debug, Serialize, Clone)]
pub struct RootResponse {
    pub name: &'static str,
    pub version: &'static str,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub freshness_threshold: Duration,
    pub keep_warm_interval: Duration,
    pub halt_duration_on_fork: Duration,
    pub api_timeout: Duration,
}

pub struct AppState {
    pub config: Config,
    pub light_block: Option<LightBlock>,
    pub last_sync: Instant,
    pub syncing: bool,
    pub last_sync_success: bool,
}

pub type SharedState = Arc<RwLock<AppState>>;
