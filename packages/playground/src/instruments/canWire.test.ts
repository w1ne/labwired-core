import { describe, it, expect } from 'vitest';
import { applyBitStuffing, canFrameLogicalBits, canLineLevel, serializeCanFrame } from './canWire';

describe('applyBitStuffing', () => {
  it('inserts one opposite stuff bit after five consecutive identical bits', () => {
    const out = applyBitStuffing([0, 0, 0, 0, 0, 0]);
    // Five dominant bits force a recessive stuff bit before the sixth.
    expect(out.map((b) => b.bit)).toEqual([0, 0, 0, 0, 0, 1, 0]);
    expect(out.map((b) => b.stuffed)).toEqual([false, false, false, false, false, true, false]);
  });

  it('counts an inserted stuff bit toward the next run', () => {
    // 5 zeros -> stuff a 1; that stuff-1 plus the next four input 1s make five 1s -> stuff a 0;
    // then the final input 1 remains.
    const out = applyBitStuffing([0, 0, 0, 0, 0, 1, 1, 1, 1, 1]);
    expect(out.map((b) => b.bit)).toEqual([0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 0, 1]);
    expect(out.filter((b) => b.stuffed).length).toBe(2);
  });

  it('does not stuff when no five-in-a-row run exists', () => {
    const out = applyBitStuffing([0, 1, 0, 1, 0, 1]);
    expect(out.map((b) => b.bit)).toEqual([0, 1, 0, 1, 0, 1]);
    expect(out.some((b) => b.stuffed)).toBe(false);
  });
});

describe('canFrameLogicalBits', () => {
  it('encodes SOF, 11-bit ID (MSB first), RTR, IDE, r0, DLC, and data for a classic standard data frame', () => {
    const bits = canFrameLogicalBits({ id: 0x100, extended: false, remote: false, fd: false, data: [0xab] });

    // Start-of-frame is a single dominant bit.
    expect(bits[0]).toMatchObject({ bit: 0, field: 'sof' });

    // 11-bit identifier 0x100 transmitted MSB-first.
    const id = bits.filter((b) => b.field === 'id').map((b) => b.bit);
    expect(id).toEqual([0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0]);

    // RTR dominant for a data frame; IDE dominant for a standard frame.
    expect(bits.find((b) => b.field === 'rtr')?.bit).toBe(0);
    expect(bits.find((b) => b.field === 'ide')?.bit).toBe(0);

    // DLC = 1 (one data byte), 4 bits MSB-first.
    const dlc = bits.filter((b) => b.field === 'dlc').map((b) => b.bit);
    expect(dlc).toEqual([0, 0, 0, 1]);

    // Data byte 0xAB transmitted MSB-first.
    const data = bits.filter((b) => b.field === 'data').map((b) => b.bit);
    expect(data).toEqual([1, 0, 1, 0, 1, 0, 1, 1]);
  });
});

describe('serializeCanFrame (classic data frame)', () => {
  // The real UDS tester request observed on the H563 demo bus.
  const request = serializeCanFrame({
    id: 0x7e0,
    extended: false,
    remote: false,
    fd: false,
    data: [0x03, 0x22, 0xf1, 0x90],
  });

  it('begins with a dominant SOF', () => {
    expect(request.bits[0]).toMatchObject({ field: 'sof', bit: 0 });
  });

  it('carries a 15-bit CRC field', () => {
    expect(request.bits.filter((b) => b.field === 'crc').length).toBe(15);
  });

  it('ends with a recessive EOF of seven bits', () => {
    const eof = request.bits.filter((b) => b.field === 'eof');
    expect(eof.length).toBe(7);
    expect(eof.every((b) => b.bit === 1)).toBe(true);
  });

  it('never emits six consecutive identical bits through the stuffed region', () => {
    // Bit stuffing covers SOF through the CRC field. Reaching six-in-a-row there
    // would be a stuffing bug that corrupts the waveform.
    const stuffedRegion = request.bits.filter((b) => b.field !== 'crc-delim' && b.field !== 'ack' && b.field !== 'ack-delim' && b.field !== 'eof' && b.field !== 'ifs');
    let run = 1;
    for (let i = 1; i < stuffedRegion.length; i += 1) {
      run = stuffedRegion[i].bit === stuffedRegion[i - 1].bit ? run + 1 : 1;
      expect(run).toBeLessThan(6);
    }
  });

  it('CRC-15 of an all-dominant CRC sequence is zero', () => {
    // A standard data frame with ID 0 and no data is an all-zero (all-dominant)
    // CRC sequence; CAN CRC-15 with a zero seed leaves the register at zero.
    const zero = serializeCanFrame({ id: 0, extended: false, remote: false, fd: false, data: [] });
    const crc = zero.bits.filter((b) => b.field === 'crc').map((b) => b.bit);
    expect(crc).toEqual([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
  });
});

describe('canLineLevel', () => {
  it('maps dominant/recessive bits onto the differential CAN_H and CAN_L lines', () => {
    // Dominant (0): CAN_H driven high, CAN_L driven low.
    expect(canLineLevel(0, 'CAN_H')).toBe(1);
    expect(canLineLevel(0, 'CAN_L')).toBe(0);
    // Recessive (1): lines diverge the other way in the two-level model.
    expect(canLineLevel(1, 'CAN_H')).toBe(0);
    expect(canLineLevel(1, 'CAN_L')).toBe(1);
  });
});
