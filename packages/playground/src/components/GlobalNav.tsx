// Shared top-level navigation for every LabWired page. One source of
// truth for the four nav links — Playground, Library, For CI, GitHub —
// plus the LabWired wordmark that links back to the marketing domain.
//
// Used by:
//   * studio/TopChrome.tsx          (playground, dark variant)
//   * library/Library.tsx           (light variant, active=library)
//   * ci/CiLanding.tsx              (light variant, active=ci)
//
// Add or rename a nav item by editing NAV_ITEMS below; every page picks
// it up automatically. Visual variant differences (dark playground
// chrome vs light marketing pages) are isolated to the `variant` prop.

import clsx from 'clsx';

export const LABWIRED_HOME_URL = 'https://labwired.com';

// Build epoch — written into a non-functional DOM attribute on the logo
// so the chunk content (and therefore Vite's output hash) changes per
// build. A CF Pages partial-upload during an API outage left certain
// chunk hashes cached with broken content; bumping this constant per
// commit guarantees a fresh hash that bypasses the broken dedupe entry.
const BUILD_EPOCH = '2026-05-22T16:31:00Z';

// app.labwired.com is JUST the demo (the running simulator). Library, For
// CI, pricing — every marketing/discovery surface — lives on labwired.com.
// All four nav items below are external URLs so the playground always
// exits back to the marketing domain when the user clicks anything except
// the brand logo (which already goes to https://labwired.com).
export const NAV_ITEMS = [
  { id: 'playground', label: 'Playground', href: '/' },
  { id: 'library', label: 'Library', href: 'https://labwired.com/library.html', external: true },
  { id: 'ci', label: 'For CI', href: 'https://labwired.com/ci.html', external: true },
  { id: 'github', label: 'GitHub', href: 'https://github.com/w1ne/labwired-core', external: true },
] as const;

export type NavId = (typeof NAV_ITEMS)[number]['id'];

export interface GlobalNavProps {
  /** Active page — gets the bolded / aria-current styling. */
  active?: NavId;
  /** `dark` matches the playground's translucent dark chrome; `light`
   *  matches the marketing pages' lw-chrome white header. */
  variant?: 'dark' | 'light';
  className?: string;
}

export function GlobalNav({ active, variant = 'light', className }: GlobalNavProps) {
  return (
    <nav className={clsx('flex items-center', variant === 'dark' ? 'gap-1' : 'gap-5 text-[14px]', className)}>
      {NAV_ITEMS.map((item) => {
        const isActive = item.id === active;
        const external = 'external' in item && item.external;
        const cls =
          variant === 'dark'
            ? clsx(
                'flex h-7 px-3 rounded-pill text-xs font-medium transition-colors duration-150 items-center shrink-0',
                isActive
                  ? 'text-fg-primary bg-white/[0.05] font-semibold'
                  : 'text-fg-secondary hover:text-fg-primary hover:bg-white/[0.05]',
              )
            : clsx(
                'font-medium transition-colors duration-150',
                isActive ? 'text-fg-primary font-semibold' : 'text-fg-secondary hover:text-fg-primary',
              );
        return (
          <a
            key={item.id}
            href={item.href}
            aria-current={isActive ? 'page' : undefined}
            target={external ? '_blank' : undefined}
            rel={external ? 'noopener noreferrer' : undefined}
            className={cls}
          >
            {item.label}
          </a>
        );
      })}
    </nav>
  );
}

export interface GlobalLogoProps {
  variant?: 'dark' | 'light';
  className?: string;
}

export function GlobalLogo({ variant = 'light', className }: GlobalLogoProps) {
  return (
    <a
      href={LABWIRED_HOME_URL}
      data-build={BUILD_EPOCH}
      className={clsx(
        'flex items-center gap-2 text-fg-primary tracking-tight shrink-0',
        variant === 'dark' ? 'font-semibold' : 'font-bold',
        className,
      )}
      title="LabWired home"
    >
      <svg viewBox="0 0 20 20" width="18" height="18" aria-hidden="true">
        <path d="M11 2 4 12h4l-1 6 8-10h-4l1-6z" fill="currentColor" />
      </svg>
      LabWired
    </a>
  );
}
