# Production Deploy — Post-mortem reference

**Status:** Production was cut over on **2026-05-15**. This document is now a
historical reference, not an action list. `api.labwired.com` is live and
accepting Stripe payments; `@labwired/mcp@0.1.0` is published to npm.

This file used to be a multi-stage runbook. Several of those stages no longer
match reality (Resend was removed from the architecture; the `workers.dev`
dry-run host is gone now that the custom domain is live). What follows is a
tight summary of what each stage actually was and what the current state is,
so the same path can be re-walked when needed.

---

## What is live

### Status update — 2026-06-02

- **Landing page:** Hosted MCP connector instructions are live on
  `labwired.com` and show the Codex hosted-URL flow without a custom scope:
  `codex mcp add labwired --url https://api.labwired.com/mcp && codex mcp login labwired`.
- **API source + production:** The hosted MCP OAuth fix is merged in
  `w1ne/labwired#205` and deployed to `api.labwired.com`. Worker
  metadata/challenges no longer advertise or request `labwired:mcp`, because
  Clerk rejects that custom scope and the MCP resource server only validates a
  Clerk bearer token.
- **API deploy automation:** `w1ne/labwired#210` added
  `.github/workflows/api-worker-deploy.yml`. Pushes to `main` that touch
  `packages/api/**` run `npm ci`, `npm test`, and `npx wrangler deploy` with
  `CLOUDFLARE_API_WORKER_TOKEN` + `CLOUDFLARE_API_ACCOUNT_ID`. The first
  successful production deploy was GitHub Actions run `26828719012`.

### Baseline — 2026-05-15

- **Worker:** `labwired-api` deployed to `api.labwired.com` (Cloudflare custom
  domain, not `workers.dev`).
- **KV namespaces:** `KV_KEYS`, `KV_WORKSPACES`, `KV_STRIPE_SUBS`,
  `KV_CLERK_TO_WORKSPACE`, `KV_SESSIONS` — all populated, IDs in
  `packages/api/wrangler.toml`.
- **Stripe:** Pro Payment Link wired, webhook endpoint at
  `https://api.labwired.com/v1/webhooks/stripe`, restricted key stored as
  `STRIPE_SECRET_KEY`, signing secret stored as `STRIPE_WEBHOOK_SECRET`.
- **Auth:** Clerk (not GitHub OAuth — that was an earlier plan and is not
  wired). On Stripe checkout return, the API key shows up in the customer's
  private cabinet immediately. No transactional email is sent.
- **MCP:** `@labwired/mcp@0.1.0` published to npm under the `@labwired` scope.

## What was retired

- **Resend.** The original plan was to send an onboarding email containing the
  API key after Stripe checkout. We replaced that with a cabinet-render path
  (Wokwi pattern): the key is fetched from `/v1/auth/me` when the customer
  returns to the playground signed-in. No domain verification, no Resend
  secret, no email deliverability surface. If you see a `RESEND_*` reference
  anywhere in the codebase or docs, it is stale.
- **`labwired-api.<account>.workers.dev` dry-run host.** Was used during
  initial staging. The custom-domain route is now permanent in
  `wrangler.toml`; the workers.dev URL still resolves but is not the
  documented surface.
- **Foundry hosted-API framing (Go backend, runs-per-month pricing).** Retired
  on 2026-05-15. See `docs/ops/FOUNDRY_DECOMMISSION.md`.

## What the stages were, conceptually

1. **Worker dry-run.** Stand the Worker up on `workers.dev` first, confirm
   404/401 shapes, then move it to `api.labwired.com`. Useful pattern if you
   ever need to re-stage a major rewrite without disturbing prod.
2. **Stripe wiring.** Restricted key + webhook signing secret as Worker
   secrets; webhook endpoint receives `checkout.session.completed`,
   `customer.subscription.{deleted,updated}`, and `invoice.payment_failed`.
3. ~~**Resend.**~~ Removed. Cabinet-render replaces it.
4. ~~**GitHub OAuth.**~~ Not wired. Clerk handles sign-in.
5. **Custom-domain cutover.** Uncomment the `routes = [...]` block in
   `wrangler.toml`, redeploy, then update Stripe webhook URL to point at
   `api.labwired.com`. Cloudflare auto-provisions the custom domain on deploy.
6. **MCP publish.** `cd packages/mcp && npm publish --access public`.
   Verifiable with `npx -y @labwired/mcp`.

## Smoke checks (re-runnable anytime)

```bash
# Format-gate rejection
curl -s -X POST https://api.labwired.com/v1/keys/validate \
  -H "Content-Type: application/json" \
  -d '{"api_key":"lwk_live_TESTINVALID"}'
# Expect: {"error":"Invalid API key format"}

# Unknown path
curl -s https://api.labwired.com/v1/nope
# Expect: {"error":"Not found"}

# MCP boot
npx -y @labwired/mcp --help 2>&1 | head

# Hosted MCP OAuth metadata
curl -s https://api.labwired.com/.well-known/oauth-protected-resource/mcp
# Expected:
# - authorization_servers includes https://clerk.labwired.com
# - scopes_supported is absent
# - no labwired:mcp scope is advertised
```

## Rollback

```bash
cd packages/api
wrangler deployments list
wrangler rollback --message "rollback to <sha>"
```

Stripe webhook URL is editable in the dashboard; revert there in lockstep.

## Where to look next

- Worker source: `packages/api/`
- MCP source: `packages/mcp/`
- Customer-facing cabinet flow: `packages/playground/src/studio/`
- Outbound prospecting: `docs/strategy/outreach/`
- Pitch artifact: `docs/strategy/pitch/labwired-as-agent-oracle.md`
