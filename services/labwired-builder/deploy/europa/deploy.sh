#!/usr/bin/env bash
# Build the labwired COMPILE image ON Europa and swap the stack — NO 15GB uplink
# transfer. Run from your laptop, from anywhere in the repo.
#
#   bash services/labwired-builder/deploy/europa/deploy.sh
#
# Only the compile source (a few MB) is uploaded; the ~8GB image is BUILT ON
# EUROPA (datacenter bandwidth pulls the PlatformIO frameworks). A BuildKit cache
# on the box makes catalog-only edits rebuild in seconds. Rollback is instant
# (the previous image is kept as :prev until the new one passes health).
#
# The 575MB builder image changes rarely; rebuild/transfer it separately when its
# code (src/server.ts etc.) changes — this script reuses the running builder.
set -euo pipefail

HOST="${EUROPA_HOST:-admin@157.180.86.12}"
KEY="${EUROPA_KEY:-$HOME/projects/0.Servers/keys/id_ed25519}"
TAG="${IMAGE_TAG:-dev}"
SSH_OPTS=(-i "$KEY" -o IdentitiesOnly=yes -o ConnectTimeout=15)
SRC_LOCAL="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"   # services/labwired-builder
REMOTE_SRC=/home/admin/labwired-builder-src

echo "==> sync compile source -> $HOST:$REMOTE_SRC  (image is built on Europa, not uploaded)"
rsync -az --delete --exclude node_modules --exclude '.env*' --exclude '*.log' \
  -e "ssh ${SSH_OPTS[*]}" "$SRC_LOCAL/" "$HOST:$REMOTE_SRC/"

echo "==> build + swap on Europa"
IMG="ghcr.io/w1ne/labwired-compile"
ssh "${SSH_OPTS[@]}" "$HOST" "TAG=$TAG IMG=$IMG REMOTE_SRC=$REMOTE_SRC bash -s" <<'REMOTE'
set -euo pipefail
cd /opt/labwired-builder-compose
C="docker compose -f docker-compose.yml -f docker-compose.europa.yml"

# Keep the current image as a rollback tag, build the new one.
docker tag "$IMG:$TAG" "$IMG:prev" 2>/dev/null || true
echo "==> building $IMG:$TAG on this box (cache makes catalog edits fast)"
docker buildx build -f "$REMOTE_SRC/Dockerfile.compile" -t "$IMG:$TAG" --load "$REMOTE_SRC"

echo "==> recreate stack (both compile lanes + builder)"
$C up -d compile compile-net builder

echo "==> health"
ok=0
for i in $(seq 1 40); do
  h=1
  for c in compile compile-net builder; do
    s=$(docker inspect "labwired-builder-compose-${c}-1" --format '{{.State.Health.Status}}' 2>/dev/null || echo x)
    [ "$s" = healthy ] || h=0
  done
  if [ "$h" = 1 ]; then ok=1; break; fi
  sleep 3
done

if [ "$ok" = 1 ]; then
  echo "==> healthy — dropping rollback image + pruning dangling"
  docker rmi "$IMG:prev" >/dev/null 2>&1 || true
  docker image prune -f >/dev/null 2>&1 || true
  $C ps
  df -h / | tail -1
else
  echo "!! unhealthy — ROLLING BACK to :prev"
  docker tag "$IMG:prev" "$IMG:$TAG"
  $C up -d compile compile-net builder
  $C ps
  exit 1
fi
REMOTE
echo "Done. https://builder.labwired.com/healthz"
