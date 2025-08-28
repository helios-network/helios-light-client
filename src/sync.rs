use std::time::{Duration, Instant};

use color_eyre::eyre::{eyre, Result};
use futures::future::join_all;
use reqwest::Client as ReqwestClient;
use tendermint::crypto::default::Sha256;
use tendermint::evidence::Evidence;
use tendermint::Time;
use tendermint_light_client::{
    builder::LightClientBuilder,
    instance::Instance,
    light_client::Options,
    store::memory::MemoryStore,
    types::{Hash, Height, LightBlock},
};
use tendermint_light_client_detector::{detect_divergence, Error as DetectorError, Provider, Trace};
use tendermint_rpc::{client::CompatMode, Client, HttpClient, HttpClientUrl};
use tokio::sync::{broadcast, watch};
use tracing::{debug, error, info, warn};

use crate::{
    cli::Args,
    state::{AppState, SharedState},
};

fn fmt_peer_url<T: std::fmt::Display>(peer_id: T, url: &HttpClientUrl) -> String {
    format!(
        "peer: {}, url: {}",
        peer_id,
        tendermint_rpc::Url::from(url.clone()).to_string()
    )
}

pub async fn run_sync(
    args: Args,
    state: SharedState,
    mut sync_trigger_rx: broadcast::Receiver<()>,
    sync_done_tx: watch::Sender<()>,
) {
    let options = Options {
        trust_threshold: args.trust_threshold,
        trusting_period: Duration::from_secs(args.trusting_period),
        clock_drift: Duration::from_secs(args.max_clock_drift),
    };

    let mut primary = match make_provider(
        &args.chain_id,
        args.primary.clone(),
        args.trusted_height,
        args.trusted_hash,
        options,
    )
    .await
    {
        Ok(provider) => provider,
        Err(e) => {
            error!("failed to initialize primary provider: {}", e);
            return;
        }
    };

    let trusted_block = match primary.latest_trusted() {
        Some(block) => block,
        None => {
            error!("failed to get initial trusted block from primary ({})", fmt_peer_url(primary.peer_id(), &args.primary));
            return;
        }
    };

    let witnesses = join_all(args.witnesses.0.iter().map(|addr| {
        make_provider(
            &args.chain_id,
            addr.clone(),
            trusted_block.height(),
            trusted_block.signed_header.header.hash(),
            options,
        )
    }))
    .await;

    let mut witnesses: Vec<Provider> = match witnesses.into_iter().collect() {
        Ok(witnesses) => witnesses,
        Err(e) => {
            error!("failed to initialize one or more witnesses: {}", e);
            return;
        }
    };

    info!(
        "Initialized primary provider ({})",
        fmt_peer_url(primary.peer_id(), &args.primary),
    );
    for (i, witness) in witnesses.iter().enumerate() {
        info!(
            "Initialized witness provider #{} ({})",
            i + 1,
            fmt_peer_url(witness.peer_id(), &args.witnesses.0[i]),
        );
    }

    let keep_warm_interval = Duration::from_secs(args.keep_warm_interval);
    let mut keep_warm_timer = tokio::time::interval(keep_warm_interval);
    let mut backoff_secs: u64 = 1;
    let max_backoff_secs: u64 = 30;

    loop {
        tokio::select! {
            _ = keep_warm_timer.tick() => {
                debug!("sync triggered by periodic timer");
            },
            Ok(_) = sync_trigger_rx.recv() => {
                debug!("sync triggered by API request");
            }
        }

        info!("Syncing from primary...");
        match primary.verify_to_highest() {
            Ok(new_block) => {
                info!(
                    "Sync successful to block height {}",
                    new_block.height()
                );

                // Fork detection starts here
                let primary_trace = primary.get_trace(new_block.height());
                let fork_detected = run_fork_detector(
                    &mut primary,
                    &mut witnesses,
                    primary_trace,
                    &args,
                )
                .await;

                if !fork_detected {
                    // Happy path: no fork, update state
                    let mut lock = state.write().await;
                    lock.light_block = Some(new_block);
                    lock.last_sync = Instant::now();
                    lock.last_sync_success = true;
                    backoff_secs = 1; // reset backoff on success
                } else {
                    // Fork detected, enter halted state
                    warn!(
                        "Fork detected! Halting all sync operations for {} seconds.",
                        args.halt_duration_on_fork
                    );
                    tokio::time::sleep(Duration::from_secs(args.halt_duration_on_fork)).await;
                }
            }
            Err(e) => {
                error!("failed to verify to highest on primary ({}): {}", fmt_peer_url(primary.peer_id(), &args.primary), e);
                // mark failure and back off
                {
                    let mut lock = state.write().await;
                    lock.last_sync_success = false;
                }
                tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
                backoff_secs = (backoff_secs * 2).min(max_backoff_secs);
            }
        }

        // Reset the syncing flag and notify any waiting handlers
        {
            let mut lock = state.write().await;
            lock.syncing = false;
        }
        let _ = sync_done_tx.send(());
    }
}

