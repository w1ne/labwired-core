[← Back to Hub](../README.md)

# Foundry Production Readiness Checklist

Use this checklist to cut the first production deployment of the Foundry service.

## 0. Release Gate

- [ ] Target commit merged to `main` through PR (no direct pushes).
- [ ] Branch protection on `main` enforced (PR + required checks + linear history).
- [ ] All required checks are green for deployment commit.
- [ ] Deployment owner and rollback owner assigned.

## 1. Build and Test Gate

Run from `foundry/backend`:

```bash
/usr/local/go/bin/go test ./...
/usr/local/go/bin/go build ./cmd/server
```

Run from `foundry/frontend`:

```bash
npm ci
npm run build
```

- [ ] Backend tests pass.
- [ ] Backend server binary builds.
- [ ] Frontend production build succeeds.

## 2. Security and Config Gate

- [ ] `APP_ENV=production` set.
- [ ] `STRIPE_WEBHOOK_SECRET` configured.
- [ ] `ALLOW_INSECURE_STRIPE_WEBHOOKS` unset or explicitly `false`.
- [ ] `WORKER_LEASE_TIMEOUT_SECONDS` and `WORKER_HEARTBEAT_INTERVAL_SECONDS` configured with `heartbeat < lease`.
- [ ] `OPENAI_API_KEY` configured via secret manager (not plaintext in repo).
- [ ] API key backfill completed if legacy rows exist (`KEY_PREFIX_BACKFILL_PATH`).
- [ ] CORS policy reviewed for production origin restrictions.

## 3. Data and Retention Gate

- [ ] Persistent volume mounted for SQLite DB (`DB_PATH`).
- [ ] Persistent volume mounted for artifacts (`ARTIFACTS_DIR`).
- [ ] `ARTIFACT_RETENTION_DAYS` explicitly set.
- [ ] `RUN_METADATA_RETENTION_DAYS` explicitly set.
- [ ] Backup policy defined for DB + artifact metadata.
- [ ] Restore drill performed from latest backup.

## 4. Runtime and SLO Gate

- [ ] Health endpoint checked: `GET /v1/health`.
- [ ] Health metrics include non-zero/stable lease behavior (`components.metrics.lease_requeues` monitored).
- [ ] Readiness smoke checked:
  - `GET /v1/info`
  - `GET /v1/hardware`
  - authenticated `GET /v1/usage`
- [ ] Alerting configured for:
  - non-2xx health checks
  - queue saturation sustained over threshold
  - webhook failures
- [ ] Log retention and PII policy documented.

## 5. Billing and Webhook Gate

- [ ] Stripe webhook endpoint reachable externally.
- [ ] Stripe signature validation enabled in production.
- [ ] Duplicate webhook idempotency validated with replay test.
- [ ] Quota credit flow validated end-to-end.

## 6. Deployment Gate

- [ ] Deploy performed via `foundry/deploy/docker-compose.prod.yml` (or equivalent).
- [ ] Reverse proxy TLS enabled (Caddy/Nginx) with HTTPS redirect.
- [ ] Service auto-restart enabled.
- [ ] Rollback command tested before release window.

## 7. Post-Deploy Verification

- [ ] Smoke verification run submitted and completed.
- [ ] Artifacts retrieval validated for workspace ownership.
- [ ] Error path validated (invalid key returns 401 with no leakage).
- [ ] Runbook updated with deployment timestamp and commit SHA.

## Deployment Record Template

- Date:
- Commit SHA:
- Deployed by:
- Rollback owner:
- Backend image/tag:
- Frontend image/tag:
- Health URL:
- Backup location:
- Known issues:
