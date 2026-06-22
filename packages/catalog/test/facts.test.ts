import { describe, it, expect } from 'vitest';
import { execFileSync } from 'node:child_process';
import {
  isKnownDeviceType,
  isKnownChip,
  CATALOG_FACTS,
  PERIPHERAL_DEVICE_TYPES,
  PERIPHERALS,
  getPeripheral,
  listPeripherals,
  schemaMatches,
  assertSchemaCompatible,
} from '../src/index';

describe('catalog-facts generation', () => {
  it('src/catalog-facts.json is up to date with the generator', () => {
    expect(() =>
      execFileSync('npm', ['run', 'check:facts'], {
        cwd: new URL('..', import.meta.url),
        stdio: 'pipe',
      }),
    ).not.toThrow();
  });
});

describe('catalog facts helpers', () => {
  it('recognises a kit device_type', () => {
    expect(isKnownDeviceType('ssd1680_tricolor_290')).toBe(true);
  });
  it('recognises a legacy (catalog-only) device_type', () => {
    expect(isKnownDeviceType('uc8151d_tricolor_290')).toBe(true);
  });
  it('rejects an unknown device_type', () => {
    expect(isKnownDeviceType('totally-made-up')).toBe(false);
  });
  it('recognises a chip family', () => {
    expect(isKnownChip('esp32')).toBe(true);
  });
  it('pins the schema version', () => {
    expect(CATALOG_FACTS.schema_version).toBe(2);
  });

  it('exposes enriched peripheral metadata', () => {
    // The coverage set and the enriched list never disagree.
    expect(PERIPHERALS.map((p) => p.device_type)).toEqual([...PERIPHERAL_DEVICE_TYPES]);
    const ssd1680 = getPeripheral('ssd1680_tricolor_290');
    expect(ssd1680?.transport).toBe('spi');
    expect(ssd1680?.kit).toBe(true);
    expect(ssd1680?.label.length).toBeGreaterThan(0);
    // Catalog-only (non-kit) peripheral still gets a label + transport.
    const uc8151d = getPeripheral('uc8151d_tricolor_290');
    expect(uc8151d?.kit).toBe(false);
    expect(uc8151d?.transport).toBe('spi');
    expect(uc8151d?.label).toContain('UC8151D');
    expect(getPeripheral('totally-made-up')).toBeUndefined();
  });

  it('lists peripherals filtered by transport', () => {
    const i2c = listPeripherals('i2c');
    expect(i2c.length).toBeGreaterThan(0);
    expect(i2c.every((p) => p.transport === 'i2c')).toBe(true);
    expect(listPeripherals().length).toBe(PERIPHERALS.length);
  });
  it('treats external peripherals as coverage but excludes passives', () => {
    expect(PERIPHERAL_DEVICE_TYPES).toContain('bme280');
    expect(PERIPHERAL_DEVICE_TYPES).not.toContain('resistor');
    expect(PERIPHERAL_DEVICE_TYPES).not.toContain('mcu');
  });

  // Guard against roast item #5: the generator reaches into board-config's
  // source by deviceClass. A renamed/dropped class would silently shrink the
  // coverage set. These anchors must survive any board-config refactor — one
  // i2c, one spi, one uart device, plus a kit-only (manifest, not catalog) one.
  it('keeps known external peripherals across bus classes in the coverage set', () => {
    for (const dt of ['oled-ssd1306', 'ssd1680_tricolor_290', 'neo6m-gps', 'vl53l1x']) {
      expect(PERIPHERAL_DEVICE_TYPES, `${dt} dropped from coverage set`).toContain(dt);
    }
  });

  // Roast #7: importing never throws on version; assertion is opt-in.
  it('matches its own schema and does not crash on import', () => {
    expect(schemaMatches).toBe(true);
    expect(() => assertSchemaCompatible()).not.toThrow();
  });

  // Roast #2: chips is a clean family list, not polluted with board variants.
  it('includes real chip families but excludes board-variant aliases', () => {
    expect(CATALOG_FACTS.chips).toContain('esp32'); // proto.cat uses this
    expect(CATALOG_FACTS.chips).toContain('stm32f103');
    expect(CATALOG_FACTS.chips).not.toContain('esp32-s3-zero');
    expect(CATALOG_FACTS.chips).not.toContain('nrf52840-onboarding');
  });
});
