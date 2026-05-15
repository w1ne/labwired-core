[← Back to Strategy](./README.md)

# Launch Readiness Plan — Gap to "Sellable"

**Date:** 2026-05-15
**Status:** Draft · plan-of-record for the path from "feature-complete" to "first paying customer"
**Context:** The product is feature-complete (Studio playground · 10 working labs · CI productization page · Library page · open-source Rust core). This document enumerates every remaining gap before LabWired can charge money, then prioritizes the work.

> **Read this first if you're tempted to add features.** The product isn't the bottleneck — the *sales motion* is. This plan front-loads the boring infra that gates the first dollar of revenue.

---

## 1. What's already shipped (baseline)

For context — the gap analysis below assumes all of this is in place:

| Surface | Status |
|---|---|
| Studio playground (dark shell, 10 working labs, Upload ELF) | ✅ shipped |
| 8 supported chip platforms (STM32F1/F4/H5, RP2040, nRF52, ESP32-C3/S3) | ✅ shipped |
| Cross-cutting plumbing (I²C / SPI / UART / ADC device attach) | ✅ shipped |
| Library page (boards + featured labs) | ✅ shipped |
| CI productization page (`/playground/ci.html`) | ✅ shipped |
| Landing page (light/neo-brutalist palette, multi-arch messaging) | ✅ shipped |
| Open-source Rust core | ✅ shipped (MIT) |
| `labwired test` CLI runner | ✅ shipped |
| GitHub Action wrapper | ✅ shipped |
| VS Code extension | ✅ shipped (per existing roadmap) |
| HIL Displacement Showcase whitepaper (Markdown) | ✅ exists in `docs/strategy/` |

**What this means:** You have the **product**. You do not yet have the **business**.

---

## 2. Definition of "sellable"

LabWired is sellable when:

1. A visitor can land on `labwired.com`, follow a clear path to the CI tier, and **pay**.
2. The payment results in a **provisioned workspace + API key** they can drop into a GitHub Action.
3. The GitHub Action **authenticates** against their key and meters usage.
4. The customer receives a **receipt + onboarding email** they can forward to their procurement team.
5. **Legal** is sufficient (privacy policy, ToS, cookie consent) for Stripe to keep the account open.
6. **You can recognize when a real human is interested** (analytics, Slack notification, CRM record).

Anything beyond this is icing. Anything below this is a leak.

---

## 3. Day-1 blockers — must ship before charging $1

These items have **no workaround**. Until each is done, the pricing page is theater.

### 3.1 Real Stripe Checkout + webhook (1-2 days)
- Current state: Payment Link placeholder in `CiLanding.tsx` line 11 (`STRIPE_TEAM_PAYMENT_LINK`).
- Action:
  1. Create Stripe product "LabWired CI Team" — `$49/mo` recurring (or final price).
  2. Create Payment Link OR full Checkout integration via Stripe API.
  3. Webhook endpoint (Cloudflare Worker or Vercel Function) that listens for `checkout.session.completed`.
  4. On success: generate API key, store in KV (Cloudflare KV / Upstash Redis), email customer with the key.
  5. Customer receipt automatically sent by Stripe.
- Files to touch: `packages/playground/src/ci/CiLanding.tsx` (real URL), new `api/stripe-webhook.ts` Worker, new `api/issue-key.ts`.
- Definition of done: a test transaction in Stripe test mode produces an email with a working API key.

### 3.2 API-key gating on the CI runner (2-3 days)
- Current state: `labwired-test` GitHub Action runs anonymously. No auth, no metering, no quota.
- Action:
  1. Extend `labwired-cli` to accept `LABWIRED_API_KEY` env var.
  2. CLI POSTs run metadata (cycles consumed, firmware hash, runner ID) to `https://api.labwired.com/runs` with the key.
  3. Backend validates key against KV store; rejects if expired/over-quota; logs cycles consumed against the workspace.
  4. Update GitHub Action composite to forward `secrets.LABWIRED_API_KEY` to the CLI.
  5. Free tier (no key) still works but rate-limited to N runs/day per IP.
- Files to touch: `core/crates/cli/src/main.rs`, `.github/actions/labwired-test/action.yml`, new `api/runs.ts` endpoint.
- Definition of done: a paid customer's CI build authenticates, runs, and shows up as a cycle-consumption record in the backend.

