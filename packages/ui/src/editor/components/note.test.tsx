// @vitest-environment jsdom
import { describe, it, expect } from 'vitest';
import { render } from '@testing-library/react';
import { COMPONENT_REGISTRY, getComponentsByCategory, renderComponentBody } from './index';

describe('note component', () => {
  it('is registered as an inert tool with no pins or board binding', () => {
    const def = COMPONENT_REGISTRY.get('note');
    expect(def).toBeDefined();
    expect(def!.category).toBe('tool');
    expect(def!.pins).toEqual([]);
    expect(def!.boardIoKind).toBeUndefined();
  });

  it('appears under the Tools palette group', () => {
    const groups = getComponentsByCategory();
    expect(groups.tool?.some((d) => d.type === 'note')).toBe(true);
  });

  it('renders the attr text without throwing (empty and long)', () => {
    const def = COMPONENT_REGISTRY.get('note')!;
    const long = 'x'.repeat(400);
    expect(() =>
      render(<svg>{renderComponentBody(def, { text: '' }, { id: 'n1' })}</svg>),
    ).not.toThrow();
    expect(() =>
      render(<svg>{renderComponentBody(def, { text: long }, { id: 'n2' })}</svg>),
    ).not.toThrow();
  });
});
