/**
 * Emitter-catalog coverage gate.
 *
 * Every part type that compile/emitters.ts has a dedicated emitter or special
 * case for must exist in CATALOG. This test fails CI when someone adds a new
 * emitter without a matching catalog entry (or removes a catalog entry that an
 * emitter still references).
 */
import { describe, it, expect } from 'vitest';
import { EMITTED_PART_TYPES } from '../src/compile/emitters';
import { getCatalogPart } from '../src/catalog';

describe('catalog-emitter coverage', () => {
  it('every EMITTED_PART_TYPE has a matching CATALOG entry', () => {
    const missing: string[] = [];
    for (const type of EMITTED_PART_TYPES) {
      if (!getCatalogPart(type)) missing.push(type);
    }
    expect(
      missing,
      `These emitter types are missing from CATALOG: ${missing.join(', ')}`,
    ).toHaveLength(0);
  });

  it('EMITTED_PART_TYPES is non-empty (sanity)', () => {
    expect(EMITTED_PART_TYPES.length).toBeGreaterThan(0);
  });
});