async fn run_fork_detector(
    primary: &mut Provider,
    witnesses: &mut [Provider],
    primary_trace: Vec<LightBlock>,
    args: &Args,
) -> bool {
    if witnesses.is_empty() {
        info!("No witnesses provided, skipping fork detection");
        return false;
    }

    info!(
        "Performing fork detection with {} witnesses against primary",
        witnesses.len(),
    );

    let primary_trace = match Trace::new(primary_trace) {
        Ok(trace) => trace,
        Err(e) => {
            error!("failed to construct trace from primary ({}) light blocks: {}", fmt_peer_url(primary.peer_id(), &args.primary), e);
            return false; // Cannot perform detection without a valid trace
        }
    };

    let last_verified_height = primary_trace.last().height();
    let max_clock_drift = Duration::from_secs(args.max_clock_drift);
    let max_block_lag = Duration::from_secs(args.max_block_lag);
    let mut fork_detected = false;

    for (i, witness) in witnesses.iter_mut().enumerate() {
        let divergence = detect_divergence::<Sha256>(
            Some(primary),
            witness,
            primary_trace.clone().into_vec(),
            max_clock_drift,
            max_block_lag,
        )
        .await;

        let evidence = match divergence {
            Ok(Some(divergence)) => {
                error!(
                    "fork detected: primary ({}) presented a conflicting header vs witness ({}) at block height {}",
                    fmt_peer_url(primary.peer_id(), &args.primary),
                    fmt_peer_url(witness.peer_id(), &args.witnesses.0[i]),
                    divergence.evidence.against_primary.conflicting_block.signed_header.header.height
                );
                fork_detected = true;
                divergence.evidence
            }
            Ok(None) => {
                debug!(
                    "no divergence found between primary ({}) and witness ({}) at block height {}",
                    fmt_peer_url(primary.peer_id(), &args.primary),
                    fmt_peer_url(witness.peer_id(), &args.witnesses.0[i]),
                    last_verified_height,
                );
                continue;
            }
            Err(e) => {
                error!(
                    "failed to run attack detector against witness ({}): {}",
                    fmt_peer_url(witness.peer_id(), &args.witnesses.0[i]),
                    e
                );
                continue; // An error is not a fork, but we should not trust this witness for this round
            }
        };

        // Report the evidence
        if let Err(e) = witness
            .report_evidence(Evidence::from(evidence.against_primary))
            .await
        {
            error!(
                "failed to report evidence to witness ({}): {}",
                fmt_peer_url(witness.peer_id(), &args.witnesses.0[i]),
                e
            );
        }

        if let Some(against_witness) = evidence.against_witness {
            if let Err(e) = primary
                .report_evidence(Evidence::from(against_witness))
                .await
            {
                error!(
                    "failed to report evidence to primary ({}): {}",
                    fmt_peer_url(primary.peer_id(), &args.primary),
                    e
                );
            }
        }
    }

    if !fork_detected {
        info!(
            "No divergence found between primary and {} witnesses at block height {}",
            witnesses.len(),
            last_verified_height,
        );
    }

    fork_detected
}


async fn make_provider(
    chain_id: &str,
    rpc_addr: HttpClientUrl,
    trusted_height: Height,
    trusted_hash: Hash,
    options: Options,
) -> Result<Provider> {
    // Build a custom reqwest client with connection pooling disabled.
    let custom_reqwest_client = ReqwestClient::builder()
        .pool_max_idle_per_host(0) // Disables Keep-Alive by not pooling idle connections
        .build()?;

    // Build the tendermint HttpClient, passing in our custom reqwest client.
    let rpc_client = HttpClient::builder(rpc_addr)
        .compat_mode(CompatMode::V0_37)
        .client(custom_reqwest_client)
        .build()?;

    let node_id = rpc_client.status().await?.node_info.id;
    let light_store = Box::new(MemoryStore::new());

    let instance =
        LightClientBuilder::prod(node_id, rpc_client.clone(), light_store, options, None)
            .trust_primary_at(trusted_height, trusted_hash)?
            .build();

    Ok(Provider::new(chain_id.to_string(), instance, rpc_client))
}
