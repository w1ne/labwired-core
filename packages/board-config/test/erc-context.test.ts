import { describe, expect, it } from 'vitest';
import { buildContext, effectivePin } from '../src/erc/context';
import type { DiagramV2 } from '../src/schema';

const minDiagram = (over: Partial<DiagramV2> = {}): DiagramV2 => ({
  version: 2,
  board: 'esp32-s3-zero',
  parts: [{ id: 'mcu', type: 'esp32-s3-zero' }, { id: 'b1', type: 'bme280' }],
  nets: [],
  connections: [],
  wires: [],
  ...over,
});

describe('effectivePin — .N suffix stripping', () => {
  it('resolves mcu:GND.2 to power_out (same as mcu:GND)', () => {
    const ctx = buildContext(minDiagram());
    const pin = effectivePin(ctx, { part: 'mcu', pin: 'GND.2' });
    expect(pin).not.toBeNull();
    expect(pin!.etype).toBe('power_out');
  });

  it('resolves mcu:3V3.1 to power_out', () => {
    const ctx = buildContext(minDiagram());
    const pin = effectivePin(ctx, { part: 'mcu', pin: '3V3.1' });
    expect(pin).not.toBeNull();
    expect(pin!.etype).toBe('power_out');
  });

  it('resolves mcu:GPIO8.3 to bidirectional (same as mcu:GPIO8)', () => {
    const ctx = buildContext(minDiagram());
    const pin = effectivePin(ctx, { part: 'mcu', pin: 'GPIO8.3' });
    expect(pin).not.toBeNull();
    expect(pin!.etype).toBe('bidirectional');
  });

  it('still resolves non-suffixed pins correctly', () => {
    const ctx = buildContext(minDiagram());
    const pin = effectivePin(ctx, { part: 'mcu', pin: 'GND' });
    expect(pin).not.toBeNull();
    expect(pin!.etype).toBe('power_out');
  });

  it('returns null for truly unknown pin (no suffix confusion)', () => {
    const ctx = buildContext(minDiagram());
    const pin = effectivePin(ctx, { part: 'mcu', pin: 'NONEXISTENT_PIN' });
    expect(pin).toBeNull();
  });

  it('catalog part (bme280): strips suffix from pin name lookup', () => {
    const ctx = buildContext(minDiagram());
    // bme280 has VCC pin declared; VCC.1 should resolve to same
    const pin = effectivePin(ctx, { part: 'b1', pin: 'VCC.1' });
    expect(pin).not.toBeNull();
    expect(pin!.etype).toBe('power_in');
  });
});
