// Shared top-level navigation for every LabWired page. One source of
// truth for the nav links — Playground, Library, Tools, For CI,
// Validation, Blog, About — plus the LabWired wordmark that links back
// to the marketing domain. Mirrors components/header.html in the
// landing_page repo; change both together.
//
// Used by:
//   * studio/TopChrome.tsx                (playground, dark variant)
//   * library/Library.tsx                 (light variant, active=library)
//   * ci/CiLanding.tsx                    (light variant, active=ci)
//   * validation/ValidationLanding.tsx    (light variant, active=validation)
//
// Add or rename a nav item by editing NAV_ITEMS below; every page picks
// it up automatically. Visual variant differences (dark playground
// chrome vs light marketing pages) are isolated to the `variant` prop.

import clsx from 'clsx';
import type { MouseEvent, ReactNode } from 'react';

export const LABWIRED_HOME_URL = 'https://labwired.com';

// Build epoch — written into a non-functional DOM attribute on the logo
// so the chunk content (and therefore Vite's output hash) changes per
// build. A CF Pages partial-upload during an API outage left certain
// chunk hashes cached with broken content; bumping this constant per
// commit guarantees a fresh hash that bypasses the broken dedupe entry.
const BUILD_EPOCH = '2026-05-22T16:31:00Z';

// app.labwired.com is JUST the demo (the running simulator). Library, For
// CI, pricing — every marketing/discovery surface — lives on labwired.com,
// with one deliberate exception: the Validation matrix page is served from
// the playground app (same-origin fetch + shared design system); a
// labwired.com nav entry for Validation is tracked separately.
// The Tools entry opens the in-app tool panel; the rest exit back to the
// marketing domain when the user clicks anything except the brand logo.
export const NAV_ITEMS = [
  { id: 'playground', label: 'Playground', href: '/' },
  { id: 'library', label: 'Library', href: 'https://labwired.com/library.html', external: true },
  { id: 'tools', label: 'Tools', href: '/?tools=1' },
  { id: 'ci', label: 'For CI', href: 'https://labwired.com/ci.html', external: true },
  { id: 'validation', label: 'Validation', href: '/validation.html' },
  { id: 'blog', label: 'Blog', href: 'https://labwired.com/blog/', external: true },
  { id: 'about', label: 'About', href: 'https://labwired.com/about.html', external: true },
] as const;

export type NavId = (typeof NAV_ITEMS)[number]['id'];

export interface GlobalNavProps {
  /** Active page — gets the bolded / aria-current styling. */
  active?: NavId;
  /** `dark` matches the playground's translucent dark chrome; `light`
   *  matches the marketing pages' lw-chrome white header. */
  variant?: 'dark' | 'light';
  className?: string;
  onToolsClick?: () => void;
  toolsSlot?: ReactNode;
  /** Nav items to hide on this surface (e.g. the playground app hides Validation). */
  exclude?: NavId[];
}

export function GlobalNav({ active, variant = 'light', className, onToolsClick, toolsSlot, exclude }: GlobalNavProps) {
  return (
    <nav className={clsx('flex items-center', variant === 'dark' ? 'gap-1' : 'gap-5 text-[14px]', className)}>
      {NAV_ITEMS.filter((item) => item.id !== active && !(exclude ?? []).includes(item.id)).map((item) => {
        if (item.id === 'tools' && toolsSlot) {
          return <div key={item.id}>{toolsSlot}</div>;
        }
        const isActive = item.id === active;
        const external = 'external' in item && item.external;
        const handleClick =
          item.id === 'tools' && onToolsClick
            ? (event: MouseEvent<HTMLAnchorElement>) => {
                event.preventDefault();
                onToolsClick();
              }
            : undefined;
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
            onClick={handleClick}
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
      <svg viewBox="0 0 32 32" width="20" height="20" fill="none" aria-hidden="true">
        <path d="M11 7V23H23" stroke="#4d9fff" stroke-width="3" stroke-linecap="round" stroke-linejoin="round" />
        <circle cx="11" cy="7" r="3" fill="currentColor" />
        <circle cx="23" cy="23" r="3" fill="#4d9fff" />
      </svg>
      LabWired
    </a>
  );
}
