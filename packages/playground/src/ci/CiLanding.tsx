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
const GITHUB_REPO = 'https://github.com/w1ne/labwired';

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
      {/* Top chrome */}
      <header className="sticky top-0 z-30 h-12 px-6 flex items-center gap-4 bg-[rgba(13,14,18,0.7)] backdrop-blur border-b border-border/60">
        <a href="/playground/" className="flex items-center gap-2 text-fg-primary font-semibold tracking-tight shrink-0">
          <svg viewBox="0 0 20 20" width="18" height="18" aria-hidden="true">
            <path d="M11 2 4 12h4l-1 6 8-10h-4l1-6z" fill="currentColor" />
          </svg>
          LabWired
        </a>
        <span className="text-fg-tertiary text-[11px] hidden md:inline tracking-[0.01em]">
          Deterministic firmware simulation
        </span>
        <div className="flex-1" />
        <nav className="flex items-center gap-4 text-[13px]">
          <a href="/playground/" className="text-fg-secondary hover:text-fg-primary transition-colors duration-150">
            Playground
          </a>
          <a
            href="https://github.com/w1ne/labwired"
            target="_blank"
            rel="noopener noreferrer"
            className="text-fg-secondary hover:text-fg-primary transition-colors duration-150"
          >
            GitHub
          </a>
          <a
            href="#waitlist"
            className="h-7 px-3 rounded-pill text-xs font-medium bg-accent text-bg-base hover:bg-accent-hover transition-colors duration-150 flex items-center"
          >
            Get access
          </a>
        </nav>
      </header>

      {/* Hero */}
      <section className="px-6 pt-24 pb-20 max-w-[1080px] mx-auto">
        <div className="inline-flex items-center gap-2 text-[11px] uppercase tracking-[0.12em] text-magenta font-semibold mb-6">
          <span className="w-1.5 h-1.5 rounded-full bg-magenta animate-pulse" />
          LabWired for CI
        </div>
        <h1 className="text-[44px] md:text-[56px] leading-[1.05] font-bold tracking-tight max-w-[18ch]">
          Replace your HIL bench with deterministic simulation.
        </h1>
        <p className="text-fg-secondary text-[18px] leading-[1.5] mt-6 max-w-[60ch]">
          Run STM32 firmware regression tests on every commit. Cycle-accurate. Reproducible. Parallel. No
          benches, no cables, no flaky tests. <span className="text-fg-primary">$0 per seat.</span>
        </p>

        <div className="flex flex-wrap gap-3 mt-10">
          <a
            href="#waitlist"
            className="h-10 px-5 rounded-pill bg-accent text-bg-base font-semibold hover:bg-accent-hover transition-colors duration-150 flex items-center"
          >
            Request early access →
          </a>
          <a
            href="#how-it-works"
            className="h-10 px-5 rounded-pill bg-white/[0.05] hover:bg-white/[0.10] text-fg-primary font-medium transition-colors duration-150 flex items-center"
          >
            See it run
          </a>
        </div>

        {/* Hero metrics */}
        <div className="grid grid-cols-2 md:grid-cols-4 gap-6 mt-16">
          {[
            { value: '~6,000×', label: 'faster than real-time', note: 'on commodity CI runners' },
            { value: '100%', label: 'deterministic', note: 'identical PC at every cycle' },
            { value: '0 hrs', label: 'rig setup', note: 'YAML manifest, runs immediately' },
            { value: '$0', label: 'free tier', note: 'public repos · unlimited runs while in beta' },
          ].map((m) => (
            <div key={m.label}>
              <div className="text-accent text-[28px] font-bold tracking-tight font-mono">{m.value}</div>
              <div className="text-fg-primary text-[14px] mt-0.5 font-medium">{m.label}</div>
              <div className="text-fg-tertiary text-[12px] mt-1">{m.note}</div>
            </div>
          ))}
        </div>
      </section>

      {/* Why CI section — 3 value props */}
      <section className="px-6 py-20 bg-bg-surface/30">
        <div className="max-w-[1080px] mx-auto">
          <div className="text-[11px] uppercase tracking-[0.12em] text-fg-tertiary font-semibold mb-3">
            Why teams switch
          </div>
          <h2 className="text-[32px] font-bold tracking-tight mb-12 max-w-[18ch]">
            Three problems HIL benches can't solve.
          </h2>
          <div className="grid md:grid-cols-3 gap-8">
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
              <div key={v.title} className="lw-glass p-6">
                <div className="text-3xl mb-3" aria-hidden>{v.icon}</div>
                <h3 className="text-fg-primary font-semibold text-[17px] mb-2">{v.title}</h3>
                <p className="text-fg-secondary text-[14px] leading-[1.55]">{v.body}</p>
              </div>
            ))}
          </div>
        </div>
      </section>

      {/* How it works — GitHub Action snippet */}
      <section id="how-it-works" className="px-6 py-20">
        <div className="max-w-[1080px] mx-auto">
          <div className="text-[11px] uppercase tracking-[0.12em] text-fg-tertiary font-semibold mb-3">
            Drop it in
          </div>
          <h2 className="text-[32px] font-bold tracking-tight mb-3 max-w-[20ch]">
            One YAML file. Zero hardware.
          </h2>
          <p className="text-fg-secondary text-[16px] mb-8 max-w-[60ch]">
            Add the LabWired GitHub Action to any repo with a Rust or C firmware target. Push a commit — see
            the simulation run. Get JUnit XML for your CI dashboard, JSON for your custom tooling.
          </p>

          <div className="lw-glass overflow-hidden">
            <div className="flex items-center justify-between px-4 py-2 border-b border-border bg-bg-elevated/40">
              <span className="text-fg-tertiary text-[11px] font-mono">.github/workflows/firmware.yml</span>
              <button
                type="button"
                onClick={copySnippet}
                className="text-fg-tertiary hover:text-fg-primary text-[11px] font-medium transition-colors duration-150"
              >
                {copyState === 'copied' ? '✓ Copied' : 'Copy'}
              </button>
            </div>
            <pre className="text-[13px] leading-[1.65] p-5 overflow-x-auto font-mono text-fg-secondary">
              <code>{GITHUB_ACTION_SNIPPET}</code>
            </pre>
          </div>

          <div className="mt-6 text-fg-tertiary text-[12px] flex flex-wrap gap-x-6 gap-y-2">
            <span>✓ GitHub Actions</span>
            <span>✓ GitLab CI</span>
            <span>✓ Docker image</span>
            <span>✓ Self-hosted runners</span>
            <span>✓ Native ARM64</span>
          </div>
        </div>
      </section>

      {/* Comparison table */}
      <section className="px-6 py-20 bg-bg-surface/30">
        <div className="max-w-[1080px] mx-auto">
          <div className="text-[11px] uppercase tracking-[0.12em] text-fg-tertiary font-semibold mb-3">
            How we compare
          </div>
          <h2 className="text-[32px] font-bold tracking-tight mb-10 max-w-[22ch]">
            Built for the regression suite that has to ship.
          </h2>

          <div className="overflow-x-auto">
            <table className="w-full text-[13px]">
              <thead>
                <tr className="border-b border-border">
                  <th className="text-left py-3 pr-4 text-fg-tertiary font-medium uppercase tracking-wider text-[11px]"></th>
                  <th className="text-left py-3 px-4 text-fg-primary font-semibold">
                    <span className="text-accent">LabWired CI</span>
                  </th>
                  <th className="text-left py-3 px-4 text-fg-secondary font-medium">Wokwi CI</th>
                  <th className="text-left py-3 px-4 text-fg-secondary font-medium">Renode</th>
                  <th className="text-left py-3 px-4 text-fg-secondary font-medium">HIL bench</th>
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
                ].map(([row, lw, wk, ren, hil], i) => (
                  <tr key={i} className="border-b border-border/40">
                    <td className="py-3 pr-4 text-fg-secondary font-sans">{row}</td>
                    <td className="py-3 px-4 text-accent font-semibold">{lw}</td>
                    <td className="py-3 px-4 text-fg-secondary">{wk}</td>
                    <td className="py-3 px-4 text-fg-secondary">{ren}</td>
                    <td className="py-3 px-4 text-fg-secondary">{hil}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>

          <p className="text-fg-tertiary text-[12px] mt-4">
            Wokwi is great for prototyping and IoT. Renode is best-in-class for low-level peripheral fidelity
            on the desktop. Our wedge: <span className="text-fg-primary">cycle-accurate STM32 with a
            zero-setup browser playground and drop-in CI</span>. The right answer for embedded teams
            shipping STM32-based products.
          </p>
        </div>
      </section>

      {/* Pricing */}
      <section className="px-6 py-20">
        <div className="max-w-[1080px] mx-auto">
          <div className="text-[11px] uppercase tracking-[0.12em] text-fg-tertiary font-semibold mb-3">
            Pricing
          </div>
          <h2 className="text-[32px] font-bold tracking-tight mb-3 max-w-[22ch]">
            Pricing that scales with you.
          </h2>
          <p className="text-fg-secondary text-[16px] mb-12 max-w-[60ch]">
            Free for public repos. <span className="text-fg-primary font-semibold">Designer at $5/seat/month</span> for solo
            tinkerers who want privacy. <span className="text-fg-primary font-semibold">Pro at $19/seat/month</span> for
            teams shipping firmware in CI. Enterprise contracts for SAML, on-prem, and compliance evidence.
          </p>

          <div className="grid md:grid-cols-2 lg:grid-cols-4 gap-6">
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
                    ? 'lw-glass p-6 ring-2 ring-accent/40 relative'
                    : 'lw-glass p-6'
                }
              >
                {tier.highlighted && (
                  <div className="absolute -top-3 left-6 text-[10px] uppercase tracking-[0.1em] bg-accent text-bg-base px-2 py-0.5 rounded font-semibold">
                    Most popular
                  </div>
                )}
                <div className="text-fg-tertiary text-[11px] uppercase tracking-[0.1em] font-semibold mb-3">
                  {tier.name}
                </div>
                <div className="text-fg-primary text-[28px] font-bold tracking-tight">{tier.price}</div>
                <div className="text-fg-tertiary text-[12px] mb-5">{tier.priceNote}</div>
                <ul className="space-y-2 mb-6">
                  {tier.features.map((f) => (
                    <li key={f} className="flex items-start gap-2 text-fg-secondary text-[13px]">
                      <span className="text-ok mt-0.5" aria-hidden>✓</span>
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
                      ? 'block text-center h-10 leading-[2.5rem] rounded-pill bg-accent text-bg-base font-semibold hover:bg-accent-hover transition-colors duration-150'
                      : 'block text-center h-10 leading-[2.5rem] rounded-pill bg-white/[0.05] hover:bg-white/[0.10] text-fg-primary font-medium transition-colors duration-150'
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

      {/* Waitlist CTA */}
      <section id="waitlist" className="px-6 py-24 bg-bg-surface/30">
        <div className="max-w-[640px] mx-auto text-center">
          <h2 className="text-[36px] font-bold tracking-tight mb-4">
            Early access. Real teams. Real bugs.
          </h2>
          <p className="text-fg-secondary text-[16px] mb-8">
            We're onboarding 20 embedded teams to the closed beta. Drop your email and we'll get you a
            workspace + onboarding call within 48 hours.
          </p>
          {submitted ? (
            <div className="lw-glass p-6 text-fg-primary">
              <div className="text-ok text-2xl mb-2">✓ Thanks!</div>
              <p className="text-fg-secondary">
                Your mail client should be open. If not, write us at{' '}
                <a className="text-accent" href="mailto:andrii@shylenko.com">
                  andrii@shylenko.com
                </a>
                .
              </p>
            </div>
          ) : (
            <form onSubmit={submitWaitlist} className="flex flex-col sm:flex-row gap-2 max-w-[480px] mx-auto">
              <input
                type="email"
                required
                value={email}
                onChange={(event) => setEmail(event.target.value)}
                placeholder="you@yourcompany.com"
                className="flex-1 h-12 px-4 rounded-pill bg-bg-surface border border-border text-fg-primary placeholder:text-fg-tertiary outline-none focus:border-accent text-[15px]"
              />
              <button
                type="submit"
                className="h-12 px-6 rounded-pill bg-accent text-bg-base font-semibold hover:bg-accent-hover transition-colors duration-150"
              >
                Request access
              </button>
            </form>
          )}
          <div className="text-fg-tertiary text-[11px] mt-4">
            Or open-source it today on{' '}
            <a
              className="text-accent hover:underline"
              href="https://github.com/w1ne/labwired"
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
      <footer className="px-6 py-10 border-t border-border/60">
        <div className="max-w-[1080px] mx-auto flex flex-wrap items-center justify-between gap-4 text-[12px] text-fg-tertiary">
          <div className="flex items-center gap-2">
            <svg viewBox="0 0 20 20" width="14" height="14" aria-hidden>
              <path d="M11 2 4 12h4l-1 6 8-10h-4l1-6z" fill="currentColor" />
            </svg>
            <span>LabWired · Deterministic firmware simulation</span>
          </div>
          <div className="flex items-center gap-5">
            <a className="hover:text-fg-primary transition-colors" href="/playground/">Playground</a>
            <a className="hover:text-fg-primary transition-colors" href="https://github.com/w1ne/labwired" target="_blank" rel="noopener noreferrer">GitHub</a>
            <a className="hover:text-fg-primary transition-colors" href="mailto:andrii@shylenko.com">Contact</a>
          </div>
        </div>
      </footer>
    </div>
  );
}
