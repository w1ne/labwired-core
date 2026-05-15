# Foundry Decommission Runbook

**Created:** 2026-05-15
**Status:** Active — manual steps required outside this repo
**Context:** "Foundry" as a hosted-API product framing has been retired in favour of the LabWired CI tier (`packages/api` Cloudflare Worker + `$19/mo` Pro plan). This runbook captures the manual cleanup outside the codebase. Code-level scrub is already committed.

> The legacy `/foundry/` directory, its Go backend, the Hetzner deployment, and the `foundry-deploy.yml` workflow file are **kept** for now. They simply stop auto-deploying (workflow trigger changed to `workflow_dispatch`). Tear them down when you're sure nothing depends on them.

---

## What's already done (in this repo)

| Change | Where |
|---|---|
| Disabled auto-deploy on push to main | `.github/workflows/foundry-deploy.yml` |
| Deleted 6 Foundry-only spec docs | `docs/specs/FOUNDRY_*.md`, `docs/ops/FOUNDRY_PRODUCTION_CHECKLIST.md`, `docs/specs/asset_foundry.md`, `docs/specs/design/FOUNDRY_UI.md` |
| Deleted Foundry user-flows doc | `docs/user_flows.md` |
| Scrubbed positioning copy | `README.md`, `docs/README.md`, strategy/vision docs, `ai/README.md` |

## What you still need to do (manual)

### 1. Stripe — archive the Foundry product

The active Foundry Payment Link is `https://buy.stripe.com/4gM5kC0QeelF1ySa4X5AQ01` (set in commit `519027d`, "feat(foundry): set live Stripe payment link"). It points at a Stripe product whose ID is not in this repo — find it in the dashboard.

> **Do not touch** the LabWired Pro product (`prod_UWR520EMNlJ7uz` / price `price_1TXODOC1n7clsM1CaRfa5EV7`, $19/mo). That's the live CI tier and is wired into `packages/playground/src/ci/CiLanding.tsx`.

Steps:

1. Stripe Dashboard → **Products** → find the product behind the Foundry payment link above. Note its `prod_*` and `price_*` IDs.
2. **Check for active subscriptions first** — Dashboard → Subscriptions → filter by that product. If anyone is paying, *do not archive* until you've migrated them or refunded.
3. Stripe Dashboard → **Payment Links** → find link `4gM5kC0QeelF1ySa4X5AQ01` → **Deactivate**. This stops new signups immediately.
4. Stripe Dashboard → **Products** → open the Foundry product → **Archive product**. Archiving hides it from the dashboard but does not delete payment history.
5. Stripe Dashboard → **Developers → Webhooks** → review whether the Foundry backend webhook endpoint (likely pointing at the Hetzner host) is still listed. If yes and you want signups to stop being delivered there, disable that endpoint. **Do not disable the `api.labwired.com/v1/webhooks/stripe` endpoint** — that's the new CI tier.

### 2. Hetzner VPS — leave running for now

User chose "docs + disable auto-deploy + archive Stripe" — not full infra teardown. The Hetzner box stays up. When you're ready to retire it:

1. Confirm no live subscriptions reach it (step 1 above).
2. SSH in and `docker compose -f /home/w1ne/labwired/foundry/deploy/docker-compose.prod.yml down`.
3. Snapshot the SQLite DB (`foundry/backend/foundry_*.db`) somewhere safe before destroying — it's the audit trail.
4. Cancel the Hetzner project in the Hetzner Cloud console.
5. Remove DNS records for the Foundry subdomain(s) in Cloudflare (likely `foundry.labwired.com` / `foundry.labwired.dev` — confirm in Cloudflare DNS panel).

### 3. CI workflows — currently kept

These workflows still reference `/foundry/`:

- `.github/workflows/foundry-ci.yml` — runs on push, tests the Go backend. Will continue passing while the code is intact.
- `.github/workflows/foundry-deploy.yml` — now `workflow_dispatch` only.
- `.github/workflows/api-ci.yml` — triggers on `foundry/backend/**` paths.
- `.github/workflows/pluto-maintenance.yml` — SSHes into Hetzner for emergency repair.

If/when you fully retire the Hetzner stack, delete these four files in a separate commit.

### 4. SDK telemetry — re-wire later

`ai/labwired_ai/telemetry.py` still exports to `LABWIRED_FOUNDRY_URL`. When you update the AI SDK, point it at the Cloudflare Worker (`api.labwired.com`) instead — schema will need a small adapter since the new Worker takes `POST /v1/runs` with a different body.

---

## Verification checklist

- [ ] Foundry Payment Link `4gM5kC0QeelF1ySa4X5AQ01` shows **Inactive** in Stripe dashboard
- [ ] Foundry product archived in Stripe
- [ ] `$19/mo` Pro product (`prod_UWR520EMNlJ7uz`) still **Active** and accepting payments
- [ ] `https://buy.stripe.com/bJeaEW56u3H16Tc3Gz5AQ03` (Pro Payment Link) still works
- [ ] `foundry-deploy.yml` no longer runs on push (verify by pushing a commit and checking the Actions tab)
- [ ] No mention of "hosted Foundry" remains in the public-facing `README.md` and `docs/README.md`

When the live infra is also retired, add a final commit deleting `/foundry/`, the 4 workflows, and this runbook.
