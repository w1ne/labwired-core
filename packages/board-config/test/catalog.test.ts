import { describe, expect, it } from 'vitest';
import { CATALOG, getCatalogPart, type PinDecl } from '../src/catalog';
import { COMPONENT_META } from '../src/component-meta';


describe('catalog', () => {
  it('every legacy COMPONENT_META key exists in the catalog with the same boardIoKind', () => {
    for (const [type, meta] of Object.entries(COMPONENT_META)) {
      const part = getCatalogPart(type);
      expect(part, `catalog missing legacy type '${type}'`).toBeDefined();
      expect(part!.boardIoKind).toEqual(meta.boardIoKind);
    }
  });

  it('pca9685 declares typed pins including open-drain I2C and 16 outputs', () => {
    const pins = getCatalogPart('pca9685')!.pins!;
    const byName = Object.fromEntries(pins.map((p) => [p.name, p]));
    expect(byName.SDA).toEqual<PinDecl>({ name: 'SDA', etype: 'open_drain', role: 'i2c_sda' });
    expect(byName.SCL).toEqual<PinDecl>({ name: 'SCL', etype: 'open_drain', role: 'i2c_scl' });
    expect(byName.VCC.etype).toBe('power_in');
    expect(byName.GND.etype).toBe('power_in');
    expect(pins.filter((p) => p.name.startsWith('LED')).length).toBe(16);
  });

  it('resistor pins are passive; led pins are passive; button pins are passive', () => {
    for (const type of ['resistor', 'led', 'button']) {
      const part = getCatalogPart(type);
      expect(part?.pins?.every((p) => p.etype === 'passive'), type).toBe(true);
    }
  });

  it('bme280 declares operating voltage range', () => {
    expect(getCatalogPart('bme280')!.operatingVoltage).toEqual({ min: 1.71, max: 3.6 });
  });

  it('parts without pin declarations are explicitly legacy (pins undefined)', () => {
    // Incremental adoption: ERC pin rules only run where pins are declared.
    expect(getCatalogPart('keypad')!.pins).toBeUndefined();
  });

  it('unknown type returns undefined', () => {
    expect(getCatalogPart('definitely-not-a-part')).toBeUndefined();
  });
});

describe('deviceClass', () => {
  it('classifies the key part families', () => {
    expect(getCatalogPart('esp32-s3-zero')!.deviceClass).toBe('mcu');
    expect(getCatalogPart('pca9685')!.deviceClass).toBe('i2c_device');
    expect(getCatalogPart('bme280')!.deviceClass).toBe('i2c_device');
    expect(getCatalogPart('resistor')!.deviceClass).toBe('passive');
    expect(getCatalogPart('led')!.deviceClass).toBe('board_io');
    expect(getCatalogPart('button')!.deviceClass).toBe('board_io');
    expect(getCatalogPart('neo6m-gps')!.deviceClass).toBe('uart_device');
  });
  it('every catalog entry has a deviceClass', () => {
    for (const [type, part] of Object.entries(CATALOG)) {
      expect(part.deviceClass, type).toBeDefined();
    }
  });
});
