# GitHub to GHCR to Hetzner Runbook

This runbook documents production deployment and operations for Foundry using:
- GitHub Actions CI/CD
- GHCR container registry (private images)
- Docker Compose on a Hetzner VPS

## Architecture

1. `main` push or manual workflow dispatch triggers `.github/workflows/foundry-deploy.yml`.
2. Workflow runs backend tests and frontend build.
3. Workflow builds and pushes images:
   - `ghcr.io/<owner>/foundry-backend:sha-<commit>` and `:latest`
   - `ghcr.io/<owner>/foundry-frontend:sha-<commit>` and `:latest`
4. Workflow SSHes to VPS and:
   - updates `.env` image tags to `sha-<commit>`
   - `docker compose pull`
   - `docker compose up -d`

## Prerequisites

- Hetzner VPS with public DNS (`A` record) for your domain.
- Docker Engine + Compose plugin installed.
- Repo access and ability to configure GitHub secrets.
- GHCR pull token with `read:packages`.

## One-Time VPS Bootstrap

```bash
sudo apt-get update
sudo apt-get install -y docker.io docker-compose-plugin git
sudo systemctl enable --now docker

sudo mkdir -p /srv/labwired
sudo chown -R "$USER":"$USER" /srv/labwired
cd /srv/labwired
git clone git@github.com:<owner>/labwired.git .
```

### Provide `labwired` runtime binary

The backend uses `LABWIRED_PATH=labwired` and validates it in production.

```bash
cd /srv/labwired/core
cargo build --release -p labwired-cli
sudo mkdir -p /srv/labwired/bin
cp target/release/labwired-cli /srv/labwired/bin/labwired
chmod +x /srv/labwired/bin/labwired
```

## VPS Deploy Config

```bash
cd /srv/labwired/foundry/deploy
cp .env.example .env
```

Edit `Caddyfile` and replace `foundry.example.com` with your real domain.

Edit `.env` and set at minimum:
- `OPENAI_API_KEY`
- `STRIPE_WEBHOOK_SECRET`
- `BACKEND_IMAGE=ghcr.io/<owner>/foundry-backend`
- `FRONTEND_IMAGE=ghcr.io/<owner>/foundry-frontend`
- `BACKEND_IMAGE_TAG=latest`
- `FRONTEND_IMAGE_TAG=latest`
- `LABWIRED_BINARY_PATH=/srv/labwired/bin/labwired`

## GitHub Secrets

Add these repository secrets:
- `VPS_HOST`
- `VPS_USER`
- `VPS_SSH_KEY`
- `VPS_DEPLOY_PATH` (recommended: `/srv/labwired/foundry/deploy`)
- `GHCR_PULL_USER`
- `GHCR_PULL_TOKEN`

## First Deployment

1. Merge deployment workflow/config PR to `main`.
2. In GitHub Actions, run `Foundry Deploy` (or wait for automatic trigger).
3. Validate workflow jobs: `test`, `build-and-push`, `deploy` all green.

### VPS Verification

```bash
cd /srv/labwired/foundry/deploy
docker compose --env-file .env -f docker-compose.prod.yml ps
docker compose --env-file .env -f docker-compose.prod.yml logs --tail=200 foundry-backend
```

### External Health Checks

```bash
curl -fsS https://<your-domain>/v1/health | jq
curl -fsS https://<your-domain>/v1/info | jq
```

## Create API Key for Real Testing

```bash
cd /srv/labwired

docker run --rm \
  -v labwired_foundry_data:/data \
  -v /srv/labwired/foundry/backend:/src \
  -w /src \
  golang:1.24-alpine \
  sh -lc 'go run ./cmd/addkey -db /data/foundry.db -workspace ws-prod'
```

Save the emitted `lw_sk_live_...` key.

## Real-System Smoke Test

```bash
BASE="https://<your-domain>/v1"
KEY="lw_sk_live_..."

RESP=$(curl -sS -X POST "$BASE/systems/verify" \
  -H "Authorization: Bearer $KEY" \
  -H "Content-Type: application/json" \
  -H "Idempotency-Key: smoke-$(date +%s)" \
  -d '{"system_yaml":"name: tiny-system\nchip: tiny-chip.yaml"}')

echo "$RESP" | jq
RUN_ID=$(echo "$RESP" | jq -r .run_id)

curl -sS -H "Authorization: Bearer $KEY" "$BASE/runs/$RUN_ID" | jq
```

## Rollback

1. Identify previous known-good image tag (`sha-<old-commit>`).
2. On VPS, set in `.env`:
   - `BACKEND_IMAGE_TAG=sha-<old-commit>`
   - `FRONTEND_IMAGE_TAG=sha-<old-commit>`
3. Redeploy:

```bash
cd /srv/labwired/foundry/deploy
docker compose --env-file .env -f docker-compose.prod.yml pull
docker compose --env-file .env -f docker-compose.prod.yml up -d
```

## Troubleshooting

### `docker compose pull` returns `unauthorized`
- Verify `GHCR_PULL_USER`/`GHCR_PULL_TOKEN` secret values.
- Ensure token has `read:packages`.
- Ensure package visibility/access grants include the pull user.

### Backend exits at startup with `LABWIRED_PATH command not found`
- Check `LABWIRED_BINARY_PATH` in `.env`.
- Confirm host file exists and is executable:
  - `ls -l /srv/labwired/bin/labwired`

### Deploy workflow updates `.env` but containers not updated
- Run on VPS:
  - `docker compose --env-file .env -f docker-compose.prod.yml pull`
  - `docker compose --env-file .env -f docker-compose.prod.yml up -d`
- Verify running image tags with:
  - `docker inspect <container> --format '{{.Config.Image}}'`

### TLS not issued by Caddy
- Confirm domain DNS points to VPS IP.
- Ensure ports `80` and `443` are open in Hetzner firewall.
- Check logs:
  - `docker compose --env-file .env -f docker-compose.prod.yml logs caddy`
