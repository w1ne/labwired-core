import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { GlobalNav, NAV_ITEMS } from './GlobalNav';

afterEach(() => cleanup());

describe('GlobalNav', () => {
  it('exposes Tools between Library and For CI', () => {
    expect(NAV_ITEMS.map((item) => item.label)).toEqual([
      'Playground',
      'Library',
      'Tools',
      'For CI',
      'Validation',
      'Blog',
      'About',
    ]);
    expect(NAV_ITEMS.find((item) => item.id === 'tools')).toMatchObject({ href: '/?tools=1' });
  });

  it('opens Tools in-app when a handler is provided', () => {
    const onToolsClick = vi.fn();
    render(<GlobalNav active="playground" onToolsClick={onToolsClick} />);

    fireEvent.click(screen.getByText('Tools'));

    expect(onToolsClick).toHaveBeenCalledTimes(1);
  });

  it('stacks links as full-width rows when orientation is vertical', () => {
    const { container } = render(<GlobalNav variant="dark" orientation="vertical" />);
    const nav = container.querySelector('nav')!;
    expect(nav.className).toContain('flex-col');
    // Every link fills the drawer width (one item per row).
    const links = Array.from(nav.querySelectorAll('a'));
    expect(links.length).toBeGreaterThan(0);
    expect(links.every((a) => a.className.includes('w-full'))).toBe(true);
  });
});
