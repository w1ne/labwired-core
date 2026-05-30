// Pure decoders for the playground's universal packet analyzer.
//
// The simulator's virtual-air registry hands the UI a list of `AirFrameTrace`
// records (most-recent-first). Each carries the bytes the transmitter actually
// put ON AIR — i.e. the BLE-whitened header+payload+CRC, "exactly what a
// sniffer on the air would capture" (see the RADIO model's TX path). The
// receiver de-whitens on the far side; the air capture itself stays whitened.
//
// Because the captured bytes are whitened, we do NOT pretend to parse logical
// [S0, LENGTH, payload] fields out of them — that would print garbage. Instead
// we present the truthful sniffer view: the frame's metadata (channel → centre
// frequency, MODE → PHY, logical address) plus the raw on-air bytes. De-whitened
// logical decoding is a separate, opt-in layer (it needs the whitening IV, which
// the trace does not yet carry).
//
// Keeping this as a pure function means it can be unit-tested in isolation and
// reused by a CLI exporter later.
import type { AirFrameTrace } from '@labwired/ui';

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
  /** Number of bytes captured on air (whitened header+payload+CRC). */
  byteCount: number;
  /** Raw on-air bytes as a spaced hex string. */
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
  const prefix = (frame.addr_prefix & 0xff).toString(16).padStart(2, '0').toUpperCase();
  const base = (frame.addr_base >>> 0).toString(16).padStart(8, '0').toUpperCase();

  return {
    channel: frame.channel,
    freqMhz: 2400 + frame.channel,
    phy: phyLabel(frame.mode),
    address: `${prefix}:${base}`,
    byteCount: bytes.length,
    hex: toHex(bytes),
  };
}

/** Decode a full most-recent-first trace snapshot. */
export function decodeBleTrace(frames: AirFrameTrace[]): BleTransaction[] {
  return (frames ?? []).map(decodeBleFrame);
}