### 3.3 Public contact email ✅ DONE
- Using `andrii@shylenko.com` (founder's PR/public inbox, already in use across other projects).
- All landing + CI + Library + WaitlistModal contact links now route to this address.
- **Migration path:** swap to `hello@labwired.com` (or a shared inbox like Front/Help Scout) once volume justifies splitting personal from product mail. Probably trigger that at ~10 inbound emails/week.

### 3.4 Real Calendly URL (15 min)
- Current state: `CALENDLY_ENTERPRISE = 'https://calendly.com/labwired/enterprise-intro'` is a placeholder.
- Action:
  1. Create personal Calendly (free tier OK).
  2. New event type: "LabWired Enterprise Intro" — 30 minutes — sync with Google Calendar.
  3. Replace placeholder URL in `CiLanding.tsx`.
- Definition of done: book a slot yourself via the public URL; verify it appears on your calendar.

### 3.5 Privacy policy + Terms of service (2-4 hours)
- Stripe legally requires both linked from the checkout flow before activating live payments.
- Action:
  1. Use Termly (free tier) or generate via Cookiebot / SimpleAnalytics templates.
  2. Add `/privacy.html` and `/terms.html` pages — match the landing page's visual language.
  3. Link in the playground footer + CI page footer + Stripe Checkout settings.
- Definition of done: both URLs return rendered pages and are linked from the footer of every public page.

### 3.6 Domain + DNS + SSL (1-2 hours)
- Verify `labwired.com` (landing) and the playground subdomain actually serve the current branch.
- Set up `api.labwired.com` and `status.labwired.com` subdomains.
- Cloudflare or similar for SSL + CDN.

**Day-1 sum: ~5-8 days of focused work** if you do all six sequentially. Stripe + API-key gating dominates.

---

## 4. Credibility tier — need before any real B2B conversation

You can ship the Day-1 items and theoretically take money — but enterprise buyers will bounce without these signals.

### 4.1 Demo video / GIF on landing hero (2-3 hours)
- 30-second screen recording: cold-load → click TFT Color → click Run → 240×320 color framebuffer fills with EBU bars → cycle counter ticks.
- Embed as `<video autoplay muted loop playsinline>` in the hero section of `index.html` next to the H1.
- Alternative: a high-quality GIF if video bandwidth is a concern.

### 4.2 HIL Displacement whitepaper PDF (1-2 hours)
- Already written: `docs/strategy/HIL_DISPLACEMENT_SHOWCASE.md`.
- Action: convert to a styled PDF (Pandoc + LaTeX, or just print the rendered HTML to PDF), host on the CDN, email-gate the download.
- Link from CI page comparison section.
- **Why it matters**: lets enterprise procurement folks pass it around internally to justify the spend.

### 4.3 Comparison pages (1 day)
- `/playground/vs-wokwi.html` — "When to pick LabWired over Wokwi"
- `/playground/vs-renode.html` — "When to pick LabWired over Renode"
- Each ~600 words, honest pros/cons table, links to the live playground for proof.
- SEO targets: "Wokwi alternative", "Renode alternative", "STM32 simulator comparison".

### 4.4 Status page (1 hour)
- `status.labwired.com` via Vercel/Netlify static site or BetterUptime free tier.
- Initially shows: "Playground · Operational" / "CI API · Operational".
- Real uptime monitoring once you have paying customers.

### 4.5 First case study or design-partner quote (weeks of sales)
- Cannot fake this. Need a real embedded team using LabWired and willing to be quoted.
- Outbound to 50 prospects → 10 calls → 3 trials → 1 quote is a realistic funnel.
- The first 5 customers should be hand-onboarded specifically to extract a quote.

### 4.6 GitHub stars + open-source flywheel (weeks)
- Post on Hacker News with the determinism wedge angle.
- Post on `/r/embedded`, `/r/rust`, `/r/electronics`.
- Tweet/X thread with the TFT framebuffer rendering in the browser.
- Submit to *Hardware Hub* / *Embedded Weekly* / *This Week in Rust*.

---

## 5. Sales motion — who buys, who reaches out, how

**The product is more than ready. The sales motion is the missing organ.**

### 5.1 First 10 customers — founder-led only
You personally sell the first 10. No SDRs, no AEs, no salesperson hire. Reason: only the founder can iterate the pricing/packaging/messaging fast enough off feedback from real conversations.

### 5.2 Build the outbound list (4-8 hours)
- 50 named prospects: embedded engineers + engineering managers at companies with HIL pain.
- Verticals to target first:
  - **Automotive** (ISO 26262 angle) — Tier-1 suppliers, EV startups
  - **Medical** (IEC 62304 angle) — infusion pumps, monitors
  - **Industrial control** (IEC 61508) — PLC vendors, motor drives
  - **Consumer IoT** — ESP32-heavy shops with regression-test pain
- Source: LinkedIn Sales Navigator (paid), Apollo.io (paid trial), GitHub stargazers of relevant projects (free).

### 5.3 Outbound script (1 hour)
- 3-sentence cold email: hook (their pain) + proof (HIL whitepaper link) + ask (15-min call).
- A/B test 2-3 hooks across batches of 25.

### 5.4 Discovery-call script (1 hour)
- 5 questions:
  1. Walk me through your current regression-test pain.
  2. What's the cost (time + $) of your current HIL setup?
  3. Who'd be the budget owner for replacing it?
  4. What would need to be true for you to greenlight a 30-day trial?
  5. Is there a deal-breaker (SAML, on-prem, ISO cert) I should know about?

### 5.5 CRM (2 hours setup)
- Notion / Airtable / Pipedrive — anything that tracks: prospect, status, next action, last touch, source.
- Beyond ~20 prospects in flight, an actual CRM saves your sanity.

### 5.6 Tier-2 sales material (1 day, on demand)
- Slide deck for enterprise procurement — 8-10 slides: problem, solution, proof, pricing, security posture, references, roadmap.
- Security questionnaire pre-answers (data residency, encryption, SOC 2 roadmap, BCP).
- Build only when an enterprise asks; don't pre-build for procurement that doesn't exist.

---

## 6. Compliance / legal

| Item | Effort | Triggers |
|---|---|---|
| Privacy policy | 2 hours | Required by Stripe; required by GDPR |
| Terms of service | 2 hours | Required by Stripe |
| Cookie consent banner | 2 hours | GDPR (EU traffic); just use Cookiebot free tier |
| Data Processing Agreement template | 4 hours | Enterprise customers in EU will ask |
| MIT license confirmation in README | ✅ done | Already there |
| Trademark filing on "LabWired" | $250 + 2 hours | Optional; do when revenue > $10k MRR |
| EU VAT registration | 4 hours | Triggered at €10k EU sales/year |
| Sales tax registration (US) | varies | Triggered per state thresholds (Stripe handles most) |
| SOC 2 Type 1 readiness | 3-6 months | Required by big-co enterprise procurement |
| ISO 26262 evidence kit | weeks | The automotive wedge angle — sell as Tier-2 add-on |

---

## 7. Analytics & instrumentation — fly blind, get sold blind

### 7.1 Plausible or PostHog on landing + playground (1 hour)
- Plausible is privacy-friendly, no cookie banner needed, $9/mo. PostHog is free+rich but requires consent.
- Track: visit → playground click → CI page → Calendly book → Stripe checkout.
- This is the funnel you'll optimize for the rest of the company's life.

### 7.2 Stripe → Slack/Discord webhook (30 min)
- Every new signup pings a channel. Founder excitement + 2-hour onboarding response time.

### 7.3 Email capture for non-buyers (1 hour)
- Already partial via waitlist mailto. Upgrade to ConvertKit/Mailerlite ($0 to ~$15/mo at low list size).
- Drip sequence: HIL whitepaper → comparison post → "request demo" CTA after 7 days.

---

## 8. Long-term product credibility (months out)

Build these only after the first 5 paying customers ask for them.

| Item | Why | Effort |
|---|---|---|
| Customer dashboard with usage graphs | Self-serve clarity reduces support load | 1 week |
| ISO 26262 Tool Qualification Kit (TQK) | Automotive procurement requires it; $50-100k contracts | 3-4 weeks |
| SAML / SSO for Enterprise tier | Required at Series-A-ish target companies | 1 week (WorkOS or Auth0) |
| On-prem / self-hosted bundle | Paranoid customers — already Docker, license-key it | 1 week |
| Status & changelog page (auto-generated) | Maturity signal | 1 day |
| Public roadmap (Productboard / Linear public) | Signals you're listening | 2 hours |
| Discord community | Self-serve support, community marketing | 2 hours setup + ongoing care |
| Blog / changelog cadence | Inbound SEO + retention | weekly commitment |

---

## 9. Strategic / positioning gaps

### 9.1 Distribution strategy
- **Inbound:** SEO (comparison pages), HN/Reddit launches, GitHub star flywheel.
- **Outbound:** the 50-prospect list above.
- **Partner:** ST Microelectronics dev community, embedded.fm podcast sponsorship, embedded conferences (FOSDEM embedded track, Embedded World, Hackaday Supercon).
- **Channel:** Wokwi has a marketplace for custom chips — explore whether LabWired components could be cross-listed.

### 9.2 Pricing experimentation
- The placeholder `$49/mo Team` is a guess. After 5 paying customers, run a Van Westendorp price-sensitivity analysis on the next 20 prospects.
- Likely actual price points: Team `$99-199/mo`, Enterprise `$10k-50k/year`.
- Usage-based add-on for high-volume CI: per-million-cycles pricing.

### 9.3 The Wokwi tension
- Wokwi is bigger and more polished today.
- Don't compete head-on on consumer hobbyist. Differentiate on cycle-accuracy + CI + enterprise.
- Partner-not-compete possibility: cross-promote, share customers at different price tiers.

### 9.4 VS Code marketplace presence
- Currently not promoted on landing. The extension already exists (`w1ne.labwired-vscode`).
- Add "Install in VS Code →" CTA to landing + library + CI page.
- VS Code search is a free distribution channel that competes for "STM32 debugger" queries.

---

## 10. Priority stack — what to do in what order

### Week 1 — unblock revenue (Day-1 list)
1. Stripe Checkout + webhook + API-key issuance
2. API-key gating in CLI + GitHub Action
3. Working email + Calendly + domain config
4. Privacy + ToS + cookie banner
5. Plausible analytics on landing + playground

### Week 2 — credibility + sales prep
6. Demo GIF/video on landing hero
7. HIL whitepaper PDF (email-gated)
8. Comparison pages (vs Wokwi, vs Renode)
9. CRM setup + outbound list of 50
10. Discovery script + outbound email script

### Week 3 — execute sales motion
11. Cold-email batch 1 (25 prospects)
12. Book first 3-5 discovery calls
13. Iterate pitch based on call feedback

### Week 4-8 — close first 3-10 customers
14. Hand-onboard each paying customer
15. Extract testimonial / case study from any 1
16. Iterate pricing based on objections
17. Status page + comparison-page SEO maintenance

### Month 3+ — scale + product credibility
18. Customer dashboard
19. SAML / SSO
20. On-prem bundle
21. TQK / ISO 26262 prep (if automotive interest materializes)

---

## 11. Definition of "done" for this plan

This plan is complete when:

- [ ] A real human can pay $49 on the website and receive a working API key by email
- [ ] Their GitHub Action runs against the paid backend, authenticates, and metering data appears
- [ ] Plausible shows visit-to-paid funnel with ≥1 conversion
- [ ] At least 1 named customer is quoted publicly with permission
- [ ] Stripe payments are activated (not test mode)
- [ ] Privacy + ToS + cookie banner live on every public page

After that the plan is "we have a business" — iteration replaces planning.

---

## 12. Risks & open questions

### Risks
1. **Wokwi launches a cycle-accurate STM32 simulator** — their wedge erodes ours. Mitigation: ship CI productization fast, lock in enterprise contracts.
2. **No outbound list works** — embedded teams are notoriously hard to reach via cold email. Backup: paid sponsorship of embedded podcasts/newsletters.
3. **Pricing too low** — `$49/mo` feels cheap for an HIL replacement. Likely needs to be `$199+` for the value prop to land seriously with enterprise.
4. **Pricing too high for self-serve** — embedded hobbyists won't pay $49/mo. Reframe: free playground for hobbyists, paid CI for teams. Already done in the page structure.
5. **Engineering tail** — bugs in the new device library will surface as customers run real firmware. Budget 30% engineering time for bug-fix triage.

### Open questions for the founder (you)
1. Who's writing the cold emails — you alone for 6 months, or hire a part-time SDR after month 3?
2. What's the seed funding situation — are you raising, bootstrapping, or revenue-funded?
3. What's the target ARR for Q4 2026? That sets pricing + customer count math.
4. Are you willing to do support yourself for the first 6 months? (Founder support = best product feedback loop.)
5. Is "labwired.com" the final brand, or is rebranding on the table?

---

## 13. Out of scope (deliberately deferred)

- AI features (resurrect only when 10+ customers ask)
- Mobile native apps
- Multi-tenant collaboration / real-time co-editing
- Hardware-vendor co-marketing deals (engage only after first 5 customers)
- Series A fundraising (revenue-first; raise from strength)
- International expansion beyond English-language EU/US (defer to Q4)
- White-label / OEM licensing (after 20+ direct customers)

---

## 14. Plan ownership & next review

- **Owner:** Andrii (founder)
- **First review:** end of Week 1 (Day-1 list completion check-in)
- **Cadence:** weekly Friday review until first paying customer; biweekly after
- **Where to log progress:** this file's git history + a `STATUS.md` companion if needed

---

**Stop building features. Start having sales conversations.** The product is ready. The business isn't, but every gap above is days-to-weeks of work — not months. Pick the Day-1 list, execute, then talk to customers.
