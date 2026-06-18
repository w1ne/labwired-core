import { CHIP_YAMLS } from './chip-yamls';

// Each wireable chip carries a REAL 3D model. This maps every chip in
// CHIP_YAMLS (the chips a user can place + wire) to the kernelCAD catalog id of
// its package's 3D model — see kernelCAD-web scripts/electronics-parts.json,
// served from the kernelcad-parts catalog. The id resolves to a measured STEP
// via find_part / fetch_part, so a placed chip is real geometry, not a box.
//
// A value of `null` is an explicit, tracked GAP (no faithful model yet) — never
// a silent omission, never a fake. The package per chip is taken from the
// specific part the config models (e.g. stm32f103c8 = LQFP-48), not guessed.
//
// chip-models.test.ts enforces that EVERY CHIP_YAMLS chip has an entry here, so
// adding a new wireable chip forces a model decision (real ref or tracked gap).
export const CHIP_MODEL_REFS: Record<string, string | null> = {
  // STM32F103C8 — "Blue Pill", LQFP-48.
  stm32f103: 'ic-lqfp-48',
  // STM32F401RE — Nucleo part, LQFP-64.
  stm32f401: 'ic-lqfp-64',
  // STM32L476RG — Nucleo part, LQFP-64.
  stm32l476: 'ic-lqfp-64',
};

/** Catalog id for a chip's 3D model, or null when none is mapped yet. */
export function chipModelRef(chipId: string): string | null {
  return CHIP_MODEL_REFS[chipId] ?? null;
}

/** Chip ids that are wireable but have no 3D model mapped yet (tracked gaps). */
export function chipModelGaps(): string[] {
  return Object.keys(CHIP_YAMLS)
    .filter((id) => !CHIP_MODEL_REFS[id])
    .sort();
}
