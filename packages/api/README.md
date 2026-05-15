# LabWired API — Cloudflare Worker Deploy Guide

This Worker handles Stripe webhooks, API key issuance, and per-run cycle metering for the LabWired Pro tier.

## Architecture

```
Stripe checkout → POST /v1/webhooks/stripe → issue key → Resend email → customer
CLI run starts  → POST /v1/keys/validate   → 200 (valid + quota) | 401 | 403
CLI run ends    → POST /v1/runs            → record cycles, return 200 | 429
Dashboard       → GET  /v1/workspaces/me   → workspace info
```

**KV namespaces:**

| Namespace | Key | Value |
|-----------|-----|-------|
| `KV_KEYS` | `lwk_live_<32chars>` | `{ workspace_id, status, created_at, last_used_at }` |
| `KV_WORKSPACES` | `ws_<16hex>` | `{ stripe_customer_id, plan, cycles_used_mtd, ... }` |
| `KV_STRIPE_SUBS` | `sub_xxx` | `workspace_id` |

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

# Also create preview namespaces for local dev:
wrangler kv:namespace create KV_KEYS --preview
wrangler kv:namespace create KV_WORKSPACES --preview
wrangler kv:namespace create KV_STRIPE_SUBS --preview
```

Then edit `wrangler.toml` and replace the `REPLACE_WITH_*` placeholders with the real IDs.

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

### 5. Set up Resend

1. Sign up at [resend.com](https://resend.com)
2. Add and verify the `labwired.com` domain (Resend will give you DNS records to add in Cloudflare)
3. Create an API key with "Sending access":

```bash
wrangler secret put RESEND_API_KEY
# Paste the re_... key when prompted
```

### 6. DNS for api.labwired.com

Option A — Cloudflare Worker custom domain (recommended):

1. Cloudflare Dashboard → Workers & Pages → your worker → Settings → Domains & Routes
2. Add custom domain: `api.labwired.com`

Option B — CNAME (if custom domains aren't available on your plan):

1. Cloudflare Dashboard → DNS → Add record
2. Type: CNAME, Name: `api`, Target: `labwired-api.<your-account>.workers.dev`, Proxy: enabled

### 7. Deploy

```bash
cd packages/api
npm install
wrangler deploy
```

### 8. Verify the deploy

```bash
# Should return 401
curl -X POST https://api.labwired.com/v1/keys/validate \
  -H "Content-Type: application/json" \
  -d '{"api_key":"lwk_live_TESTINVALID"}'
# Expected: {"error":"API key not found"}

# Should return 404
curl https://api.labwired.com/v2/whatever
```

### 9. End-to-end test

1. Set up a Stripe test mode Payment Link pointing at your Pro product
2. Use a [Stripe test card](https://docs.stripe.com/testing) to complete checkout
3. Watch `wrangler tail` for the webhook log lines
4. Check your email for the onboarding message
5. Test the key:

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

There is no automated key rotation endpoint yet (v1 scope). To rotate manually:

1. Look up the workspace: `wrangler kv:key get --binding=KV_KEYS "lwk_live_<old_key>"`
2. Note the `workspace_id`
3. Generate a new key (any base32 string starting with `lwk_live_`)
4. Write the new key record and update the workspace's `api_key` field in KV
5. Email the customer with the new key
6. Delete the old key: `wrangler kv:key delete --binding=KV_KEYS "lwk_live_<old_key>"`
