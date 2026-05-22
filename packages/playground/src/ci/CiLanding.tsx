import { useState } from 'react';
import { useUser } from '@clerk/clerk-react';
import { buildStripeUpgradeUrl } from '../studio/stripeUpgrade';

// ──────────────────────────────────────────────────────────────────────────
// External services — replace these placeholders with your real account URLs.
// Stripe Payment Link lives in ../studio/stripeUpgrade.ts so the cabinet and
// the marketing page agree on one URL (plus Clerk-aware query params).
//   • Calendly             → personal link at https://calendly.com/<you>/intro
//   • Formspree            → free 50/mo form endpoint, or replace with mailto:
// ──────────────────────────────────────────────────────────────────────────
const CALENDLY_ENTERPRISE = 'https://calendly.com/labwired/enterprise-intro';
const WAITLIST_FORM_ACTION = 'mailto:andrii@shylenko.com'; // swap for https://formspree.io/f/<id> when ready
const GITHUB_REPO = 'https://github.com/w1ne/labwired-core';

const GITHUB_ACTION_SNIPPET = `name: Firmware Regression
on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Build firmware
        run: cargo build --release --target thumbv7m-none-eabi

      - name: Run LabWired simulation
        uses: w1ne/labwired/.github/actions/labwired-test@main
        with:
          script: tests/firmware-regression.yaml
          output_dir: test-results
        env:
          LABWIRED_API_KEY: \${{ secrets.LABWIRED_API_KEY }}

      - name: Upload artifacts
        if: always()
        uses: actions/upload-artifact@v4
        with:
          name: labwired-results
          path: test-results/`;

// Feature flag: render the Designer tier card only when the Stripe product is live.
// Set VITE_DESIGNER_TIER_ENABLED=true in packages/playground/.env.local after creating it.
const DESIGNER_TIER_ENABLED = import.meta.env.VITE_DESIGNER_TIER_ENABLED === 'true';

