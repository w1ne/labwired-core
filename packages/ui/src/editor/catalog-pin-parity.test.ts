/**
 * Anti-drift gate: the board-config CATALOG pins (used by the MCP validator)
 * must match the renderer's COMPONENT_REGISTRY pins exactly. If they drift, a
 * wire can validate but be undrawable (or vice versa) — that's the "looks wired
 * but isn't" slop that shipped in share HdH8Qv7EdcYR. We compare every part
 * present in BOTH registries with typed pins; legacy pin-less catalog parts are
 * skipped (tracked as a coverage gap, not a drift failure).
 */
import { describe, expect, it } from 'vitest';
import { CATALOG } from '@labwired/board-config';
import { COMPONENT_REGISTRY } from './components/index';

describe('catalog ↔ renderer pin parity', () => {
  const dualTyped = Object.values(CATALOG).filter(
    (c) => c.pins?.length && COMPONENT_REGISTRY.get(c.type)?.pins?.length,
  );

  it('has parts to check', () => {
    expect(dualTyped.length).toBeGreaterThan(0);
  });

  for (const cat of dualTyped) {
    it(`"${cat.type}" pins match the renderer`, () => {
      const catalogPins = [...cat.pins!.map((p) => p.name)].sort();
      const rendererPins = [...COMPONENT_REGISTRY.get(cat.type)!.pins.map((p) => p.id)].sort();
      expect(catalogPins).toEqual(rendererPins);
    });
  }
});
