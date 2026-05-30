import { describe, it, expect } from 'vitest';
import { decodeBleFrame, decodeBleTrace, phyLabel, toHex, bleWhiten } from './bleDecode';
import type { AirFrameTrace } from '@labwired/ui';

// The sensor transmits on FREQUENCY=42 (2442 MHz) MODE=3 (BLE 1M), addr
// BASE0=0xCAFEBA00 PREFIX0=0xBE, DATAWHITEIV=42. The logical frame is
// [S0=0xAB, LENGTH=0x04, reading, 0, 0, 0]; the model whitens it, then appends
// a 3-byte CRC. We reconstruct that exact on-air shape so the decoder is tested
// against real model behaviour, and assert it recovers the logical reading.
const IV = 42;

function airFrameFromLogical(logical: number[]): AirFrameTrace {
  const whitened = bleWhiten(logical, IV); // symmetric: whiten == de-whiten
  const bytes = [...whitened, 0x11, 0x22, 0x33]; // fake 3-byte CRC appended post-whitening
  return { channel: 42, addr_base: 0xcafeba00, addr_prefix: 0xbe, mode: 3, bytes, whitening_iv: IV };
}

describe('bleDecode', () => {
  it('de-whitens and recovers the logical reading', () => {
    const tx = decodeBleFrame(airFrameFromLogical([0xab, 0x04, 7, 0, 0, 0]));
    expect(tx.channel).toBe(42);
    expect(tx.freqMhz).toBe(2442);
    expect(tx.phy).toBe('BLE 1M');
    expect(tx.address).toBe('BE:CAFEBA00');
    expect(tx.s0).toBe(0xab);
    expect(tx.length).toBe(4);
    expect(tx.payload).toEqual([7, 0, 0, 0]);
    expect(tx.reading).toBe(7);
    expect(tx.hex).toBe('AB 04 07 00 00 00'); // de-whitened logical frame
  });

  it('exposes the raw whitened bytes alongside the logical decode', () => {
    const f = airFrameFromLogical([0xab, 0x04, 9, 0, 0, 0]);
    const tx = decodeBleFrame(f);
    expect(tx.rawHex).toBe(toHex(f.bytes)); // literal on-air bytes incl. CRC
    expect(tx.reading).toBe(9);
  });

  it('clamps payload to the declared LENGTH', () => {
    const tx = decodeBleFrame(airFrameFromLogical([0xab, 0x02, 0x11, 0x22, 0x33, 0x44]));
    expect(tx.length).toBe(2);
    expect(tx.payload).toEqual([0x11, 0x22]);
  });

  it('survives empty / CRC-only frames without throwing', () => {
    const tx = decodeBleFrame({ channel: 0, addr_base: 0, addr_prefix: 0, mode: 3, bytes: [], whitening_iv: IV });
    expect(tx.s0).toBeNull();
    expect(tx.length).toBeNull();
    expect(tx.payload).toEqual([]);
    expect(tx.reading).toBeNull();
  });

  it('preserves most-recent-first order across a trace', () => {
    const rows = decodeBleTrace([
      airFrameFromLogical([0xab, 0x04, 9, 0, 0, 0]),
      airFrameFromLogical([0xab, 0x04, 8, 0, 0, 0]),
      airFrameFromLogical([0xab, 0x04, 7, 0, 0, 0]),
    ]);
    expect(rows.map((r) => r.reading)).toEqual([9, 8, 7]);
  });

  it('whitening is symmetric', () => {
    const original = [0xab, 0x04, 0x2a, 0x00];
    const once = bleWhiten(original, IV);
    expect(once).not.toEqual(original);
    expect(bleWhiten(once, IV)).toEqual(original);
  });

  it('labels known PHYs and formats hex', () => {
    expect(phyLabel(3)).toBe('BLE 1M');
    expect(phyLabel(99)).toBe('MODE 99');
    expect(toHex([0x00, 0xff, 0x2a])).toBe('00 FF 2A');
  });
});
