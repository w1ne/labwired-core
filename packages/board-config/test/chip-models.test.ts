import { describe, it, expect } from 'vitest';
import { CHIP_YAMLS } from '../src/chip-yamls';
import { CHIP_MODEL_REFS, chipModelGaps } from '../src/chip-models';

// "We don't want to design electronics without 3D models." Enforce it for the
// wireable chips: every CHIP_YAMLS chip must have a CHIP_MODEL_REFS entry (a
// real catalog id or an explicit null gap), and the set of gaps cannot grow
// silently — a new wireable chip with no model decision fails CI.

const KNOWN_MODEL_GAPS: string[] = []; // every wireable chip currently carries a model

describe('chip 3D-model coverage', () => {
  it('every wireable chip has an explicit model decision', () => {
    for (const id of Object.keys(CHIP_YAMLS)) {
      expect(CHIP_MODEL_REFS, `chip ${id}`).toHaveProperty(id);
      const ref = CHIP_MODEL_REFS[id];
      expect(
        ref === null || (typeof ref === 'string' && ref.length > 0),
        `chip ${id} model ref must be a non-empty catalog id or explicit null, got ${JSON.stringify(ref)}`,
      ).toBe(true);
    }
  });

  it('the set of unmodelled chips matches the tracked gap list (no silent growth)', () => {
    expect(chipModelGaps()).toEqual([...KNOWN_MODEL_GAPS].sort());
  });

  it('does not map chips that are not wireable', () => {
    for (const id of Object.keys(CHIP_MODEL_REFS)) {
      expect(CHIP_YAMLS, `mapped chip ${id} must exist in CHIP_YAMLS`).toHaveProperty(id);
    }
  });
});
