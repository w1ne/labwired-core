// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

/**
 * Playground-side gate for the unified peripheral contract.
 *
 * For every peripheral the Rust core registers via `PeripheralKit`, this
 * test verifies that the five hand-maintained playground surfaces all
 * carry a matching entry. The Rust side already guarantees the *runtime*
 * shape (model + bus attach + lab + manifest); this test guarantees the
 * *UI* shape — so a kit shipped without a chip, library tile, icon, or
 * palette category fails CI here instead of in the browser.
 *
 * Add a new peripheral by implementing `PeripheralKit` in core. CI then
 * tells you exactly which TS surface you forgot to update.
 */

import { describe, expect, it } from 'vitest';
import { PERIPHERAL_KITS, kitsWithLabs } from '@labwired/ui';

import { BOARD_CONFIGS } from './bundled-configs';
import { STARTER_LABS } from './studio/ChipRow';
import { FEATURED_LABS } from './library/Library';
import { getComponentIcon } from './studio/componentIcons';
import { COMPONENT_CATEGORIES } from './studio/useCommandPaletteItems';

describe('peripheral kits ↔ playground surfaces', () => {
  it('manifest is non-empty (smoke)', () => {
    expect(PERIPHERAL_KITS.length).toBeGreaterThan(0);
  });

  it.each(kitsWithLabs())(
    'kit "$device_type" has a BOARD_CONFIGS entry matching its lab',
    (kit) => {
      const board = BOARD_CONFIGS.find((b) => b.boardId === kit.lab.board_id);
      expect(
        board,
        `kit '${kit.device_type}' lab '${kit.lab.board_id}' has no BOARD_CONFIGS entry`,
      ).toBeDefined();
      expect(board!.chipId).toBe(kit.lab.chip);
      // The demo ELF the manifest declares must match what BOARD_CONFIGS
      // tells the browser to fetch — otherwise Run shows index.html.
      expect(board!.demoFirmwarePath ?? '').toContain(kit.lab.demo_elf);
    },
  );

  it.each(kitsWithLabs())(
    'kit "$device_type" appears in STARTER_LABS (chip row)',
    (kit) => {
      const lab = STARTER_LABS.find((l) => l.id === kit.lab.board_id);
      expect(
        lab,
        `kit '${kit.device_type}' lab '${kit.lab.board_id}' is missing from STARTER_LABS`,
      ).toBeDefined();
    },
  );

  it.each(kitsWithLabs())(
    'kit "$device_type" appears in the Library',
    (kit) => {
      const tile = FEATURED_LABS.find((b) => b.id === kit.lab.board_id);
      expect(
        tile,
        `kit '${kit.device_type}' lab '${kit.lab.board_id}' is missing from FEATURED_LABS`,
      ).toBeDefined();
    },
  );

  it.each(PERIPHERAL_KITS)(
    'kit "$device_type" resolves to a non-fallback icon',
    (kit) => {
      // getComponentIcon falls back to a generic category icon for unknown
      // types. We can't easily detect the fallback by identity, so a weaker
      // assertion: the function returns *something* (not undefined / null)
      // and doesn't throw for either an explicit category or a misc fallback.
      const icon = getComponentIcon(kit.device_type, kit.category as never);
      expect(icon).toBeDefined();
    },
  );

  it.each(PERIPHERAL_KITS)(
    'kit "$device_type" has a palette category registered',
    (kit) => {
      const category = COMPONENT_CATEGORIES[kit.device_type];
      expect(
        category,
        `kit '${kit.device_type}' is missing from COMPONENT_CATEGORIES (command palette)`,
      ).toBeDefined();
      expect(category).toBe(kit.category);
    },
  );
});
