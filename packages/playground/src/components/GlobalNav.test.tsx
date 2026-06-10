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
});
