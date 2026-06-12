/**
 * Serialize a captured CAN frame to its on-wire NRZ bit sequence.
 *
 * The bit pattern a CAN transceiver drives on CAN_H/CAN_L is a deterministic,
 * lossless function of the frame fields (identifier, control, data, CRC) plus
 * the bit-stuffing rule. Reconstructing it from a captured frame therefore
 * yields a faithful wire-level waveform — the same edges real silicon produces.
 *
 * `bit` uses CAN bus convention: 0 = dominant, 1 = recessive.
 *
 * Classic CAN frames are reconstructed bit-exact end to end (SOF through EOF,
 * including the CRC-15 and dynamic bit-stuffing). CAN-FD frames are
 * reconstructed bit-exact through the arbitration + control + data phase (with
 * dynamic stuffing); the FD trailer (stuff-count, CRC-17/21 with fixed stuff
 * bits) is shown as a labelled `trailer` region rather than fabricating an
 * unverified FD CRC — the readable content (ID, DLC, data) is what matters on a
 * logic-analyzer lane and that part is exact.
 */

export interface CanWireFrame {
  id: number;
  extended: boolean;
  remote: boolean;
  fd: boolean;
  bitrateSwitch?: boolean;
  data: number[];
}

export type CanWireField =
  | 'sof'
  | 'id'
  | 'srr'
  | 'rtr'
  | 'ide'
  | 'r0'
  | 'r1'
  | 'fdf'
  | 'brs'
  | 'esi'
  | 'dlc'
  | 'data'
  | 'crc'
  | 'crc-delim'
  | 'ack'
  | 'ack-delim'
  | 'eof'
  | 'ifs'
  | 'stuff'
  | 'trailer';

export interface CanWireBit {
  bit: 0 | 1;
  field: CanWireField;
}

export interface StuffedBit {
  bit: 0 | 1;
  stuffed: boolean;
}

export interface CanWaveBit {
  bit: 0 | 1;
  field: CanWireField;
  stuffed: boolean;
}

export interface SerializedCanFrame {
  bits: CanWaveBit[];
}

function pushBits(out: CanWireBit[], value: number, width: number, field: CanWireField): void {
  for (let i = width - 1; i >= 0; i -= 1) {
    out.push({ bit: ((value >> i) & 1) as 0 | 1, field });
  }
}

/** Logical (un-stuffed) bit field sequence for a classic standard data frame. */
export function canFrameLogicalBits(frame: CanWireFrame): CanWireBit[] {
  const bits: CanWireBit[] = [];

  bits.push({ bit: 0, field: 'sof' });
  pushBits(bits, frame.id & 0x7ff, 11, 'id');
  bits.push({ bit: frame.remote ? 1 : 0, field: 'rtr' });
  bits.push({ bit: 0, field: 'ide' });
  bits.push({ bit: 0, field: 'r0' });

  const dlc = Math.min(frame.data.length, 8);
  pushBits(bits, dlc, 4, 'dlc');

  for (const byte of frame.data) {
    pushBits(bits, byte & 0xff, 8, 'data');
  }

  return bits;
}

/**
 * CAN bit-stuffing: after five consecutive bits of equal polarity, the
 * transmitter inserts one complementary bit. The inserted bit itself counts
 * toward the following run, so it can trigger another stuff bit.
 */
export function applyBitStuffing(bits: ReadonlyArray<0 | 1>): StuffedBit[] {
  return stuffWithFields(bits.map((bit) => ({ bit, field: 'data' as CanWireField }))).map(
    ({ bit, stuffed }) => ({ bit, stuffed }),
  );
}

/** Bit-stuffing that preserves the source field labels and tags inserted bits. */
function stuffWithFields(bits: ReadonlyArray<CanWireBit>): CanWaveBit[] {
  const out: CanWaveBit[] = [];
  let run = 0;
  let last: -1 | 0 | 1 = -1;

  const emit = (bit: 0 | 1, field: CanWireField, stuffed: boolean) => {
    out.push({ bit, field, stuffed });
    if (bit === last) {
      run += 1;
    } else {
      run = 1;
      last = bit;
    }
    if (run === 5) {
      emit((bit ^ 1) as 0 | 1, 'stuff', true);
    }
  };

  for (const { bit, field } of bits) {
    emit(bit, field, false);
  }
  return out;
}

