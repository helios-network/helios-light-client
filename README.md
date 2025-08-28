# Helios Light Client

**Helios Light Client** is a long-running daemon that maintains a cryptographically verified view of a CometBFT/Tendermint blockchain and exposes it over a simple HTTP API for other services ("consumers") to use as a trust anchor.

It continuously syncs headers from a primary RPC provider, verifies them with the [Tendermint light client protocol](https://arxiv.org/pdf/2010.07031), performs fork detection against optional witness RPC providers, and serves the latest trusted block information via GET `/v1/status`.

## What it is and how it works

`helios-light-client` runs as a background service that other applications can query to obtain a fresh, verified blockchain state without implementing consensus verification themselves.

- On startup, it is bootstrapped with a **trusted checkpoint**: `--trusted-height` (H) and `--trusted-hash` corresponding to the signed header at height H for the configured `--chain-id`.
- It connects to a primary RPC endpoint (`--primary`) and optional witness endpoints (`--witnesses`) over HTTP.
- Using the Tendermint light client algorithm, it verifies forward to the highest available height from the checkpoint, honoring the configured safety parameters:
  - Trust threshold: `--trust-threshold` (default: 2/3) defines the minimum voting power fraction required to trust a validator set change.
  - Trusting period: `--trusting-period` limits how long a trusted header remains valid with respect to potential validator set changes.
  - Maximum clock drift and block lag: `--max-clock-drift`, `--max-block-lag` constrain acceptable time and progress discrepancies during verification and fork detection.
- For ongoing operation, it periodically attempts to advance to the latest header and also supports **on-demand refresh**: a request to `/v1/status` triggers a sync if the last successful sync is older than the configured `--freshness-threshold`. If a sync is already in progress, the handler briefly waits for completion (bounded by `--api-timeout`).
- Fork detection: after advancing, it compares the primary's trace of light blocks against each witness using a divergence detector. If conflicting headers are found, it reports evidence to peers and enters a protective halted state for `--halt-duration-on-fork`, avoiding serving potentially divergent updates.

The `/v1/status` response returns the latest trusted light block metadata:

```json
{
  "block_height": "<height>",
  "block_hash": "<hash>",
  "block_timestamp": "<rfc3339 timestamp>"
}
```

With a trusted pair of blocks at heights h and H from `helios-light-client`, a consumer can safely query any untrusted RPC endpoint for application data accompanied by ICS‑23 Merkle proofs and verify those proofs against the trusted header(s) it obtained. This decouples consensus security from data access, letting consumers treat the network and intermediate RPCs as untrusted transport.

`helios-light-client` is designed to run inside a TEE (Trusted Execution Environment) within an internal network. In that setup, the daemon and its key verification logic execute in an attested environment, so consumers can place trust in the attested binary rather than the surrounding infrastructure. *When deployed outside a TEE and exposed over HTTP, downstream consumers implicitly trust the light client service itself; a TEE deployment reduces this trust surface by ensuring the exact audited code is what executes, while network transport may remain untrusted.*

## Technical stack

The daemon is implemented in Rust on top of the Tokio async runtime, using Axum for the HTTP API. It builds on the official `tendermint-rs` ecosystem: `tendermint-light-client` for header verification, `tendermint-rpc` for node communication, and `tendermint-light-client-detector` for fork/divergence detection. The original single-run `tendermint-light-client-cli` flow is adapted and daemonized here: verification runs continuously in a background task, maintains an in-memory trusted state, and exposes a narrow, stable HTTP surface (`/` and `/v1/status`). The service is containerized with a multi-stage Docker build that produces a small, static MUSL binary image and is intended to run under `docker-compose`, allowing other services on the same network to consume the trust anchor reliably.

## Build and Run

### Prerequisites
- Rust toolchain (1.88+) and Cargo
- Docker (for containerized builds)

### Local Build
```bash
cargo build --release
```

### Run (binary)
```bash
# Example; the exact flags will depend on your environment.
# See the Usage section for the full list of flags.
./target/release/helios-light-client \
  --listen-addr 0.0.0.0:8080 \
  --chain-id <CHAIN_ID> \
  --primary <PRIMARY_RPC_URL> \
  --witnesses <W1,W2,...> \
  --trusted-height <H> \
  --trusted-hash <HASH>
```

### Docker Build
```bash
docker build -t helios-light-client:latest .
```

### Docker Run
```bash
docker run --rm -p 8080:8080 \
  helios-light-client:latest
```

### Docker Compose (example)

An example `docker-compose.yml` is provided to demonstrate integration: it runs `helios-light-client` and a minimal `consumer-app` that periodically queries `GET /v1/status`. Use it as a starting point to craft your own setup. *For quick dev/test, `compose_from_env.sh` lets you launch the stack using values from your `.env.local` or `.env.testnet` files, mapping environment variables to the CLI flags described in Usage.*

## Usage

### CLI flags

| Flag | Description | Type | Default | Required |
| --- | --- | --- | --- | --- |
| `--listen-addr` | Address to bind the HTTP API server | `SocketAddr` (`host:port`) | `127.0.0.1:8080` | Optional |
| `--chain-id` | Identifier of the target chain | `String` | — | Required |
| `--primary` | Primary RPC endpoint used for verification and syncing | `URL` | — | Required |
| `--witnesses` | Comma-separated list of witness RPC endpoints for fork detection | `List<URL>` | — | Required |
| `--trusted-height` | Height of the trusted checkpoint header (H) | `Height` (integer) | — | Required |
| `--trusted-hash` | Hash of the trusted checkpoint header at height H | `Hash` (hex) | — | Required |
| `--trust-threshold` | Minimum voting power fraction required for validator set changes | `TrustThreshold` (`X/Y`) | `2/3` | Optional |
| `--trusting-period` | Duration a trusted header remains valid | `u64` (seconds) | `1209600` (2 weeks) | Optional |
| `--max-clock-drift` | Allowed clock skew during verification/detection | `u64` (seconds) | `5` | Optional |
| `--max-block-lag` | Max allowed block lag between peers in detection | `u64` (seconds) | `5` | Optional |
| `--freshness-threshold` | Max age of the last successful sync before an API call triggers a refresh | `u64` (seconds) | `10` | Optional |
| `--keep-warm-interval` | Periodic background sync interval when idle | `u64` (seconds) | `300` | Optional |
| `--halt-duration-on-fork` | Time to halt after fork detection before resuming | `u64` (seconds) | `3600` | Optional |
| `--api-timeout` | Max time the API waits for an on-demand sync to complete | `u64` (seconds) | `5` | Optional |
| `-v, --verbose` | Increase log verbosity (repeat up to 2 times) | `count` (`0..2`) | `0` | Optional |

**Notes:**
- `--witnesses` can be provided as an empty list to effectively disable fork detection; doing so is not recommended for production (see Security below).
- `--trust-threshold` expects a rational `X/Y`. The default corresponds to the canonical 2/3 threshold.

### Security considerations

- Primary and witnesses:
  - Provide multiple independent `--witnesses` operated by distinct entities. This enables effective fork/divergence detection. Running with zero or homogeneous witnesses weakens detection and should be avoided in production.
  - Prefer witnesses reachable over distinct network paths/providers.
- Trusted checkpoint (`--trusted-height`, `--trusted-hash`):
  - Obtain from a socially trusted channel (e.g., governance, official announcements). Do not derive solely from a single RPC. Keep checkpoints within the `--trusting-period` window.
- Trust parameters:
  - Keep `--trust-threshold` at `2/3` unless you have strong reasons and a thorough risk assessment. Lowering it increases the chance of accepting invalid validator set transitions.
  - Choose `--trusting-period` according to the chain's unbonding/validator change dynamics. Too long increases exposure to stale trust; too short may cause frequent re-bootstrap needs.
- Detection tolerances:
  - `--max-clock-drift` and `--max-block-lag` trade off sensitivity vs. false positives. Stricter values may flag benign conditions; looser values may delay detection. Align with observed network conditions.
- Fork handling:
  - On detection, the service halts syncing for `--halt-duration-on-fork`. Keep this non-trivial to avoid flapping while you investigate and remediate upstream.
- API exposure:
  - Bind `--listen-addr` to an internal interface or service network. The API enables status queries and on-demand syncing and is intended for internal consumption.
  - The server enables permissive CORS by default; deploy behind private networks or ingress rules, especially outside a TEE.
- Operations:
  - `--freshness-threshold` balances load vs. staleness. Lower values increase RPC pressure but reduce stale reads. Security-wise, stale trusted state may reduce the usefulness of subsequent ICS‑23 verification if too far behind.

## Development

This repository uses [Just](https://github.com/casey/just) for common developer workflows. Install `just`, then use the recipes below:

```bash
default                # List all available commands
run-local *extra       # Run the client against a local node [alias: rl]
run-testnet *extra     # Run the client against the public testnet [alias: rt]
update-trusted-local   # Update trusted values from local RPC's latest block [alias: ul]
update-trusted-testnet # Update trusted values from testnet RPC's latest block [alias: ut]
```

*The `update-trusted-*` recipes are for development convenience only; in production you must source trusted checkpoints from a socially trusted channel, not from a single RPC.*

### Environment configuration

Create environment files for your workflows and keep secrets out of the repo: use `.env.local` for local development and `.env.testnet` for the public testnet. **Bootstrap by copying the committed template** then edit the files to set the required keys (`CHAIN_ID`, `PRIMARY`, `WITNESSES`, `TRUSTED_HEIGHT`, `TRUSTED_HASH`) and, if desired, any optional keys documented in `.env.example`.

```bash
# Bootstrap env files from the template
cp .env.example .env.local
cp .env.example .env.testnet

# Edit and set required keys: CHAIN_ID, PRIMARY, WITNESSES, TRUSTED_HEIGHT, TRUSTED_HASH
```

## License

Licensed under the Apache License, Version 2.0. See `LICENSE` for details.

### Attribution

Built on [`tendermint-rs`](https://github.com/cometbft/tendermint-rs) (Apache-2.0 by Informal Systems and contributors).
