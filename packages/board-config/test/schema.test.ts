import { describe, expect, it } from 'vitest';
import { migrateToV2, parsePinRef, type DiagramV2 } from '../src/schema';
import type { Diagram } from '../src/types';

describe('parsePinRef', () => {
  it('parses part:pin', () => {
    expect(parsePinRef('esp1:GPIO8')).toEqual({ part: 'esp1', pin: 'GPIO8' });
  });
  it('keeps .N disambiguation suffix as part of the pin name', () => {
    expect(parsePinRef('esp1:GND.2')).toEqual({ part: 'esp1', pin: 'GND.2' });
  });
  it('returns null on malformed refs', () => {
    expect(parsePinRef('no-colon')).toBeNull();
    expect(parsePinRef(':pin')).toBeNull();
    expect(parsePinRef('part:')).toBeNull();
  });
});

describe('migrateToV2', () => {
  const v1: Diagram = {
    board: 'esp32-s3-zero',
    parts: [
      { id: 'led1', type: 'led' },
      { id: 'pca1', type: 'pca9685', attrs: { i2c_address: '0x40' } },
    ],
    wires: [
      { from: { part: 'mcu', pin: 'GPIO8' }, to: { part: 'pca1', pin: 'SDA' } },
    ],
  };

  it('wraps a v1 diagram losslessly: parts, board, wires preserved', () => {
    const v2 = migrateToV2(v1);
    expect(v2.version).toBe(2);
    expect(v2.board).toBe('esp32-s3-zero');
    expect(v2.parts).toEqual(v1.parts);
    expect(v2.nets).toEqual([]);
    expect(v2.connections).toEqual([]);
    expect(v2.wires).toEqual(v1.wires);
  });

  it('passes a v2 diagram through unchanged (same object content)', () => {
    const v2In: DiagramV2 = {
      version: 2,
      board: 'esp32-s3-zero',
      parts: [{ id: 'pca1', type: 'pca9685' }],
      nets: [{ name: '3V3', kind: 'power', voltage: 3.3 }],
      connections: [['pca1:VCC', '3V3']],
      wires: [],
    };
    expect(migrateToV2(v2In)).toEqual(v2In);
  });

  it('treats versionless input as v1', () => {
    const versionless = { ...v1 } as Record<string, unknown>;
    delete versionless.version;
    const v2 = migrateToV2(versionless as unknown as Diagram);
    expect(v2.version).toBe(2);
  });

  it('does not mutate its input', () => {
    const frozen = Object.freeze({ ...v1, parts: Object.freeze([...v1.parts]) });
    expect(() => migrateToV2(frozen as Diagram)).not.toThrow();
  });
});