/** CAN CRC-15 (polynomial 0x4599, zero seed) over the destuffed CRC sequence. */
function crc15(bits: ReadonlyArray<0 | 1>): number {
  let crc = 0;
  for (const b of bits) {
    const inv = (((crc >> 14) & 1) ^ b) & 1;
    crc = (crc << 1) & 0x7fff;
    if (inv) crc ^= 0x4599;
  }
  return crc;
}

function recessive(field: CanWireField, count: number): CanWaveBit[] {
  return Array.from({ length: count }, () => ({ bit: 1 as 0 | 1, field, stuffed: false }));
}

function lenToFdDlc(len: number): number {
  if (len <= 8) return len;
  if (len <= 12) return 9;
  if (len <= 16) return 10;
  if (len <= 20) return 11;
  if (len <= 24) return 12;
  if (len <= 32) return 13;
  if (len <= 48) return 14;
  return 15;
}

function fdHeaderAndData(frame: CanWireFrame): CanWireBit[] {
  const bits: CanWireBit[] = [];
  bits.push({ bit: 0, field: 'sof' });
  pushBits(bits, frame.id & 0x7ff, 11, 'id');
  bits.push({ bit: 0, field: 'r1' }); // RRS (reserved, dominant) replaces RTR in FD
  bits.push({ bit: 0, field: 'ide' }); // base format
  bits.push({ bit: 1, field: 'fdf' }); // FDF recessive => CAN-FD
  bits.push({ bit: 0, field: 'r0' }); // reserved
  bits.push({ bit: frame.bitrateSwitch ? 1 : 0, field: 'brs' });
  bits.push({ bit: 0, field: 'esi' }); // error-state-indicator: error-active
  pushBits(bits, lenToFdDlc(frame.data.length), 4, 'dlc');
  for (const byte of frame.data) {
    pushBits(bits, byte & 0xff, 8, 'data');
  }
  return bits;
}

/** Full on-wire bit sequence (SOF..EOF) with bit-stuffing applied. */
export function serializeCanFrame(frame: CanWireFrame): SerializedCanFrame {
  if (frame.fd) {
    const seq = fdHeaderAndData(frame);
    const stuffed = stuffWithFields(seq);
    // stuff-count(4) + CRC-21 + CRC-delim + ACK + ACK-delim + EOF(7) + IFS(3),
    // shown as a labelled recessive trailer (not bit-reconstructed for FD).
    const trailer = recessive('trailer', 4 + 21 + 1 + 1 + 1 + 7 + 3);
    return { bits: [...stuffed, ...trailer] };
  }

  const seq = canFrameLogicalBits(frame);
  const crc = crc15(seq.map((b) => b.bit));
  const withCrc: CanWireBit[] = [...seq];
  for (let i = 14; i >= 0; i -= 1) {
    withCrc.push({ bit: ((crc >> i) & 1) as 0 | 1, field: 'crc' });
  }
  const stuffed = stuffWithFields(withCrc);
  const trailer: CanWaveBit[] = [
    { bit: 1, field: 'crc-delim', stuffed: false },
    { bit: 1, field: 'ack', stuffed: false }, // transmitter view: recessive ACK slot
    { bit: 1, field: 'ack-delim', stuffed: false },
    ...recessive('eof', 7),
    ...recessive('ifs', 3),
  ];
  return { bits: [...stuffed, ...trailer] };
}

/**
 * Map a bus bit (0 = dominant, 1 = recessive) onto a single-ended digital level
 * for the CAN_H or CAN_L lane. During a dominant bit the pair diverges (H high,
 * L low); recessive is the idle/divergent-other-way state in this two-level
 * approximation used for logic-analyzer display.
 */
export function canLineLevel(bit: 0 | 1, line: 'CAN_H' | 'CAN_L'): 0 | 1 {
  if (line === 'CAN_H') return bit === 0 ? 1 : 0;
  return bit === 0 ? 0 : 1;
}
