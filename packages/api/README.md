# LabWired API — Cloudflare Worker Deploy Guide

This Worker handles Stripe webhooks, API key issuance, and per-run cycle metering for the LabWired Pro tier.

## Architecture

```
Stripe checkout → POST /v1/webhooks/stripe → issue key (mapped to Clerk user via client_reference_id)
Playground      → GET  /v1/auth/me         → key shown in private cabinet (Wokwi-pattern)
CLI run starts  → POST /v1/keys/validate   → 200 (valid + quota) | 401 | 403
CLI run ends    → POST /v1/runs            → record cycles, return 200 | 429
Cabinet rotate  → POST /v1/keys/rotate     → new key, old key invalidated
Dashboard       → GET  /v1/workspaces/me   → workspace info (Bearer api_key)
```

**KV namespaces:**

| Namespace | Key | Value |
|-----------|-----|-------|
| `KV_KEYS` | `lwk_live_<32chars>` | `{ workspace_id, status, created_at, last_used_at }` |
| `KV_WORKSPACES` | `ws_<16hex>` | `{ stripe_customer_id, plan, cycles_used_mtd, clerk_user_id, ... }` |
| `KV_STRIPE_SUBS` | `sub_xxx` | `workspace_id` |
| `KV_CLERK_TO_WORKSPACE` | `user_xxx` (Clerk user id) | `workspace_id` |

---

## Deploy steps

Run these commands yourself — the deploy requires your Cloudflare account.

### 1. Prerequisites

```bash
npm install -g wrangler
wrangler login
```

### 2. Create KV namespaces

Run each command and paste the returned `id` into `wrangler.toml`:

```bash
wrangler kv:namespace create KV_KEYS
wrangler kv:namespace create KV_WORKSPACES
wrangler kv:namespace create KV_STRIPE_SUBS
wrangler kv:namespace create KV_CLERK_TO_WORKSPACE

# Also create preview namespaces for local dev:
wrangler kv:namespace create KV_KEYS --preview
wrangler kv:namespace create KV_WORKSPACES --preview
wrangler kv:namespace create KV_STRIPE_SUBS --preview
wrangler kv:namespace create KV_CLERK_TO_WORKSPACE --preview
```

Then edit `wrangler.toml` and replace the `REPLACE_ME` / placeholder IDs with the real values.

### 3. Set Stripe secrets

**Important:** Do NOT use the Stripe secret key that was previously exposed. Create a new restricted key:

1. Go to Stripe Dashboard → Developers → API keys → Create restricted key
2. Give it: Read access to Customers, Subscriptions; no other permissions needed for the webhook handler
3. Store it:

```bash
wrangler secret put STRIPE_SECRET_KEY
# Paste the new restricted key when prompted
```

### 4. Set up Stripe webhook

1. Stripe Dashboard → Developers → Webhooks → Add endpoint
2. URL: `https://api.labwired.com/v1/webhooks/stripe`
3. Select events:
   - `checkout.session.completed`
   - `customer.subscription.deleted`
   - `customer.subscription.updated`
   - `invoice.payment_failed`
4. Copy the signing secret (starts with `whsec_`):

```bash
wrangler secret put STRIPE_WEBHOOK_SECRET
# Paste the whsec_... signing secret when prompted
```

### 5. DNS for api.labwired.com

Option A — Cloudflare Worker custom domain (recommended):

1. Cloudflare Dashboard → Workers & Pages → your worker → Settings → Domains & Routes
2. Add custom domain: `api.labwired.com`

Option B — CNAME (if custom domains aren't available on your plan):

1. Cloudflare Dashboard → DNS → Add record
2. Type: CNAME, Name: `api`, Target: `labwired-api.<your-account>.workers.dev`, Proxy: enabled

### 6. Deploy

```bash
cd packages/api
npm install
wrangler deploy
```

### 7. Verify the deploy

```bash
# Should return 401
curl -X POST https://api.labwired.com/v1/keys/validate \
  -H "Content-Type: application/json" \
  -d '{"api_key":"lwk_live_TESTINVALID"}'
# Expected: {"error":"Invalid API key format"}
# (lwk_live_TESTINVALID is too short to pass the format gate; a well-formed but
# unknown key like lwk_live_<32 random chars> returns {"error":"API key not found"}.)

# Should return 404
curl https://api.labwired.com/v2/whatever
```

### 8. End-to-end test

1. Set up a Stripe test mode Payment Link pointing at your Pro product
2. Visit the playground signed in via Clerk, click "Upgrade" — the link includes
   `?client_reference_id=<clerk_user_id>&prefilled_email=<email>` so the webhook
   can map the new workspace back to that Clerk user.
3. Use a [Stripe test card](https://docs.stripe.com/testing) to complete checkout
4. Watch `wrangler tail` for the webhook log lines
5. Re-open the playground cabinet — the new `lwk_live_*` API key is shown,
   copyable, and rotatable. No email is sent.
6. Test the key:

```bash
LABWIRED_API_KEY=lwk_live_<your_key> labwired test --script tests/example.yaml
```

---

## Local development

```bash
# Copy and fill in the example secrets
cp .dev.vars.example .dev.vars
# Edit .dev.vars with real test values

# Create preview KV data (optional)
wrangler kv:key put --binding=KV_KEYS "lwk_live_TESTKEY" '{"workspace_id":"ws_test","status":"active","created_at":"2026-01-01T00:00:00Z","last_used_at":null}' --preview

# Start local dev server
npm run dev
```

For Stripe webhooks in local dev, use the [Stripe CLI](https://stripe.com/docs/stripe-cli):

```bash
stripe listen --forward-to localhost:8787/v1/webhooks/stripe
```

---

## Monitoring

```bash
# Stream live logs
wrangler tail

# Check KV contents
wrangler kv:key list --binding=KV_WORKSPACES
wrangler kv:key get --binding=KV_KEYS "lwk_live_<key>"
```

---

## Rotating an API key

Customers can self-serve rotation from the playground cabinet — that calls
`POST /v1/keys/rotate` with their Clerk session JWT. The Worker generates a
new `lwk_live_*`, swaps it into `KV_KEYS` + `KV_WORKSPACES`, and deletes the
old key in a single request.

To rotate from the CLI for a workspace whose owner can't sign in, edit KV by
hand: write a new `lwk_live_*` key record, update `KV_WORKSPACES.api_key`, then
delete the old `KV_KEYS` entry.
