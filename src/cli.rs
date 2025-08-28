use std::net::SocketAddr;
use std::str::FromStr;

use clap::Parser;
use color_eyre::eyre::{eyre, Result};
use tendermint_light_client::types::{Hash, Height, TrustThreshold};
use tendermint_rpc::HttpClientUrl;
use tracing::metadata::LevelFilter;

pub fn parse_trust_threshold(s: &str) -> Result<TrustThreshold> {
    if let Some((l, r)) = s.split_once('/') {
        TrustThreshold::new(l.parse()?, r.parse()?).map_err(Into::into)
    } else {
        Err(eyre!(
            "invalid trust threshold: {s}, format must be X/Y where X and Y are integers"
        ))
    }
}

#[derive(Clone, Debug)]
pub struct List<T>(pub Vec<T>);

impl<E, T: FromStr<Err = E>> FromStr for List<T> {
    type Err = E;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.split(',')
            .map(|s| s.parse())
            .collect::<Result<Vec<_>, _>>()
            .map(Self)
    }
}

#[derive(clap::Args, Debug, Clone)]
pub struct Verbosity {
    /// Increase verbosity, can be repeated up to 2 times
    #[arg(long, short, action = clap::ArgAction::Count)]
    verbose: u8,
}

impl Verbosity {
    pub fn to_level_filter(&self) -> LevelFilter {
        match self.verbose {
            0 => LevelFilter::INFO,
            1 => LevelFilter::DEBUG,
            _ => LevelFilter::TRACE,
        }
    }
}

#[derive(Debug, Parser, Clone)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// The address to bind the RPC server to
    #[arg(long, default_value = "127.0.0.1:8080")]
    pub listen_addr: SocketAddr,

    /// Identifier of the chain
    #[arg(long)]
    pub chain_id: String,

    /// Primary RPC address
    #[arg(long)]
    pub primary: HttpClientUrl,

    /// Comma-separated list of witnesses RPC addresses
    #[arg(long)]
    pub witnesses: List<HttpClientUrl>,

    /// Height of trusted header
    #[arg(long)]
    pub trusted_height: Height,

    /// Hash of trusted header
    #[arg(long)]
    pub trusted_hash: Hash,

    /// Trust threshold
    #[arg(long, value_parser = parse_trust_threshold, default_value_t = TrustThreshold::TWO_THIRDS)]
    pub trust_threshold: TrustThreshold,

    /// Trusting period, in seconds (default: two weeks)
    #[arg(long, default_value = "1209600")]
    pub trusting_period: u64,

    /// Maximum clock drift, in seconds
    #[arg(long, default_value = "5")]
    pub max_clock_drift: u64,

    /// Maximum block lag, in seconds
    #[arg(long, default_value = "5")]
    pub max_block_lag: u64,

    /// The maximum age of the trusted state before a new sync is triggered by an API request (in seconds)
    #[arg(long, default_value = "10")]
    pub freshness_threshold: u64,

    /// The interval for the periodic 'keep-warm' syncs when the server is idle (in seconds) (default: 5 minutes)
    #[arg(long, default_value = "300")]
    pub keep_warm_interval: u64,

    /// The duration to halt syncing for after a fork is detected (in seconds) (default: 60 minutes)
    #[arg(long, default_value = "3600")]
    pub halt_duration_on_fork: u64,

    /// Timeout for API-triggered waits (in seconds)
    #[arg(long, default_value = "5")]
    pub api_timeout: u64,

    /// Increase verbosity
    #[command(flatten)]
    pub verbose: Verbosity,
}