export function CiLanding() {
  const [email, setEmail] = useState('');
  const [submitted, setSubmitted] = useState(false);
  const [copyState, setCopyState] = useState<'idle' | 'copied'>('idle');
  const { isSignedIn, user } = useUser();
  const stripeProUrl = buildStripeUpgradeUrl({
    tier: 'pro',
    clerkUserId: isSignedIn ? user?.id : undefined,
    email: isSignedIn ? user?.primaryEmailAddress?.emailAddress : undefined,
  });
  const stripeDesignerUrl = buildStripeUpgradeUrl({
    tier: 'designer',
    clerkUserId: isSignedIn ? user?.id : undefined,
    email: isSignedIn ? user?.primaryEmailAddress?.emailAddress : undefined,
  });

  const copySnippet = async () => {
    await navigator.clipboard.writeText(GITHUB_ACTION_SNIPPET);
    setCopyState('copied');
    setTimeout(() => setCopyState('idle'), 2000);
  };

  const submitWaitlist = (event: React.FormEvent) => {
    event.preventDefault();
    if (!email) return;
    if (WAITLIST_FORM_ACTION.startsWith('mailto:')) {
      const subject = encodeURIComponent('LabWired CI waitlist');
      const body = encodeURIComponent(
        `I'd like early access to LabWired CI.\n\nEmail: ${email}\n\nWhat I'm hoping to use it for:\n`,
      );
      window.location.href = `${WAITLIST_FORM_ACTION}?subject=${subject}&body=${body}`;
    } else {
      // For Formspree, Tally, ConvertKit etc. — POST and let the service redirect.
      const form = document.createElement('form');
      form.action = WAITLIST_FORM_ACTION;
      form.method = 'POST';
      form.target = '_self';
      const emailField = document.createElement('input');
      emailField.name = 'email';
      emailField.value = email;
      form.appendChild(emailField);
      const sourceField = document.createElement('input');
      sourceField.name = 'source';
      sourceField.value = 'ci-landing';
      form.appendChild(sourceField);
      document.body.appendChild(form);
      form.submit();
    }
    setSubmitted(true);
  };

  return (
    <div className="min-h-screen bg-bg-base text-fg-primary font-sans">
      {/* Top chrome — sticky, translucent white, hard bottom border */}
      <header className="lw-chrome">
        <a href="https://labwired.com" className="flex items-center gap-2 text-fg-primary font-bold tracking-tight shrink-0" title="LabWired home">
          <svg viewBox="0 0 20 20" width="18" height="18" aria-hidden="true">
            <path d="M11 2 4 12h4l-1 6 8-10h-4l1-6z" fill="currentColor" />
          </svg>
          LabWired
        </a>
        <span className="text-fg-tertiary text-[12px] hidden md:inline tracking-[0.01em]">
          Deterministic firmware simulation
        </span>
        <div className="flex-1" />
        <nav className="flex items-center gap-5 text-[14px]">
          <a href="/" className="text-fg-secondary hover:text-fg-primary font-medium transition-colors duration-150">Playground</a>
          <a href="library.html" className="text-fg-secondary hover:text-fg-primary font-medium transition-colors duration-150">Library</a>
          <a href="ci.html" aria-current="page" className="text-fg-primary font-semibold transition-colors duration-150">For CI</a>
          <a
            href="https://github.com/w1ne/labwired-core"
            target="_blank"
            rel="noopener noreferrer"
            className="text-fg-secondary hover:text-fg-primary font-medium transition-colors duration-150"
          >
            GitHub
          </a>
        </nav>
      </header>

      {/* Hero */}
      <section className="px-6 pt-24 pb-20 max-w-[1120px] mx-auto">
        <div className="lw-kicker-pill mb-6">
          <span className="lw-kicker-dot" />
          LabWired for CI
        </div>
        <h1 className="text-[44px] md:text-[60px] leading-[1.05] font-bold tracking-tight max-w-[18ch] text-fg-primary">
          Replace your HIL bench with{' '}
          <span className="text-accent">deterministic simulation.</span>
        </h1>
        <p className="text-fg-secondary text-[19px] leading-[1.5] mt-6 max-w-[60ch]">
          Run STM32 firmware regression tests on every commit. Cycle-accurate. Reproducible. Parallel. No
          benches, no cables, no flaky tests. <span className="text-fg-primary font-semibold">$0 per seat.</span>
        </p>

        <div className="flex flex-wrap gap-3.5 mt-10">
          <a href="#waitlist" className="lw-cta-primary">
            Request early access &rarr;
          </a>
          <a href="#how-it-works" className="lw-cta-secondary">
            See it run
          </a>
        </div>

        {/* Hero metrics — brutalist white cards with hard shadow */}
        <div className="grid grid-cols-2 md:grid-cols-4 gap-6 mt-16">
          {[
            { value: '~6,000×', label: 'faster than real-time', note: 'on commodity CI runners' },
            { value: '100%', label: 'deterministic', note: 'identical PC at every cycle' },
            { value: '0 hrs', label: 'rig setup', note: 'YAML manifest, runs immediately' },
            { value: '$0', label: 'free tier', note: 'public repos · unlimited runs while in beta' },
          ].map((m) => (
            <div
              key={m.label}
              className="bg-white border-2 border-[#1a1a1a] rounded-[10px] p-5 shadow-[3px_3px_0_#1a1a1a]"
            >
              <div className="text-accent text-[28px] font-bold tracking-tight font-mono">{m.value}</div>
              <div className="text-fg-primary text-[14px] mt-1 font-semibold">{m.label}</div>
              <div className="text-fg-tertiary text-[12px] mt-1">{m.note}</div>
            </div>
          ))}
        </div>
      </section>

      {/* Why CI section — 3 value props */}
      <section className="lw-section-bg px-6 py-24">
        <div className="max-w-[1120px] mx-auto">
          <div className="text-[11px] uppercase tracking-[0.14em] text-fg-tertiary font-bold mb-3">
            Why teams switch
          </div>
          <h2 className="text-[32px] md:text-[40px] font-bold tracking-tight mb-12 max-w-[20ch] text-fg-primary">
            Three problems HIL benches can't solve.
          </h2>
          <div className="grid md:grid-cols-3 gap-7">
            {[
              {
                icon: '🎯',
                title: 'Deterministic',
                body: 'Same firmware, same cycle-exact result every run. No cable jitter, no power noise, no "ghost bug" race conditions that vanish under the logic analyzer. Bugs reproduce in CI, not just on Friday afternoons.',
              },
              {
                icon: '⚡',
                title: 'Parallel & cheap',
                body: '~6,000× wall-clock speedup means a 30-minute regression suite runs in seconds. Spawn 50 concurrent jobs across hardware variants. No queueing for the one rig in the lab, no $50k capex per seat.',
              },
              {
                icon: '🔬',
                title: 'Observable',
                body: 'Every run produces a JSON result + VCD trace + UART log + cycle-by-cycle PC history. Attach artifacts to bug reports. Diff traces between commits to find regressions instantly. Logic analyzer-grade visibility, in CI.',
              },
            ].map((v) => (
              <div
                key={v.title}
                className="bg-white border-2 border-[#1a1a1a] rounded-[10px] p-7 shadow-[5px_5px_0_#1a1a1a] transition-all duration-150 hover:-translate-x-[2px] hover:-translate-y-[2px] hover:shadow-[7px_7px_0_#1a1a1a]"
              >
                <div className="text-3xl mb-3" aria-hidden>{v.icon}</div>
                <h3 className="text-fg-primary font-bold text-[18px] mb-2">{v.title}</h3>
                <p className="text-fg-secondary text-[14.5px] leading-[1.55]">{v.body}</p>
              </div>
            ))}
          </div>
        </div>
      </section>

      {/* How it works — GitHub Action snippet */}
      <section id="how-it-works" className="px-6 py-24">
        <div className="max-w-[1120px] mx-auto">
          <div className="text-[11px] uppercase tracking-[0.14em] text-fg-tertiary font-bold mb-3">
            Drop it in
          </div>
          <h2 className="text-[32px] md:text-[40px] font-bold tracking-tight mb-3 max-w-[20ch] text-fg-primary">
            One YAML file. Zero hardware.
          </h2>
          <p className="text-fg-secondary text-[16px] mb-10 max-w-[60ch]">
            Add the LabWired GitHub Action to any repo with a Rust or C firmware target. Push a commit — see
            the simulation run. Get JUnit XML for your CI dashboard, JSON for your custom tooling.
          </p>

          <div className="lw-snippet">
            <div className="lw-snippet-header">
              <span className="text-[#9098a8] text-[11px] font-mono">
                .github/workflows/firmware.yml
              </span>
              <button
                type="button"
                onClick={copySnippet}
                className="text-[#9098a8] hover:text-white text-[11px] font-semibold transition-colors duration-150"
              >
                {copyState === 'copied' ? '✓ Copied' : 'Copy'}
              </button>
            </div>
            <pre>
              <code>{GITHUB_ACTION_SNIPPET}</code>
            </pre>
          </div>

          <div className="mt-6 text-fg-tertiary text-[13px] flex flex-wrap gap-x-6 gap-y-2">
            <span>✓ GitHub Actions</span>
            <span>✓ GitLab CI</span>
            <span>✓ Docker image</span>
            <span>✓ Self-hosted runners</span>
            <span>✓ Native ARM64</span>
          </div>
        </div>
      </section>

      {/* Comparison table */}
      <section className="lw-section-bg px-6 py-24">
        <div className="max-w-[1120px] mx-auto">
          <div className="text-[11px] uppercase tracking-[0.14em] text-fg-tertiary font-bold mb-3">
            How we compare
          </div>
          <h2 className="text-[32px] md:text-[40px] font-bold tracking-tight mb-10 max-w-[22ch] text-fg-primary">
            Built for the regression suite that has to ship.
          </h2>

          <div className="bg-white border-2 border-[#1a1a1a] rounded-[10px] shadow-[5px_5px_0_#1a1a1a] overflow-x-auto">
            <table className="w-full text-[13px]">
              <thead>
                <tr className="border-b-2 border-[#1a1a1a] bg-[#f8f9fa]">
                  <th className="text-left py-3 px-4 text-fg-tertiary font-bold uppercase tracking-wider text-[11px]"></th>
                  <th className="text-left py-3 px-4 text-fg-primary font-bold">
                    <span className="text-accent">LabWired CI</span>
                  </th>
                  <th className="text-left py-3 px-4 text-fg-secondary font-semibold">Wokwi CI</th>
                  <th className="text-left py-3 px-4 text-fg-secondary font-semibold">Renode</th>
                  <th className="text-left py-3 px-4 text-fg-secondary font-semibold">HIL bench</th>
                </tr>
              </thead>
              <tbody className="font-mono text-[12.5px]">
                {[
                  ['STM32 cycle-accurate', '✓', '~', '✓', '✓'],
                  ['VCD trace per run', '✓', '—', '✓', '~'],
                  ['Parallel concurrency', 'unlimited', 'per plan', 'self-host', '1 per bench'],
                  ['Setup time', '< 1 s', '< 1 s', 'hours', '2–4 hours'],
                  ['Per-seat capex', '$0', '$0', '$0', '$10k–$100k'],
                  ['Determinism guarantee', 'cycle-exact', 'best-effort', 'cycle-exact', 'flaky'],
                  ['VS Code timeline', '✓', '✓', '✓', '—'],
                  ['Fault injection', 'roadmap', '—', '✓', 'manual'],
                  ['Open-source core', '✓', '—', '✓', 'n/a'],
                ].map(([row, lw, wk, ren, hil], i, arr) => (
                  <tr
                    key={i}
                    className={i === arr.length - 1 ? '' : 'border-b border-[#d6d8dc]'}
                  >
                    <td className="py-3 px-4 text-fg-secondary font-sans font-semibold">{row}</td>
                    <td className="py-3 px-4 text-accent font-bold">{lw}</td>
                    <td className="py-3 px-4 text-fg-secondary">{wk}</td>
                    <td className="py-3 px-4 text-fg-secondary">{ren}</td>
                    <td className="py-3 px-4 text-fg-secondary">{hil}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>

          <p className="text-fg-tertiary text-[13px] mt-5 max-w-[80ch]">
            Wokwi is great for prototyping and IoT. Renode is best-in-class for low-level peripheral fidelity
            on the desktop. Our wedge: <span className="text-fg-primary font-semibold">cycle-accurate STM32 with a
            zero-setup browser playground and drop-in CI</span>. The right answer for embedded teams
            shipping STM32-based products.
          </p>
        </div>
      </section>

      {/* Pricing */}
      <section className="px-6 py-24">
        <div className="max-w-[1120px] mx-auto">
          <div className="text-[11px] uppercase tracking-[0.14em] text-fg-tertiary font-bold mb-3">
            Pricing
          </div>
          <h2 className="text-[32px] md:text-[40px] font-bold tracking-tight mb-3 max-w-[22ch] text-fg-primary">
            Pricing that scales with you.
          </h2>
          <p className="text-fg-secondary text-[16px] mb-12 max-w-[60ch]">
            Free for public repos.{' '}
            {DESIGNER_TIER_ENABLED && (
              <>
                <span className="text-fg-primary font-semibold">Designer at $5/seat/month</span> for solo
                tinkerers who want privacy.{' '}
              </>
            )}
            <span className="text-fg-primary font-semibold">Pro at $19/seat/month</span> for
            teams shipping firmware in CI. Enterprise contracts for SAML, on-prem, and compliance evidence.
          </p>

          <div className={`grid md:grid-cols-2 ${DESIGNER_TIER_ENABLED ? 'lg:grid-cols-4' : 'lg:grid-cols-3'} gap-7`}>
            {[
              {
                name: 'Open Source',
                price: 'Free',
                priceNote: 'forever',
                features: [
                  'Public repos',
                  'Up to 10k cycles / run',
                  '~1k runs / month',
                  'JSON + JUnit artifacts',
                  'Community Discord',
                ],
                cta: 'Use today',
                ctaHref: GITHUB_REPO,
                ctaExternal: true,
              },
              ...(DESIGNER_TIER_ENABLED
                ? [
                    {
                      name: 'Designer',
                      price: '$5',
                      priceNote: 'per seat · per month · cancel anytime',
                      features: [
                        'Private projects (coming soon)',
                        '10M cycles / month included',
                        'Save / share / fork in browser',
                        'Community Discord access',
                        'All future Designer updates',
                      ],
                      cta: 'Start with Designer →',
                      ctaHref: stripeDesignerUrl,
                      ctaExternal: true,
                      hint: !isSignedIn
                        ? 'Sign in first so your workspace links to your account after checkout.'
                        : undefined,
                    },
                  ]
                : []),
              {
                name: 'Pro',
                price: '$19',
                priceNote: 'per seat · per month · cancel anytime',
                features: [
                  'Private projects',
                  '100M cycles / month included',
                  'Priority email support',
                  'VCD trace retention 30 days',
                  'All future updates',
                ],
                cta: 'Start with Pro →',
                ctaHref: stripeProUrl,
                ctaExternal: true,
                highlighted: true,
                hint: !isSignedIn
                  ? 'Sign in first so your API key shows up in the playground after checkout.'
                  : undefined,
              },
              {
                name: 'Enterprise',
                price: 'Custom',
                priceNote: 'annual contract',
                features: [
                  'Everything in Pro',
                  'SAML / SSO',
                  'On-prem / self-hosted',
                  'ISO 26262 evidence kit',
                  'Dedicated SLA',
                  'Tool qualification (TQK)',
                ],
                cta: 'Book a call',
                ctaHref: CALENDLY_ENTERPRISE,
                ctaExternal: true,
              },
            ].map((tier) => (
              <div
                key={tier.name}
                className={
                  tier.highlighted
                    ? 'relative bg-white border-2 border-[#0056b3] rounded-[10px] p-7 shadow-[5px_5px_0_#0056b3] transition-all duration-150 hover:-translate-x-[2px] hover:-translate-y-[2px] hover:shadow-[7px_7px_0_#0056b3]'
                    : 'relative bg-white border-2 border-[#1a1a1a] rounded-[10px] p-7 shadow-[5px_5px_0_#1a1a1a] transition-all duration-150 hover:-translate-x-[2px] hover:-translate-y-[2px] hover:shadow-[7px_7px_0_#1a1a1a]'
                }
              >
                {tier.highlighted && (
                  <div className="absolute -top-3 left-6 text-[10px] uppercase tracking-[0.12em] bg-[#0056b3] text-white px-2.5 py-1 rounded-pill font-bold border-2 border-[#1a1a1a]">
                    Most popular
                  </div>
                )}
                <div className="text-fg-tertiary text-[11px] uppercase tracking-[0.14em] font-bold mb-3">
                  {tier.name}
                </div>
                <div className="text-fg-primary text-[32px] font-bold tracking-tight">{tier.price}</div>
                <div className="text-fg-tertiary text-[12px] mb-5">{tier.priceNote}</div>
                <ul className="space-y-2 mb-7">
                  {tier.features.map((f) => (
                    <li key={f} className="flex items-start gap-2 text-fg-secondary text-[13.5px]">
                      <span className="text-accent mt-0.5 font-bold" aria-hidden>✓</span>
                      {f}
                    </li>
                  ))}
                </ul>
                <a
                  href={tier.ctaHref}
                  target={tier.ctaExternal ? '_blank' : undefined}
                  rel={tier.ctaExternal ? 'noopener noreferrer' : undefined}
                  className={
                    tier.highlighted
                      ? 'block text-center py-3 rounded-pill bg-[#0056b3] text-white font-semibold border-2 border-[#1a1a1a] shadow-[3px_3px_0_#1a1a1a] hover:bg-[#004494] hover:-translate-x-[1px] hover:-translate-y-[1px] hover:shadow-[4px_4px_0_#1a1a1a] transition-all duration-150'
                      : 'block text-center py-3 rounded-pill bg-white text-fg-primary font-semibold border-2 border-[#1a1a1a] shadow-[3px_3px_0_#1a1a1a] hover:-translate-x-[1px] hover:-translate-y-[1px] hover:shadow-[4px_4px_0_#1a1a1a] transition-all duration-150'
                  }
                >
                  {tier.cta}
                </a>
                {'hint' in tier && tier.hint && (
                  <p className="mt-3 text-fg-tertiary text-[11px] text-center">
                    {tier.hint}
                  </p>
                )}
              </div>
            ))}
          </div>
        </div>
      </section>

      {/* Pricing */}
      <section id="pricing" className="px-6 py-24">
        <div className="max-w-[1120px] mx-auto">
          <div className="lw-kicker-pill mb-6">
            <span className="lw-kicker-dot" />
            Pricing
          </div>
          <h2 className="text-[32px] md:text-[40px] font-bold tracking-tight mb-4 text-fg-primary max-w-[20ch]">
            Simulator's free. Wiring it into your CI is where we earn our keep.
          </h2>
          <p className="text-fg-secondary text-[18px] leading-[1.5] max-w-[58ch] mb-12">
            Start with the open-source CLI on your own GitHub Actions runner. Bring us in when you
            want a custom firmware build pipeline, custom assertions, hosted sim runs, or a real
            engineer staring at the regression with you.
          </p>
          <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
            {/* Open Source tier */}
            <div className="border-2 border-border rounded-2xl p-8 bg-bg-base flex flex-col">
              <div className="flex items-baseline justify-between mb-2">
                <h3 className="text-[22px] font-bold text-fg-primary">Open Source</h3>
                <span className="text-fg-tertiary text-[13px]">MIT licensed</span>
              </div>
              <div className="text-[40px] font-bold tracking-tight text-fg-primary mb-1">$0</div>
              <p className="text-fg-secondary text-[14px] mb-6">Self-host. Run forever.</p>
              <ul className="space-y-3 text-fg-secondary text-[14px] mb-8 flex-1">
                <li className="flex gap-2"><span className="text-success">✓</span> The full deterministic simulator (Xtensa LX6, ARM Cortex-M, RISC-V)</li>
                <li className="flex gap-2"><span className="text-success">✓</span> <code className="text-[13px] bg-bg-surface px-1.5 py-0.5 rounded">labwired-cli</code> for local + CI runs</li>
                <li className="flex gap-2"><span className="text-success">✓</span> The <code className="text-[13px] bg-bg-surface px-1.5 py-0.5 rounded">labwired-lab-template</code> GitHub repo template</li>
                <li className="flex gap-2"><span className="text-success">✓</span> Every built-in lab board (Blinky, e-paper, OLED, IMUs, GPS…)</li>
                <li className="flex gap-2"><span className="text-success">✓</span> Community support via GitHub Issues</li>
              </ul>
              <a
                href="https://github.com/w1ne/labwired-core"
                target="_blank"
                rel="noopener noreferrer"
                className="inline-flex items-center justify-center px-4 py-2.5 rounded-pill text-[14px] font-semibold bg-bg-surface text-fg-primary border border-border hover:border-fg-secondary transition-colors duration-150"
              >
                Get it on GitHub
              </a>
            </div>
            {/* Custom CI tier */}
            <div className="border-2 border-fg-primary rounded-2xl p-8 bg-fg-primary text-bg-base flex flex-col relative">
              <span className="absolute -top-3 right-6 px-2.5 py-1 rounded-pill text-[11px] font-semibold bg-accent text-bg-base uppercase tracking-wider">Most teams</span>
              <div className="flex items-baseline justify-between mb-2">
                <h3 className="text-[22px] font-bold text-bg-base">Custom CI</h3>
                <span className="text-bg-base/60 text-[13px]">Per engagement</span>
              </div>
              <div className="text-[40px] font-bold tracking-tight text-bg-base mb-1">Talk to us</div>
              <p className="text-bg-base/70 text-[14px] mb-6">Your repo. Your firmware. Your green check.</p>
              <ul className="space-y-3 text-bg-base/80 text-[14px] mb-8 flex-1">
                <li className="flex gap-2"><span className="text-accent">✓</span> We wire LabWired CI into your existing GitHub repo, custom firmware build steps, custom assertions</li>
                <li className="flex gap-2"><span className="text-accent">✓</span> Bring-up for your chip / board if it's not in the built-in library yet</li>
                <li className="flex gap-2"><span className="text-accent">✓</span> A real engineer on-call during your regression hunt</li>
                <li className="flex gap-2"><span className="text-accent">✓</span> Hosted simulation runs (priority queue, persistent artifacts)</li>
                <li className="flex gap-2"><span className="text-accent">✓</span> Direct Slack / email channel with the LabWired team</li>
              </ul>
              <a
                href={CALENDLY_ENTERPRISE}
                target="_blank"
                rel="noopener noreferrer"
                className="inline-flex items-center justify-center px-4 py-2.5 rounded-pill text-[14px] font-semibold bg-accent text-bg-base hover:bg-accent-hover transition-colors duration-150"
              >
                Book 30 min →
              </a>
            </div>
          </div>
        </div>
      </section>

      {/* Waitlist CTA */}
      <section id="waitlist" className="lw-section-bg px-6 py-28">
        <div className="max-w-[640px] mx-auto text-center">
          <h2 className="text-[32px] md:text-[40px] font-bold tracking-tight mb-4 text-fg-primary">
            Early access. Real teams. Real bugs.
          </h2>
          <p className="text-fg-secondary text-[16px] mb-10">
            We're onboarding 20 embedded teams to the closed beta. Drop your email and we'll get you a
            workspace + onboarding call within 48 hours.
          </p>
          {submitted ? (
            <div className="bg-white border-2 border-[#1a1a1a] rounded-[10px] p-7 shadow-[5px_5px_0_#1a1a1a] text-fg-primary">
              <div className="text-accent text-2xl mb-2 font-bold">✓ Thanks!</div>
              <p className="text-fg-secondary">
                Your mail client should be open. If not, write us at{' '}
                <a className="text-accent font-semibold underline" href="mailto:andrii@shylenko.com">
                  andrii@shylenko.com
                </a>
                .
              </p>
            </div>
          ) : (
            <form onSubmit={submitWaitlist} className="flex flex-col sm:flex-row gap-3 max-w-[480px] mx-auto">
              <input
                type="email"
                required
                value={email}
                onChange={(event) => setEmail(event.target.value)}
                placeholder="you@yourcompany.com"
                className="lw-input flex-1"
              />
              <button type="submit" className="lw-cta-primary">
                Request access
              </button>
            </form>
          )}
          <div className="text-fg-tertiary text-[12px] mt-5">
            Or open-source it today on{' '}
            <a
              className="text-accent font-semibold hover:underline"
              href="https://github.com/w1ne/labwired-core"
              target="_blank"
              rel="noopener noreferrer"
            >
              GitHub
            </a>
            .
          </div>
        </div>
      </section>

      {/* Footer */}
      <footer className="px-6 py-10 border-t-2 border-[#1a1a1a] bg-white">
        <div className="max-w-[1120px] mx-auto flex flex-wrap items-center justify-between gap-4 text-[13px] text-fg-tertiary">
          <div className="flex items-center gap-2 font-semibold">
            <svg viewBox="0 0 20 20" width="14" height="14" aria-hidden>
              <path d="M11 2 4 12h4l-1 6 8-10h-4l1-6z" fill="currentColor" />
            </svg>
            <span>LabWired · Deterministic firmware simulation</span>
          </div>
          <div className="flex items-center gap-5">
            <a className="text-fg-secondary font-medium hover:text-fg-primary transition-colors" href="/">Playground</a>
            <a className="text-fg-secondary font-medium hover:text-fg-primary transition-colors" href="https://github.com/w1ne/labwired-core" target="_blank" rel="noopener noreferrer">GitHub</a>
            <a className="text-fg-secondary font-medium hover:text-fg-primary transition-colors" href="mailto:andrii@shylenko.com">Contact</a>
          </div>
        </div>
      </footer>
    </div>
  );
}
