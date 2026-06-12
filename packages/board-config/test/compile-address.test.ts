/**
 * Fix 6: COMPILE_NO_ADDRESS for i2c_device parts with no address.
 * Fix 7: Shared parseAddr — decimal "64" must NOT be parsed as hex.
 *
 * Both fixes touch compile/index.ts + src/attrs.ts.
 */

import { describe, expect, it } from 'vitest';
import { compile } from '../src/compile';
import { parseAddr } from '../src/attrs';
import type { DiagramV2 } from '../src/schema';

// ---------------------------------------------------------------------------
// Fix 7 unit tests for parseAddr
// ---------------------------------------------------------------------------
describe('parseAddr (shared utility)', () => {
  it('parses hex literal "0x40" to 64', () => {
    expect(parseAddr('0x40')).toBe(64);
  });

  it('parses upper-case hex literal "0X68" to 104', () => {
    expect(parseAddr('0X68')).toBe(104);
  });

  it('parses plain decimal string "64" to 64 (NOT 100)', () => {
    // The old parseInt(s, 16) path would give 100 for "64"
    expect(parseAddr('64')).toBe(64);
  });

  it('parses plain decimal string "40" to 40 (not 0x40=64)', () => {
    expect(parseAddr('40')).toBe(40);
  });

  it('returns undefined for undefined', () => {
    expect(parseAddr(undefined)).toBeUndefined();
  });

  it('returns undefined for empty string', () => {
    expect(parseAddr('')).toBeUndefined();
  });

  it('returns undefined for "abc" (not a valid number)', () => {
    expect(parseAddr('abc')).toBeUndefined();
  });
});

// ---------------------------------------------------------------------------
// Minimal diagram fixture helpers
// ---------------------------------------------------------------------------

/** Base diagram: esp32-s3-zero + pca9685 on I2C, no address attr yet. */
function base(): DiagramV2 {
  return {
    version: 2,
    board: 'esp32-s3-zero',
    parts: [
      { id: 'mcu', type: 'esp32-s3-zero' },
      { id: 'r1',  type: 'resistor' },
      { id: 'r2',  type: 'resistor' },
    ],
    nets: [
      { name: 'GND', kind: 'power', voltage: 0   },
      { name: 'V3',  kind: 'power', voltage: 3.3 },
      { name: 'SDA', kind: 'signal', protocol: 'i2c_sda' },
      { name: 'SCL', kind: 'signal', protocol: 'i2c_scl' },
    ],
    connections: [
      ['mcu:GND',   'GND'],
      ['mcu:3V3',   'V3'],
      ['mcu:GPIO8', 'SDA'],
      ['mcu:GPIO9', 'SCL'],
      ['r1:1', 'SDA'], ['r1:2', 'V3'],
      ['r2:1', 'SCL'], ['r2:2', 'V3'],
    ],
    wires: [],
  };
}

/** Add a pca9685 connected to the I2C bus in base(), with given attrs. */
function withPca(attrs: Record<string, string>): DiagramV2 {
  const d = base();
  d.parts.push({ id: 'pca', type: 'pca9685', attrs });
  d.connections.push(
    ['pca:VCC', 'V3'],
    ['pca:GND', 'GND'],
    ['pca:SDA', 'SDA'],
    ['pca:SCL', 'SCL'],
  );
  return d;
}

// ---------------------------------------------------------------------------
// Fix 6: COMPILE_NO_ADDRESS
// ---------------------------------------------------------------------------
describe('compile — Fix 6: COMPILE_NO_ADDRESS', () => {
  it('i2c_device with no i2c_address → ok:false + COMPILE_NO_ADDRESS', () => {
    // pca9685 is NOT in I2C_DEVICE_ADDRESSES (it's a generic device), so it
    // needs attrs.i2c_address; without it, compile must fail with COMPILE_NO_ADDRESS.
    const diagram = withPca({});
    const result = compile(diagram);
    expect(result.ok, 'Expected compile to fail without an address').toBe(false);
    const codes = result.diagnostics.map((d) => d.code);
    expect(codes, 'Expected COMPILE_NO_ADDRESS in diagnostics').toContain('COMPILE_NO_ADDRESS');
  });

  it('COMPILE_NO_ADDRESS diagnostic includes the part id as a subject', () => {
    const diagram = withPca({});
    const result = compile(diagram);
    const noAddr = result.diagnostics.find((d) => d.code === 'COMPILE_NO_ADDRESS');
    expect(noAddr).toBeDefined();
    expect(noAddr?.subjects).toContain('pca');
  });

  it('i2c_device with valid hex address → ok:true (no COMPILE_NO_ADDRESS)', () => {
    const diagram = withPca({ i2c_address: '0x40' });
    const result = compile(diagram);
    const codes = result.diagnostics.map((d) => d.code);
    expect(codes, 'Should not get COMPILE_NO_ADDRESS when address is set').not.toContain('COMPILE_NO_ADDRESS');
    expect(result.ok).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// Fix 7: parseAddr consistency in compile path
// ---------------------------------------------------------------------------
describe('compile — Fix 7: decimal i2c_address consistency', () => {
  it('attrs.i2c_address "64" is parsed as decimal 64, not hex 100', () => {
    // 0x40 = 64 decimal; if we wrongly use parseInt("64", 16) we get 100.
    // The emitted system YAML must reference 0x40 (decimal 64).
    const diagram = withPca({ i2c_address: '64' });
    const result = compile(diagram);
    expect(result.ok, `Compile failed unexpectedly: ${JSON.stringify(result.diagnostics)}`).toBe(true);
    // systemYaml should contain "0x40" (64 in hex), not "0x64" (100 in hex)
    expect(result.systemYaml).toContain('i2c_address: 0x40');
    expect(result.systemYaml).not.toContain('i2c_address: 0x64');
  });

  it('attrs.i2c_address "0x40" is parsed as 64 and emitted as 0x40', () => {
    const diagram = withPca({ i2c_address: '0x40' });
    const result = compile(diagram);
    expect(result.ok, `Compile failed unexpectedly: ${JSON.stringify(result.diagnostics)}`).toBe(true);
    expect(result.systemYaml).toContain('i2c_address: 0x40');
  });
});
