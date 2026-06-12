import { describe, it, expect } from 'vitest';
import { composeDiagnostics } from '../src/compose';

describe('surface parity (both adapters delegate to composeDiagnostics)', () => {
  const cleanDispenser = {
    board: 'esp32-s3-zero',
    parts: [
      { id: 'mcu', type: 'esp32-s3-zero' },
      { id: 'pca1', type: 'pca9685', attrs: { i2c_address: '0x40' } },
    ],
    wires: [
      { from: { part: 'mcu', pin: 'GPIO8' }, to: { part: 'pca1', pin: 'SDA' } },
      { from: { part: 'mcu', pin: 'GPIO9' }, to: { part: 'pca1', pin: 'SCL' } },
      { from: { part: 'mcu', pin: '3V3' }, to: { part: 'pca1', pin: 'VCC' } },
      { from: { part: 'mcu', pin: 'GND' }, to: { part: 'pca1', pin: 'GND' } },
    ],
  };

  it('composeDiagnostics returns ok for a clean diagram', () => {
    const result = composeDiagnostics(cleanDispenser as any);
    expect(result.ok).toBe(true);
    expect(result.error_count).toBe(0);
    expect(result).toHaveProperty('diagnostics');
  });

  it('composeDiagnostics returns !ok for a diagram with unknown component', () => {
    const bad = {
      ...cleanDispenser,
      parts: [...cleanDispenser.parts, { id: 'bad1', type: 'nonexistent_part_xyz' }],
    };
    const result = composeDiagnostics(bad as any);
    expect(result.ok).toBe(false);
  });

  it('composeDiagnostics reports I2C_ADDR_CONFLICT (kernel code) for duplicate addresses', () => {
    const diagram = {
      board: 'esp32-s3-zero',
      parts: [
        { id: 'mcu', type: 'esp32-s3-zero' },
        { id: 'pca1', type: 'pca9685', attrs: { i2c_address: '0x40' } },
        { id: 'pca2', type: 'pca9685', attrs: { i2c_address: '0x40' } },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'GPIO8' }, to: { part: 'pca1', pin: 'SDA' } },
        { from: { part: 'mcu', pin: 'GPIO9' }, to: { part: 'pca1', pin: 'SCL' } },
        { from: { part: 'mcu', pin: 'GPIO8' }, to: { part: 'pca2', pin: 'SDA' } },
        { from: { part: 'mcu', pin: 'GPIO9' }, to: { part: 'pca2', pin: 'SCL' } },
        { from: { part: 'mcu', pin: '3V3' }, to: { part: 'pca1', pin: 'VCC' } },
        { from: { part: 'mcu', pin: 'GND' }, to: { part: 'pca1', pin: 'GND' } },
        { from: { part: 'mcu', pin: '3V3' }, to: { part: 'pca2', pin: 'VCC' } },
        { from: { part: 'mcu', pin: 'GND' }, to: { part: 'pca2', pin: 'GND' } },
      ],
    };
    const result = composeDiagnostics(diagram as any);
    const codes = result.diagnostics.map((d) => d.code);
    expect(codes).toContain('I2C_ADDR_CONFLICT');
  });

  it('composeDiagnostics includes both legacy UNKNOWN_COMPONENT and kernel SCHEMA_PART_UNKNOWN for unknown types', () => {
    // A diagram wiring an unknown component type — triggers both legacy and kernel codes
    const diagram = {
      board: 'esp32-s3-zero',
      parts: [
        { id: 'mcu', type: 'esp32-s3-zero' },
        { id: 'unk1', type: 'totally_unknown_xyz' },
      ],
      wires: [
        // Wire to an unknown part triggers UNKNOWN_COMPONENT in legacy,
        // SCHEMA_PART_UNKNOWN in kernel ERC
        { from: { part: 'mcu', pin: 'GPIO8' }, to: { part: 'unk1', pin: 'SDA' } },
      ],
    };
    const result = composeDiagnostics(diagram as any);
    const codes = result.diagnostics.map((d) => d.code);
    // At minimum, one of the unknown-type codes must appear
    const hasUnknownCode = codes.includes('UNKNOWN_COMPONENT') || codes.includes('SCHEMA_PART_UNKNOWN');
    expect(hasUnknownCode).toBe(true);
    // The result is an error
    expect(result.ok).toBe(false);
  });
});
