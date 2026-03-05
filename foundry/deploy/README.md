# Foundry Deployment

This folder contains a production-oriented baseline deployment for Foundry.

## Files

- `docker-compose.prod.yml`: backend + frontend + Caddy reverse proxy.
- `Caddyfile`: HTTPS reverse proxy and API/frontend routing.
- `.env.example`: required environment variables template.
- `systemd/foundry-compose.service`: optional service wrapper for VPS boot persistence.

## 1) Prepare host

```bash
sudo apt-get update
sudo apt-get install -y docker.io docker-compose-plugin
sudo systemctl enable --now docker
```

## 2) Configure secrets

```bash
cd /srv/labwired/foundry/deploy
cp .env.example .env
# edit .env and set OPENAI_API_KEY / STRIPE_WEBHOOK_SECRET
```

## 3) Configure domain

Edit `Caddyfile` and replace `foundry.example.com` with your real domain.

## 4) Deploy

```bash
docker compose --env-file .env -f docker-compose.prod.yml up -d --build
```

## 5) Verify

```bash
curl -fsS https://<your-domain>/v1/health
curl -fsS https://<your-domain>/v1/info
```

## 6) Optional systemd integration

```bash
sudo cp systemd/foundry-compose.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now foundry-compose.service
```

## Rollback

Rollback to the previous image tag by pinning image tags in `docker-compose.prod.yml` and rerunning:

```bash
docker compose --env-file .env -f docker-compose.prod.yml up -d
```
