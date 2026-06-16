import { GlobalLogo, GlobalNav } from '../components/GlobalNav';
import { GlobalFooter } from '../components/GlobalFooter';
import { ValidationMatrix } from '../ValidationMatrix';

export function ValidationLanding() {
  return (
    <div className="min-h-screen bg-bg-base text-fg-primary font-sans">
      {/* Sticky chrome — mirrors CiLanding */}
      <header className="lw-chrome">
        <GlobalLogo />
        <span className="text-fg-tertiary text-[12px] hidden md:inline tracking-[0.01em]">
          Deterministic firmware simulation
        </span>
        <div className="flex-1" />
        <GlobalNav active="validation" />
      </header>

      {/* Hero */}
      <section className="px-6 pt-24 pb-16 max-w-[1120px] mx-auto">
        <div className="lw-kicker-pill mb-6">
          <span className="lw-kicker-dot" />
          Tier-1 Validation
        </div>
        <h1 className="text-[44px] md:text-[60px] leading-[1.05] font-bold tracking-tight max-w-[22ch] text-fg-primary">
          Every peripheral, every chip.{' '}
          <span className="text-accent">Proven in CI.</span>
        </h1>
        <p className="text-fg-secondary text-[19px] leading-[1.5] mt-6 max-w-[60ch]">
          The table below is the public audit trail for LabWired&rsquo;s Tier-1
          chip matrix. Each cell links the exact CI run that produced the result —
          refreshed every night from real firmware on labwired-core.{' '}
          <span className="text-fg-primary font-semibold">No link, no claim.</span>
        </p>
      </section>

      {/* Matrix section */}
      <section className="lw-section-bg px-6 py-20">
        <div className="max-w-[1120px] mx-auto">
          <ValidationMatrix />
        </div>
      </section>

      {/* Footer */}
      <GlobalFooter />
    </div>
  );
}
