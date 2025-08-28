# Aliases
alias rl := run-local
alias rt := run-testnet
alias ul := update-trusted-local
alias ut := update-trusted-testnet

# List all available commands
default:
    @just --list

# Run the client against a local node
run-local *extra:
    @just --dotenv-path .env.local --justfile {{justfile()}} \
        _run \
        {{extra}}

# Run the client against the public testnet
run-testnet *extra:
    @just --dotenv-path .env.testnet --justfile {{justfile()}} \
        _run \
        {{extra}}

# ---
# Helper commands for quick debugging
# WARNING: These are for development convenience only. They fetch information
# from a single, untrusted RPC endpoint. In a production context, you MUST
# obtain the trusted height and hash from a reliable source subject to social
# consensus (e.g., governance, official project announcements, etc.)
# ---

# Update trusted values from local RPC's latest block
update-trusted-local:
    @just --dotenv-path .env.local --justfile {{justfile()}} _update-trusted .env.local

# Update trusted values from testnet RPC's latest block
update-trusted-testnet:
    @just --dotenv-path .env.testnet --justfile {{justfile()}} _update-trusted .env.testnet

# ---
# Internal recipes
# ---

_run *extra:
    #!/usr/bin/env bash
    set -euo pipefail
    set -- {{extra}}
    args=(cargo run --)
    if [[ -n "${CHAIN_ID:-}" ]]; then args+=(--chain-id "$CHAIN_ID"); fi
    if [[ -n "${PRIMARY:-}" ]]; then args+=(--primary "$PRIMARY"); fi
    if [[ -n "${WITNESSES:-}" ]]; then args+=(--witnesses "$WITNESSES"); fi
    if [[ -n "${TRUSTED_HEIGHT:-}" ]]; then args+=(--trusted-height "$TRUSTED_HEIGHT"); fi
    if [[ -n "${TRUSTED_HASH:-}" ]]; then args+=(--trusted-hash "$TRUSTED_HASH"); fi
    if [[ -n "${LISTEN_ADDR:-}" ]]; then args+=(--listen-addr "$LISTEN_ADDR"); fi
    if [[ -n "${TRUST_THRESHOLD:-}" ]]; then args+=(--trust-threshold "$TRUST_THRESHOLD"); fi
    if [[ -n "${TRUSTING_PERIOD:-}" ]]; then args+=(--trusting-period "$TRUSTING_PERIOD"); fi
    if [[ -n "${MAX_CLOCK_DRIFT:-}" ]]; then args+=(--max-clock-drift "$MAX_CLOCK_DRIFT"); fi
    if [[ -n "${MAX_BLOCK_LAG:-}" ]]; then args+=(--max-block-lag "$MAX_BLOCK_LAG"); fi
    if [[ -n "${FRESHNESS_THRESHOLD:-}" ]]; then args+=(--freshness-threshold "$FRESHNESS_THRESHOLD"); fi
    if [[ -n "${KEEP_WARM_INTERVAL:-}" ]]; then args+=(--keep-warm-interval "$KEEP_WARM_INTERVAL"); fi
    if [[ -n "${HALT_DURATION_ON_FORK:-}" ]]; then args+=(--halt-duration-on-fork "$HALT_DURATION_ON_FORK"); fi
    if [[ -n "${VERBOSE:-}" ]]; then args+=(-"$VERBOSE"); fi
    if [[ $# -gt 0 ]]; then args+=("$@"); fi
    exec "${args[@]}"

_update-trusted env_file:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ ! -f "{{env_file}}" ]; then
        echo "Missing env file: {{env_file}}" >&2
        exit 1
    fi

    latest=$(curl -s "{{env('PRIMARY')}}/status" | jq -r '.result.sync_info.latest_block_height')
    if ! [[ "$latest" =~ ^[0-9]+$ ]]; then
        echo "Invalid latest height from {{env('PRIMARY')}}: $latest" >&2
        exit 1
    fi
    if [ "$latest" -le 1 ]; then
        echo "Latest height too low ($latest) to set H-1" >&2
        exit 1
    fi
    height=$((latest - 1))
    commit_json=$(curl -s "{{env('PRIMARY')}}/commit?height=$height")
    hash=$(printf "%s" "$commit_json" | jq -r '.result.signed_header.commit.block_id.hash')
    if [ -z "$hash" ] || [ "$hash" = "null" ]; then
        echo "Failed to fetch hash for height $height from {{env('PRIMARY')}}" >&2
        exit 1
    fi
    
    if ! grep -q '^TRUSTED_HEIGHT=' "{{env_file}}"; then
        echo "Missing TRUSTED_HEIGHT key in {{env_file}}" >&2
        exit 1
    fi
    if ! grep -q '^TRUSTED_HASH=' "{{env_file}}"; then
        echo "Missing TRUSTED_HASH key in {{env_file}}" >&2
        exit 1
    fi
    sed -i "s/^TRUSTED_HEIGHT=.*/TRUSTED_HEIGHT=${height}/" "{{env_file}}"
    sed -i "s/^TRUSTED_HASH=.*/TRUSTED_HASH=${hash}/" "{{env_file}}"
    echo "Updated {{env_file}} with TRUSTED_HEIGHT=${height} and TRUSTED_HASH=${hash}"
