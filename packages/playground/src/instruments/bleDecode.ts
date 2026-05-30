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

/** Decode a single air-trace frame into a display transaction. */
export function decodeBleFrame(frame: AirFrameTrace): BleTransaction {
  const onAir = frame.bytes ?? [];

  // The CRC is appended AFTER whitening, so strip it before de-whitening the
  // logical portion. Guard short frames (no CRC present yet).
  const logicalWhitened = onAir.length > CRC_LEN ? onAir.slice(0, onAir.length - CRC_LEN) : [];
  const logical = bleWhiten(logicalWhitened, frame.whitening_iv ?? 0);

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
  };
}

/** Decode a full most-recent-first trace snapshot. */
export function decodeBleTrace(frames: AirFrameTrace[]): BleTransaction[] {
  return (frames ?? []).map(decodeBleFrame);
}
