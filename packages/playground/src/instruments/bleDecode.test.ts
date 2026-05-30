import { describe, it, expect } from 'vitest';
import { decodeBleFrame, decodeBleTrace, phyLabel, toHex } from './bleDecode';
import type { AirFrameTrace } from '@labwired/ui';

// The sensor firmware puts [S0=0xAB, LENGTH=0x04, reading, 0, 0, 0] on air at
// FREQUENCY=42 (2442 MHz) MODE=3 (BLE 1M), addr BASE0=0xCAFEBA00 PREFIX0=0xBE.
function sensorFrame(reading: number): AirFrameTrace {
  return {
    channel: 42,
    addr_base: 0xcafeba00,
    addr_prefix: 0xbe,
    mode: 3,
    bytes: [0xab, 0x04, reading, 0x00, 0x00, 0x00],
  };
}

describe('bleDecode', () => {
  it('decodes the sensor frame into a readable transaction', () => {
    const tx = decodeBleFrame(sensorFrame(7));
    expect(tx.channel).toBe(42);
    expect(tx.freqMhz).toBe(2442);
    expect(tx.phy).toBe('BLE 1M');
    expect(tx.address).toBe('BE:CAFEBA00');
    expect(tx.s0).toBe(0xab);
    expect(tx.length).toBe(4);
    expect(tx.payload).toEqual([7, 0, 0, 0]);
    expect(tx.reading).toBe(7);
    expect(tx.hex).toBe('AB 04 07 00 00 00');
  });

  it('clamps payload to the declared LENGTH', () => {
    const tx = decodeBleFrame({
      channel: 42,
      addr_base: 0,
      addr_prefix: 0,
      mode: 3,
      bytes: [0xab, 0x02, 0x11, 0x22, 0x33, 0x44],
    });
    expect(tx.length).toBe(2);
    expect(tx.payload).toEqual([0x11, 0x22]);
  });

  it('survives empty / short frames without throwing', () => {
    expect(decodeBleFrame({ channel: 0, addr_base: 0, addr_prefix: 0, mode: 3, bytes: [] }).s0).toBeNull();
    const short = decodeBleFrame({ channel: 0, addr_base: 0, addr_prefix: 0, mode: 3, bytes: [0xab] });
    expect(short.s0).toBe(0xab);
    expect(short.length).toBeNull();
    expect(short.payload).toEqual([]);
    expect(short.reading).toBeNull();
  });

  it('preserves most-recent-first order across a trace', () => {
    const rows = decodeBleTrace([sensorFrame(9), sensorFrame(8), sensorFrame(7)]);
    expect(rows.map((r) => r.reading)).toEqual([9, 8, 7]);
  });

  it('labels known PHYs and formats hex', () => {
    expect(phyLabel(3)).toBe('BLE 1M');
    expect(phyLabel(99)).toBe('MODE 99');
    expect(toHex([0x00, 0xff, 0x2a])).toBe('00 FF 2A');
  });
});
