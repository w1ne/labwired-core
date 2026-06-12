import { describe, expect, it } from 'vitest';
import { compile } from '../src/compile';
import type { DiagramV2 } from '../src/schema';

/** Clean SpiceDispenser baseline (i2c0-bound pca9685). */
function dispenserFixture(irqSource?: number): DiagramV2 {
  const pcaAttrs: Record<string, string> = { i2c_address: '0x40' };
  if (irqSource !== undefined) {
    pcaAttrs.irq_source = String(irqSource);
  }
  return {
    version: 2,
    board: 'esp32-s3-zero',
    parts: [
      { id: 'mcu',  type: 'esp32-s3-zero' },
      { id: 'pca',  type: 'pca9685', attrs: pcaAttrs },
      { id: 'srv1', type: 'servo' },
      { id: 'r1',   type: 'resistor' },
      { id: 'r2',   type: 'resistor' },
    ],
    nets: [
      { name: 'GND',  kind: 'power',  voltage: 0   },
      { name: 'V3',   kind: 'power',  voltage: 3.3 },
      { name: 'V5',   kind: 'power',  voltage: 5.0 },
      { name: 'SDA',  kind: 'signal', protocol: 'i2c_sda' },
      { name: 'SCL',  kind: 'signal', protocol: 'i2c_scl' },
      { name: 'PWM1', kind: 'signal' },
    ],
    connections: [
      ['mcu:GND',   'GND'],
      ['mcu:3V3',   'V3'],
      ['mcu:5V',    'V5'],
      ['pca:VCC',   'V3'],
      ['pca:GND',   'GND'],
      ['pca:SDA',   'SDA'],
      ['pca:SCL',   'SCL'],
      ['mcu:GPIO8', 'SDA'],
      ['mcu:GPIO9', 'SCL'],
      ['r1:1', 'SDA'],
      ['r1:2', 'V3'],
      ['r2:1', 'SCL'],
      ['r2:2', 'V3'],
      ['pca:LED8',  'PWM1'],
      ['srv1:PWM',  'PWM1'],
      ['srv1:VCC',  'V5'],
      ['srv1:GND',  'GND'],
    ],
    wires: [],
  };
}

describe('IRQ source ordinal validation', () => {
  it('no irq_source attr → clean (no IRQ_SOURCE_ORDINAL diagnostic)', () => {
    const result = compile(dispenserFixture());
    expect(result.ok).toBe(true);
    const codes = result.diagnostics.map((d) => d.code);
    expect(codes).not.toContain('IRQ_SOURCE_ORDINAL');
  });

  it('correct irq_source (42 for i2c0) → clean', () => {
    const result = compile(dispenserFixture(42));
    expect(result.ok).toBe(true);
    const codes = result.diagnostics.map((d) => d.code);
    expect(codes).not.toContain('IRQ_SOURCE_ORDINAL');
  });

  it('wrong irq_source (49 for i2c0) → IRQ_SOURCE_ORDINAL error', () => {
    const result = compile(dispenserFixture(49));
    expect(result.ok).toBe(false);
    const codes = result.diagnostics.map((d) => d.code);
    expect(codes).toContain('IRQ_SOURCE_ORDINAL');
  });

  it('IRQ_SOURCE_ORDINAL hint contains the correct value "42"', () => {
    const result = compile(dispenserFixture(49));
    const irqDiag = result.diagnostics.find((d) => d.code === 'IRQ_SOURCE_ORDINAL');
    expect(irqDiag).toBeDefined();
    expect(irqDiag?.hint).toContain('42');
  });
});
