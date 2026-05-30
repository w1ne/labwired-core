import { describe, it, expect } from 'vitest';
import { decodeBleFrame, decodeBleTrace, phyLabel, toHex } from './bleDecode';
import type { AirFrameTrace } from '@labwired/ui';

// The sensor transmits on FREQUENCY=42 (2442 MHz) MODE=3 (BLE 1M), addr
// BASE0=0xCAFEBA00 PREFIX0=0xBE. The bytes that reach the air trace are
// WHITENED, so the analyzer shows them raw — it does not parse logical fields.
function airFrame(bytes: number[]): AirFrameTrace {
  return { channel: 42, addr_base: 0xcafeba00, addr_prefix: 0xbe, mode: 3, bytes };
}

describe('bleDecode', () => {
  it('decodes frame metadata into a readable sniffer row', () => {
    const tx = decodeBleFrame(airFrame([0x60, 0xf8, 0xc2, 0x78, 0xd3, 0x93]));
    expect(tx.channel).toBe(42);
    expect(tx.freqMhz).toBe(2442);
    expect(tx.phy).toBe('BLE 1M');
    expect(tx.address).toBe('BE:CAFEBA00');
    expect(tx.byteCount).toBe(6);
    expect(tx.hex).toBe('60 F8 C2 78 D3 93');
  });

  it('survives empty frames without throwing', () => {
    const tx = decodeBleFrame(airFrame([]));
    expect(tx.byteCount).toBe(0);
    expect(tx.hex).toBe('');
    expect(tx.address).toBe('BE:CAFEBA00');
  });

  it('preserves most-recent-first order across a trace', () => {
    const rows = decodeBleTrace([airFrame([0x01]), airFrame([0x02]), airFrame([0x03])]);
    expect(rows.map((r) => r.hex)).toEqual(['01', '02', '03']);
  });

  it('labels known PHYs and formats hex', () => {
    expect(phyLabel(3)).toBe('BLE 1M');
    expect(phyLabel(99)).toBe('MODE 99');
    expect(toHex([0x00, 0xff, 0x2a])).toBe('00 FF 2A');
  });
});
