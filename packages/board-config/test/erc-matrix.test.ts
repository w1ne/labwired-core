import { describe, expect, it } from 'vitest';
import { erc } from '../src/erc';
import { pairFinding } from '../src/erc/matrix-rules';
import type { DiagramV2 } from '../src/schema';

/**
 * Build a minimal two-connection diagram that puts aPart:aPin and bPart:bPin
 * on the same signal net 'N'.
 * When aPart === 'mcu' the mcu part is already in the fixed parts[] — don't add it again.
 * When bPart === 'mcu' or bPart === aPart, same logic applies.
 */
const two = (aPart: string, aType: string, aPin: string, bPart: string, bType: string, bPin: string): DiagramV2 => ({
  version: 2, board: 'esp32-s3-zero',
  parts: [
    { id: 'mcu', type: 'esp32-s3-zero' },
    ...(aPart === 'mcu' ? [] : [{ id: aPart, type: aType }]),
    ...(bPart === 'mcu' || bPart === aPart ? [] : [{ id: bPart, type: bType }]),
  ],
  nets: [{ name: 'N', kind: 'signal' as const }],
  connections: [[`${aPart}:${aPin}`, 'N'], [`${bPart}:${bPin}`, 'N']],
  wires: [],
});

const codesOf = (d: DiagramV2) => erc(d).map((x) => x.code);

describe('pin-pair matrix', () => {
  it('NET_DRIVER_CONFLICT: two outputs on one net', () => {
    // pca9685 LED0 (output) + ultrasonic ECHO (output)
    expect(codesOf(two('p1', 'pca9685', 'LED0', 'u1', 'ultrasonic', 'ECHO')))
      .toContain('NET_DRIVER_CONFLICT');
  });

  it('NET_DRIVER_CONFLICT: output driving a power_out rail pin', () => {
    expect(codesOf(two('p1', 'pca9685', 'LED0', 'mcu', '', '3V3')))
      .toContain('NET_DRIVER_CONFLICT');
  });

  it('NET_RAIL_SHORT: two power_out pins shorted', () => {
    expect(codesOf(two('mcu', '', '3V3', 'mcu', '', '5V'))).toContain('NET_RAIL_SHORT');
  });

  it('no finding for passive + anything', () => {
    expect(codesOf(two('r1', 'resistor', '1', 'p1', 'pca9685', 'LED0')))
      .not.toContain('NET_DRIVER_CONFLICT');
  });

  it('no finding for input + output', () => {
    expect(codesOf(two('u1', 'ultrasonic', 'TRIG', 'p1', 'pca9685', 'LED0')))
      .not.toContain('NET_DRIVER_CONFLICT');
  });

  it('legacy parts (no pin decls) are skipped silently', () => {
    expect(codesOf(two('k1', 'keypad', 'X', 'p1', 'pca9685', 'LED0'))).toEqual(
      expect.not.arrayContaining(['NET_DRIVER_CONFLICT', 'NET_UNSPECIFIED_PIN']),
    );
  });

  it('NET_RAIL_SHORT: two power nets with different voltages bridged by one pin', () => {
    const d: DiagramV2 = {
      version: 2, board: 'esp32-s3-zero',
      parts: [{ id: 'mcu', type: 'esp32-s3-zero' }, { id: 'p1', type: 'pca9685' }],
      nets: [
        { name: '3V3', kind: 'power', voltage: 3.3 },
        { name: '5V0', kind: 'power', voltage: 5 },
      ],
      connections: [['p1:VCC', '3V3'], ['p1:VCC', '5V0']],
      wires: [],
    };
    expect(codesOf(d)).toContain('NET_RAIL_SHORT');
  });
});

// Unit tests for pairFinding — cover nc and unspecified cells
// which have no real catalog parts in current fixture parts.
describe('pairFinding unit tests', () => {
  it('nc + anything → NET_NC_CONNECTED (error)', () => {
    expect(pairFinding('nc', 'input')).toEqual({ code: 'NET_NC_CONNECTED', severity: 'error' });
    expect(pairFinding('output', 'nc')).toEqual({ code: 'NET_NC_CONNECTED', severity: 'error' });
    expect(pairFinding('nc', 'nc')).toEqual({ code: 'NET_NC_CONNECTED', severity: 'error' });
  });

  it('output + output → NET_DRIVER_CONFLICT (error)', () => {
    expect(pairFinding('output', 'output')).toEqual({ code: 'NET_DRIVER_CONFLICT', severity: 'error' });
  });

  it('output + power_out → NET_DRIVER_CONFLICT (error)', () => {
    expect(pairFinding('output', 'power_out')).toEqual({ code: 'NET_DRIVER_CONFLICT', severity: 'error' });
    expect(pairFinding('power_out', 'output')).toEqual({ code: 'NET_DRIVER_CONFLICT', severity: 'error' });
  });

  it('power_out + power_out → NET_RAIL_SHORT (error)', () => {
    expect(pairFinding('power_out', 'power_out')).toEqual({ code: 'NET_RAIL_SHORT', severity: 'error' });
  });

  it('open_drain + output → NET_DRIVER_CONFLICT (warning)', () => {
    expect(pairFinding('open_drain', 'output')).toEqual({ code: 'NET_DRIVER_CONFLICT', severity: 'warning' });
    expect(pairFinding('output', 'open_drain')).toEqual({ code: 'NET_DRIVER_CONFLICT', severity: 'warning' });
  });

  it('unspecified + specified → NET_UNSPECIFIED_PIN (warning)', () => {
    expect(pairFinding('unspecified', 'input')).toEqual({ code: 'NET_UNSPECIFIED_PIN', severity: 'warning' });
    expect(pairFinding('output', 'unspecified')).toEqual({ code: 'NET_UNSPECIFIED_PIN', severity: 'warning' });
  });

  it('unspecified + unspecified → null (two unknowns: no finding)', () => {
    expect(pairFinding('unspecified', 'unspecified')).toBeNull();
  });

  it('input + output → null (OK)', () => {
    expect(pairFinding('input', 'output')).toBeNull();
  });

  it('passive + any → null', () => {
    expect(pairFinding('passive', 'output')).toBeNull();
    expect(pairFinding('passive', 'power_out')).toBeNull();
    expect(pairFinding('passive', 'passive')).toBeNull();
  });

  it('open_drain + open_drain → null (wired-OR is fine)', () => {
    expect(pairFinding('open_drain', 'open_drain')).toBeNull();
  });
});
