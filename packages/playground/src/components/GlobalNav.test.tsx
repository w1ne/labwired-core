import { describe, expect, it } from 'vitest';
import { NAV_ITEMS } from './GlobalNav';

describe('GlobalNav', () => {
  it('exposes Tools between Library and For CI', () => {
    expect(NAV_ITEMS.map((item) => item.label)).toEqual(['Playground', 'Library', 'Tools', 'For CI']);
    expect(NAV_ITEMS.find((item) => item.id === 'tools')).toMatchObject({ href: '/?tools=1' });
  });
});
