# CI Landing — Setup checklist

Three placeholder URLs in `CiLanding.tsx` need real values before this page accepts money. All three are zero-backend, "copy what works" services that thousands of SaaS founders use to ship v1.

| Placeholder | Replace with | Steps |
|---|---|---|
| `STRIPE_TEAM_PAYMENT_LINK` | A Stripe Payment Link URL | 1. `dashboard.stripe.com` → Products → New product. <br>2. Price: $49/mo recurring. <br>3. Payment Links → New → select product. <br>4. Copy URL like `https://buy.stripe.com/abc123`. |
| `CALENDLY_ENTERPRISE` | Your Calendly intro-call link | 1. Create Calendly account. <br>2. New event type → 30-min intro. <br>3. Copy `https://calendly.com/<you>/enterprise-intro`. |
| `WAITLIST_FORM_ACTION` | Formspree / Tally / ConvertKit endpoint | Default is `mailto:hello@labwired.com` (works today, no setup). When list grows: <br>1. `formspree.io` → New form (free 50/mo). <br>2. Replace with `https://formspree.io/f/<id>`. |

## Why this stack

- **Stripe Payment Links** — no checkout integration, no webhooks, no backend. Customer pays → Stripe emails you → you manually issue access. Good until ~50 customers. After that, automate with Stripe Checkout + webhooks.
- **Calendly** — every enterprise sales motion in 2026 starts with a Calendly link. Their procurement team books a call; you ask the qualifying questions on the call.
- **Formspree / mailto** — collecting waitlist emails before you have product. Avoid building a `/api/waitlist` route in v1.

## What's next when you outgrow this

When you have 10+ paying customers and the manual flow is painful:

1. **Auth:** Clerk (~30 min integration) or Auth.js. GitHub OAuth is what devs expect.
2. **Stripe Checkout + webhooks:** customer pays → webhook creates workspace → API key issued. ~1 day to wire up via a Cloudflare Worker or Vercel Function.
3. **API-key-gated CI:** the GitHub Action accepts an env var `LABWIRED_API_KEY`; backend meters cycle consumption per key. ~2-3 days for a real metering pipeline.
4. **Usage dashboard:** Vercel + Next.js page where the customer sees cycle consumption + invoices.

Total v2 spend: ~1 week of focused engineering, after the manual v1 has validated demand.

## Don't build prematurely

Wokwi shipped their Club tier years before they had ANY of the auth/billing infra — they collected emails, hand-onboarded, then automated. Founder-led sales beats premature SaaS infrastructure for the first 10 customers.
