#!/usr/bin/env sh
set -eu

# Usage: ./compose_from_env.sh path/to/.env [docker compose args...]
# - Exports all variables from the provided .env into the environment
# - Runs docker compose from the repo root

if [ "${1:-}" = "" ]; then
  echo "Usage: $0 path/to/.env [docker compose args...]" >&2
  exit 1
fi

ENV_FILE="$1"
shift || true

ORIG_PWD="$(pwd)"
# Resolve env file to absolute path before changing directory
case "$ENV_FILE" in
  /*) ENV_FILE_ABS="$ENV_FILE" ;;
  *) ENV_FILE_ABS="$ORIG_PWD/$ENV_FILE" ;;
esac

if [ ! -f "$ENV_FILE_ABS" ]; then
  echo "Env file not found: $ENV_FILE_ABS" >&2
  exit 1
fi

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
cd "$SCRIPT_DIR"

# Export all variables defined in the .env file
set -a
. "$ENV_FILE_ABS"
set +a

# If no extra args, default to `up --build`
if [ "$#" -eq 0 ]; then
  docker compose up --build
else
  docker compose "$@"
fi


