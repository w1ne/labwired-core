import { describe, it, expect, beforeEach } from 'vitest';
import { render } from '@testing-library/react';
import { EmbedBadge } from './EmbedBadge';

function setUrl(href: string) {
  window.history.replaceState({}, '', href);
}

describe('EmbedBadge', () => {
  beforeEach(() => {
    setUrl('/?embed=true&share=abc123');
  });

  it('renders the LabWired logo and attribution text', () => {
    const { getByText, container } = render(<EmbedBadge />);
    expect(getByText(/Made with LabWired/i)).toBeTruthy();
    // GlobalLogo's mark is an inline SVG.
    expect(container.querySelector('svg')).toBeTruthy();
  });

  it('links back to the full lab — the current URL with the embed param removed', () => {
    const { container } = render(<EmbedBadge />);
    const anchor = container.querySelector('a') as HTMLAnchorElement;
    expect(anchor).toBeTruthy();
    const href = new URL(anchor.href, window.location.origin);
    expect(href.searchParams.get('embed')).toBeNull();
    // Other params survive.
    expect(href.searchParams.get('share')).toBe('abc123');
  });

  it('opens the full lab in a new tab safely', () => {
    const { container } = render(<EmbedBadge />);
    const anchor = container.querySelector('a') as HTMLAnchorElement;
    expect(anchor.target).toBe('_blank');
    expect(anchor.rel).toContain('noopener');
  });
});
