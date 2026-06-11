import { describe, expect, it } from 'vitest';
import { erc } from '../src/erc';
import type { DiagramV2 } from '../src/schema';

const codesOf = (d: DiagramV2) => erc(d).map((x) => x.code);

const powered = (connections: DiagramV2['connections'], nets: DiagramV2['nets']): DiagramV2 => ({
  version: 2, board: 'esp32-s3-zero',
  parts: [{ id: 'mcu', type: 'esp32-s3-zero' }, { id: 'b1', type: 'bme280' }],
  nets, connections, wires: [],
});

describe('power rules', () => {
  it('PWR_RAIL_UNDRIVEN: power_in pins with no power_out on the net — and its corrected twin', () => {
    const bad = powered([['b1:VCC', 'V']], [{ name: 'V', kind: 'power', voltage: 3.3 }]);
    expect(codesOf(bad)).toContain('PWR_RAIL_UNDRIVEN');

    const good = powered(
      [['b1:VCC', 'V'], ['mcu:3V3', 'V']],
      [{ name: 'V', kind: 'power', voltage: 3.3 }],
    );
    expect(codesOf(good)).not.toContain('PWR_RAIL_UNDRIVEN');
  });

  it('PWR_VOLTAGE_MISMATCH: 5V rail feeding a 3.6V-max part — twin on 3V3 passes', () => {
    const bad = powered(
      [['b1:VCC', 'V5'], ['mcu:5V', 'V5']],
      [{ name: 'V5', kind: 'power', voltage: 5 }],
    );
    expect(codesOf(bad)).toContain('PWR_VOLTAGE_MISMATCH');

    const good = powered(
      [['b1:VCC', 'V3'], ['mcu:3V3', 'V3']],
      [{ name: 'V3', kind: 'power', voltage: 3.3 }],
    );
    expect(codesOf(good)).not.toContain('PWR_VOLTAGE_MISMATCH');
  });

  it('PWR_NO_GROUND: powered part with no pin on a 0V net — twin with GND passes', () => {
    const bad = powered(
      [['b1:VCC', 'V3'], ['mcu:3V3', 'V3']],
      [{ name: 'V3', kind: 'power', voltage: 3.3 }],
    );
    expect(codesOf(bad)).toContain('PWR_NO_GROUND');

    const good = powered(
      [['b1:VCC', 'V3'], ['mcu:3V3', 'V3'], ['b1:GND', 'G'], ['mcu:GND', 'G']],
      [{ name: 'V3', kind: 'power', voltage: 3.3 }, { name: 'G', kind: 'power', voltage: 0 }],
    );
    expect(codesOf(good)).not.toContain('PWR_NO_GROUND');
  });

  it('clean minimal diagram (mcu only, no power nets) has no power rule errors', () => {
    const d: DiagramV2 = {
      version: 2, board: 'esp32-s3-zero',
      parts: [{ id: 'mcu', type: 'esp32-s3-zero' }],
      nets: [], connections: [], wires: [],
    };
    expect(erc(d).filter((x) => x.severity === 'error')).toEqual([]);
  });
});
