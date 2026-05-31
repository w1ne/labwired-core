import { describe, it, expect } from 'vitest';
import {
  BLE_CRC_INIT,
  bleCrc24,
  bleWhiten,
  decodeBleFrame,
  decodeBleTrace,
  phyLabel,
  toHex,
} from './bleDecode';
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

// Build an on-air frame the way the RADIO model's TX path does
// (core/.../nrf52/radio.rs): whiten the logical [S0, LENGTH, payload], compute
// CRC-24 over the WHITENED bytes with the BLE init, then append the 3 CRC bytes
// little-endian (LSB first). decodeBleFrame must then read crcOk === true.
function validOnAirFrame(logical: number[]): AirFrameTrace {
  const whitened = bleWhiten(logical, IV);
  const crc = bleCrc24(whitened, BLE_CRC_INIT);
  const bytes = [...whitened, crc & 0xff, (crc >> 8) & 0xff, (crc >> 16) & 0xff];
  return { channel: 42, addr_base: 0xcafeba00, addr_prefix: 0xbe, mode: 3, bytes, whitening_iv: IV };
}

describe('bleCrc24', () => {
  it('is deterministic for a fixed input', () => {
    const data = [0x01, 0x04, 0x00, 0x01, 0x02, 0x03];
    expect(bleCrc24(data, BLE_CRC_INIT)).toBe(bleCrc24(data, BLE_CRC_INIT));
  });

  it('returns a 24-bit value', () => {
    const crc = bleCrc24([0xde, 0xad, 0xbe, 0xef], BLE_CRC_INIT);
    expect(crc).toBeGreaterThanOrEqual(0);
    expect(crc).toBeLessThanOrEqual(0xffffff);
  });

  it('returns the init unchanged for empty input (matches the Rust routine)', () => {
    expect(bleCrc24([], 0)).toBe(0);
    expect(bleCrc24([], BLE_CRC_INIT)).toBe(BLE_CRC_INIT);
  });

  it('changes when any input byte is corrupted', () => {
    const data = [0x01, 0x04, 0x00, 0x01, 0x02, 0x03];
    const good = bleCrc24(data, BLE_CRC_INIT);
    const corrupted = [...data];
    corrupted[2] ^= 0xff;
    expect(bleCrc24(corrupted, BLE_CRC_INIT)).not.toBe(good);
  });
});

describe('decodeBleFrame CRC verification', () => {
  it('reports crcOk=true for a frame whose appended CRC matches the engine', () => {
    const tx = decodeBleFrame(validOnAirFrame([0xab, 0x04, 0, 1, 2, 3]));
    expect(tx.crcOk).toBe(true);
    // Logical decode must still work alongside CRC verification.
    expect(tx.reading).toBe(0);
    expect(tx.payload).toEqual([0, 1, 2, 3]);
  });

  it('reports crcOk=false when a whitened payload byte is flipped post-CRC', () => {
    const f = validOnAirFrame([0xab, 0x04, 0, 1, 2, 3]);
    f.bytes[2] ^= 0xff; // corrupt payload, leave CRC bytes intact
    expect(decodeBleFrame(f).crcOk).toBe(false);
  });

  it('reports crcOk=false when a CRC byte itself is corrupted', () => {
    const f = validOnAirFrame([0xab, 0x02, 0x07, 0x08]);
    f.bytes[f.bytes.length - 1] ^= 0xff;
    expect(decodeBleFrame(f).crcOk).toBe(false);
  });

  it('reports crcOk=false for the fake-CRC fixtures used by the other tests', () => {
    // airFrameFromLogical appends a placeholder [0x11,0x22,0x33] CRC, so an
    // honest verifier must flag it BAD — it is not a real engine CRC.
    expect(decodeBleFrame(airFrameFromLogical([0xab, 0x04, 7, 0, 0, 0])).crcOk).toBe(false);
  });

  it('reports crcOk=null for a frame too short to hold a CRC', () => {
    const tx = decodeBleFrame({
      channel: 0,
      addr_base: 0,
      addr_prefix: 0,
      mode: 3,
      bytes: [0xaa, 0xbb],
      whitening_iv: IV,
    });
    expect(tx.crcOk).toBeNull();
  });
});
