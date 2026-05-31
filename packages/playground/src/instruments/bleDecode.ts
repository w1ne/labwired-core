// Pure decoders for the playground's universal packet analyzer.
//
// The simulator's virtual-air registry hands the UI a list of `AirFrameTrace`
// records (most-recent-first). Each carries the bytes the transmitter actually
// put ON AIR — the BLE-whitened header+payload with the 3-byte CRC appended
// AFTER whitening — plus the whitening IV the sender used. With the IV we can
// reverse the PN9 whitening and recover the logical [S0, LENGTH, payload]
// frame, which is what makes the demo's incrementing reading human-visible.
//
// Keeping this as pure functions means they can be unit-tested in isolation
// (against bytes captured from the real RADIO model) and reused by a CLI
// exporter later.
import type { AirFrameTrace } from '@labwired/ui';

/** Number of trailing CRC bytes appended (post-whitening) by the RADIO model. */
const CRC_LEN = 3;

/**
 * BLE CRC-24 init value (CRCINIT). The air trace does not carry the sender's
 * CRCINIT register, so we assume the BLE-standard 0x555555 used by every
 * board on the shared virtual air (matches the RADIO model's call sites and
 * tests in core/.../nrf52/radio.rs). If a frame used a different init, it will
 * read BAD — which is the honest answer, not a fabricated OK.
 */
export const BLE_CRC_INIT = 0x555555;

/** A decoded, display-ready row for the analyzer's protocol view. */
export interface BleTransaction {
  /** RADIO FREQUENCY register value (MHz offset from 2400). */
  channel: number;
  /** Centre frequency in MHz (2400 + channel). */
  freqMhz: number;
  /** Human label for the PHY/MODE register. */
  phy: string;
  /** Logical access address, "PREFIX:BASE" hex for display. */
  address: string;
  /** S0 header byte (first de-whitened logical byte), or null if absent. */
  s0: number | null;
  /** LENGTH header field (second logical byte), or null. */
  length: number | null;
  /** De-whitened payload bytes after [S0, LENGTH], clamped to LENGTH. */
  payload: number[];
  /** Convenience: first payload byte — the sensor's incrementing reading. */
  reading: number | null;
  /** Raw on-air (whitened) frame as spaced hex — the literal sniffer view. */
  rawHex: string;
  /** De-whitened logical frame (header+payload, CRC stripped) as spaced hex. */
  hex: string;
  /**
   * CRC-24 verification result: true if the recomputed CRC over the on-air
   * (whitened) bytes matches the trailing 3 CRC bytes, false if it mismatches,
   * null if the frame is too short to contain a CRC.
   */
  crcOk: boolean | null;
}

/** Map the RADIO MODE register to a short PHY label. */
export function phyLabel(mode: number): string {
  switch (mode) {
    case 0:
      return 'Nordic 1M';
    case 1:
      return 'Nordic 2M';
    case 2:
      return 'BLE 250k';
    case 3:
      return 'BLE 1M';
    case 4:
      return 'BLE 2M';
    case 5:
      return 'BLE Coded';
    default:
      return `MODE ${mode}`;
  }
}

/** Format a byte array as space-separated two-digit hex. */
export function toHex(bytes: number[]): string {
  return bytes.map((b) => (b & 0xff).toString(16).padStart(2, '0').toUpperCase()).join(' ');
}

/**
 * PN9 BLE whitening — XORs `data` in place with the LFSR output. Symmetric:
 * applying it twice cancels, so the same routine de-whitens. Port of the
 * RADIO model's `ble_whiten` (LFSR = x^7 + x^4 + 1, seed = (iv & 0x7F) | 0x40).
 */
