# Foundry Deployment (GitHub -> GHCR -> Hetzner VPS)

This folder contains a production deployment baseline for Foundry using:
- private GHCR images
- Docker Compose on VPS
- GitHub Actions for build/push/deploy

## Files

- `docker-compose.prod.yml`: backend + frontend + Caddy reverse proxy (image-only).
- `docker-compose.smoke.yml`: local smoke stack (backend + frontend + Caddy) for UI/API click-path checks.
- `Caddyfile`: HTTPS reverse proxy and API/frontend routing.
- `Caddyfile.smoke`: local HTTP reverse proxy for smoke stack.
- `.env.example`: required environment variables template.
- `scripts/harden_vps.sh`: baseline host hardening (SSH, fail2ban, UFW, unattended upgrades).
- `scripts/verify_foundry.sh`: post-deploy backend/API health verification.
- `scripts/smoke_frontend_e2e.sh`: dockerized frontend click-smoke via Playwright.
- `systemd/foundry-compose.service`: optional service wrapper for VPS boot persistence.
- `.github/workflows/foundry-deploy.yml`: CI build/push and VPS deploy workflow.
- `GITHUB_HETZNER_RUNBOOK.md`: full operational runbook (bootstrap, deploy, rollback, troubleshooting).

## 1) One-time VPS preparation

```bash
sudo apt-get update
sudo apt-get install -y docker.io docker-compose-plugin git
sudo systemctl enable --now docker

sudo mkdir -p /srv/labwired
sudo chown -R "$USER":"$USER" /srv/labwired
cd /srv/labwired
git clone git@github.com:<owner>/labwired.git .
```

### Build/provide `labwired` runtime binary on VPS

The backend enforces `LABWIRED_PATH=labwired` in production. Provide it once:

```bash
cd /srv/labwired/core
cargo build --release -p labwired-cli
sudo mkdir -p /srv/labwired/bin
cp target/release/labwired-cli /srv/labwired/bin/labwired
chmod +x /srv/labwired/bin/labwired
```

## 2) Configure deploy env on VPS

```bash
cd /srv/labwired/foundry/deploy
cp .env.example .env
```

Edit `.env`:
- set `OPENAI_API_KEY`, `STRIPE_WEBHOOK_SECRET`
- set `BACKEND_IMAGE` and `FRONTEND_IMAGE` to your GHCR paths
- keep `*_IMAGE_TAG=latest` initially (workflow updates tags automatically)
- set `LABWIRED_BINARY_PATH` to `/srv/labwired/bin/labwired`

## 3) Configure domain

Edit `Caddyfile` and replace `foundry.example.com` with your real domain.

## 4) Configure GitHub secrets

Add repository secrets:
- `VPS_HOST`: public IP or DNS of Hetzner VPS
- `VPS_USER`: SSH user on VPS
- `VPS_SSH_KEY`: private key for that user
- `VPS_DEPLOY_PATH`: `/srv/labwired/foundry/deploy`
- `GHCR_PULL_USER`: GitHub username (or machine user)
- `GHCR_PULL_TOKEN`: token with `read:packages` scope

Also ensure GHCR packages are readable by that pull user (private package access).

## 5) Deploy flow

Push to `main` (or run workflow manually). The workflow:
1. tests backend/frontend
2. builds and pushes GHCR images tagged with `sha-<commit>`
3. SSHes to VPS, updates image tags in `.env`, pulls, and restarts compose

## 6) Verify

```bash
curl -fsS https://<your-domain>/v1/health
curl -fsS https://<your-domain>/v1/info
./scripts/verify_foundry.sh https://<your-domain>
```

## 6.1) Baseline hardening

Run once on the VPS as root:

```bash
cd /srv/labwired/foundry/deploy
sudo ./scripts/harden_vps.sh
```

## 7) Optional systemd integration

```bash
sudo cp systemd/foundry-compose.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now foundry-compose.service
```

## Rollback

Set previous image tags in `.env`:
- `BACKEND_IMAGE_TAG=sha-<old-sha>`
- `FRONTEND_IMAGE_TAG=sha-<old-sha>`

Then:

```bash
docker compose --env-file .env -f docker-compose.prod.yml pull
docker compose --env-file .env -f docker-compose.prod.yml up -d
```

## Local Docker Smoke (Playwright)

From repo root:

```bash
foundry/deploy/scripts/smoke_frontend_e2e.sh
```

This starts `docker-compose.smoke.yml` on `http://127.0.0.1:8088`, waits for `/v1/health`, then runs frontend Playwright smoke tests.
