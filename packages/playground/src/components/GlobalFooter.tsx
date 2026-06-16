// Single source of truth for the marketing/landing footer. Previously each
// landing page (Library, CI, Validation) hand-rolled the same <footer> markup,
// which drifted — the Library copy used relative links (./, ci.html) that
// resolve wrong off the /library route. Every page now renders <GlobalFooter />
// so the brand line and links stay in lockstep, like <GlobalNav />.
import { LABWIRED_HOME_URL } from './GlobalNav';

const LINK_CLASS =
  'text-fg-secondary font-medium hover:text-fg-primary transition-colors';

// Absolute hrefs so they resolve identically from any route (/, /library.html,
// /validation.html, ...). Marketing surfaces live on labwired.com.
const FOOTER_LINKS = [
  { label: 'Playground', href: '/' },
  { label: 'For CI', href: `${LABWIRED_HOME_URL}/ci.html`, external: true },
  {
    label: 'GitHub',
    href: 'https://github.com/w1ne/labwired-core',
    external: true,
  },
  { label: 'Contact', href: 'mailto:contact@labwired.com' },
];

export function GlobalFooter() {
  return (
    <footer className="px-6 py-10 border-t-2 border-[#1a1a1a] bg-white">
      <div className="max-w-[1120px] mx-auto flex flex-wrap items-center justify-between gap-4 text-[13px] text-fg-tertiary">
        <div className="flex items-center gap-2 font-semibold">
          <svg viewBox="0 0 32 32" width="16" height="16" fill="none" aria-hidden>
            <path d="M11 7V23H23" stroke="#0056b3" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round" />
            <circle cx="11" cy="7" r="3" fill="#1a1a1a" />
            <circle cx="23" cy="23" r="3" fill="#0056b3" />
          </svg>
          <span>LabWired · Deterministic firmware simulation</span>
        </div>
        <div className="flex items-center gap-5">
          {FOOTER_LINKS.map((link) => (
            <a
              key={link.label}
              className={LINK_CLASS}
              href={link.href}
              {...(link.external ? { target: '_blank', rel: 'noopener noreferrer' } : {})}
            >
              {link.label}
            </a>
          ))}
        </div>
      </div>
    </footer>
  );
}
