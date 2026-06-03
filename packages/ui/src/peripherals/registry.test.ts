// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

import { describe, expect, it } from 'vitest';

import { findKit, kitsWithLabs, PERIPHERAL_KITS, PERIPHERAL_MANIFEST_SCHEMA } from './registry';
import manifestRaw from './manifest.json';

describe('peripheral kit registry', () => {
  it('manifest schema matches the TS reader', () => {
    // The reader throws on mismatch at import time, so reaching this test
    // already proves the schemas line up. The explicit assertion exists
    // so a future schema bump that forgets to bump the constant fails
    // loudly here instead of leaving a silent inconsistency.
    expect((manifestRaw as { schema_version: number }).schema_version).toBe(
      PERIPHERAL_MANIFEST_SCHEMA,
    );
  });

  it('exposes at least one kit', () => {
    expect(PERIPHERAL_KITS.length).toBeGreaterThan(0);
  });

  it('device_type strings are unique', () => {
    const seen = new Set<string>();
    for (const k of PERIPHERAL_KITS) {
      expect(seen.has(k.device_type), `duplicate device_type '${k.device_type}'`).toBe(false);
      seen.add(k.device_type);
    }
  });

  it('findKit resolves every registered kit by device_type', () => {
    for (const k of PERIPHERAL_KITS) {
      expect(findKit(k.device_type)).toEqual(k);
    }
    expect(findKit('this-does-not-exist')).toBeUndefined();
  });

  it('kitsWithLabs yields kits whose labs slice is non-empty', () => {
    for (const k of kitsWithLabs()) {
      expect(k.labs.length).toBeGreaterThan(0);
      for (const lab of k.labs) {
        expect(lab.board_id.length).toBeGreaterThan(0);
        expect(lab.demo_elf.length).toBeGreaterThan(0);
      }
    }
  });
});
