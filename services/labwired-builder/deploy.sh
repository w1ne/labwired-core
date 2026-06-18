#!/usr/bin/env bash
# Steady-state deploy: pull the latest images and (re)start the stack.
# Run from the deploy directory on the Hetzner box (where .env lives).
set -euo pipefail
cd "$(dirname "$0")"

if [ ! -f .env ]; then
  echo "ERROR: .env not found. Copy .env.example to .env and fill it in." >&2
  exit 1
fi

docker compose pull
docker compose up -d
docker compose ps