export function bleWhiten(data: number[], whiteningIv: number): number[] {
  let lfsr = (whiteningIv & 0x7f) | 0x40;
  const out = new Array<number>(data.length);
  for (let i = 0; i < data.length; i++) {
    let b = 0;
    for (let bit = 0; bit < 8; bit++) {
      const bitLfsr = (lfsr >> 6) & 1;
      const bitIn = (data[i] >> bit) & 1;
      b |= (bitIn ^ bitLfsr) << bit;
      const feedback = bitLfsr;
      lfsr = ((lfsr << 1) | feedback) & 0x7f;
      if (feedback !== 0) lfsr ^= 0x04;
    }
    out[i] = b;
  }
  return out;
}

/**
 * BLE CRC-24 — faithful port of the RADIO model's `ble_crc24`
 * (core/crates/core/src/peripherals/nrf52/radio.rs). MSB-first shift register,
 * polynomial 0x65B (the low bits of 0x100065B), seeded from `crcInit`.
 *
 * The engine computes this over the WHITENED on-air bytes (TX whitens the
 * header+payload in place, THEN appends the CRC of those whitened bytes), so
 * the verifier here must feed it the same whitened bytes — NOT the de-whitened
 * logical frame.
 */
export function bleCrc24(data: number[], crcInit: number): number {
  let crc = crcInit & 0xffffff;
  for (const byte of data) {
    crc ^= (byte & 0xff) << 16;
    for (let bit = 0; bit < 8; bit++) {
      if ((crc & (1 << 23)) !== 0) {
        crc = ((crc << 1) ^ 0x65b) & 0xffffff;
      } else {
        crc = (crc << 1) & 0xffffff;
      }
    }
  }
  return crc >>> 0;
}

/** Decode a single air-trace frame into a display transaction. */
export function decodeBleFrame(frame: AirFrameTrace): BleTransaction {
  const onAir = frame.bytes ?? [];

  // The CRC is appended AFTER whitening, so strip it before de-whitening the
  // logical portion. Guard short frames (no CRC present yet).
  const hasCrc = onAir.length > CRC_LEN;
  const logicalWhitened = hasCrc ? onAir.slice(0, onAir.length - CRC_LEN) : [];
  const logical = bleWhiten(logicalWhitened, frame.whitening_iv ?? 0);

  // Verify CRC-24: recompute over the whitened on-air bytes (everything before
  // the trailing 3 CRC bytes) and compare against the appended little-endian
  // CRC (byte0 = bits 7:0, byte1 = bits 15:8, byte2 = bits 23:16), matching the
  // RADIO model's TX append order exactly.
  let crcOk: boolean | null = null;
  if (hasCrc) {
    const expected = bleCrc24(logicalWhitened, BLE_CRC_INIT);
    const c0 = onAir[onAir.length - 3] & 0xff;
    const c1 = onAir[onAir.length - 2] & 0xff;
    const c2 = onAir[onAir.length - 1] & 0xff;
    const received = ((c2 << 16) | (c1 << 8) | c0) >>> 0;
    crcOk = expected === received;
  }

  const s0 = logical.length > 0 ? logical[0] & 0xff : null;
  const length = logical.length > 1 ? logical[1] & 0xff : null;
  const rawPayload = logical.slice(2);
  const payload =
    length !== null && length <= rawPayload.length ? rawPayload.slice(0, length) : rawPayload;

  const prefix = (frame.addr_prefix & 0xff).toString(16).padStart(2, '0').toUpperCase();
  const base = (frame.addr_base >>> 0).toString(16).padStart(8, '0').toUpperCase();

  return {
    channel: frame.channel,
    freqMhz: 2400 + frame.channel,
    phy: phyLabel(frame.mode),
    address: `${prefix}:${base}`,
    s0,
    length,
    payload,
    reading: payload.length > 0 ? payload[0] & 0xff : null,
    rawHex: toHex(onAir),
    hex: toHex(logical),
    crcOk,
  };
}

/** Decode a full most-recent-first trace snapshot. */
export function decodeBleTrace(frames: AirFrameTrace[]): BleTransaction[] {
  return (frames ?? []).map(decodeBleFrame);
}
