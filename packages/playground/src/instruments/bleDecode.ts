// Pure decoders for the playground's universal packet analyzer.
//
// The simulator's virtual-air registry hands the UI a list of `AirFrameTrace`
// records (most-recent-first), each carrying the PRE-whitening logical bytes
// the transmitter put on air — `[S0, LENGTH, payload…]` for the nRF RADIO's
// BLE/proprietary framing. These functions turn one raw trace into a
// display-ready transaction without any wasm/React coupling, so they can be
// unit-tested in isolation and reused by a CLI exporter later.
import type { AirFrameTrace } from '@labwired/ui';

/** A decoded, display-ready row for the analyzer's protocol view. */
export interface BleTransaction {
  /** RADIO FREQUENCY register value (MHz offset from 2400). */
  channel: number;
  /** Centre frequency in MHz (2400 + channel). */
  freqMhz: number;
  /** Human label for the PHY/MODE register. */
  phy: string;
  /** Logical access address, big-endian-ish "PREFIX:BASE" hex for display. */
  address: string;
  /** S0 header byte (first logical byte), or null if the frame is empty. */
  s0: number | null;
  /** LENGTH header field (second logical byte), or null. */
  length: number | null;
  /** Payload bytes after [S0, LENGTH], clamped to LENGTH when sane. */
  payload: number[];
  /** Convenience: first payload byte — the sensor's incrementing reading. */
  reading: number | null;
  /** Whole logical frame as a spaced hex string for the raw column. */
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

/** Decode a single air-trace frame into a display transaction. */
export function decodeBleFrame(frame: AirFrameTrace): BleTransaction {
  const bytes = frame.bytes ?? [];
  const s0 = bytes.length > 0 ? bytes[0] & 0xff : null;
  const length = bytes.length > 1 ? bytes[1] & 0xff : null;

  // Payload sits after [S0, LENGTH]. Clamp to the declared LENGTH when it
  // fits the frame; otherwise show whatever bytes are actually present so a
  // malformed frame is still visible rather than silently truncated.
  const rawPayload = bytes.slice(2);
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
    hex: toHex(bytes),
  };
}

/** Decode a full most-recent-first trace snapshot. */
export function decodeBleTrace(frames: AirFrameTrace[]): BleTransaction[] {
  return (frames ?? []).map(decodeBleFrame);
}
