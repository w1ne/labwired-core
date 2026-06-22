import { describe, it, expect } from 'vitest';
import { execFileSync } from 'node:child_process';
import {
  isKnownDeviceType,
  isKnownChip,
  CATALOG_FACTS,
  PERIPHERAL_DEVICE_TYPES,
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
    expect(CATALOG_FACTS.schema_version).toBe(1);
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
});
