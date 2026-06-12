import { describe, it, expect } from 'vitest';
import { buildCanWaveformSamples, canSamplesFromTrace, maxTraceSeq } from './canWaveform';

const traceFrame = (over: Partial<{ seq: number; peripheral: string; id: number; data: number[] }>) => ({
  seq: 0,
  peripheral: 'fdcan1',
  direction: 'tx' as const,
  id: 0x7e0,
  data: [0x03, 0x22, 0xf1, 0x90],
  extended: false,
  fd: false,
  bitrate_switch: false,
  remote: false,
  ...over,
});

describe('canSamplesFromTrace', () => {
  const channels = [{ channel: 'CH0', pin: 'CAN_H' as const }];
  const peripherals = new Set(['fdcan1']);

  it('includes only frames from the probed peripherals', () => {
    const trace = [traceFrame({ seq: 0, peripheral: 'fdcan1' }), traceFrame({ seq: 1, peripheral: 'fdcan2' })];
    const samples = canSamplesFromTrace({ trace, canChannels: channels, peripherals });
    // One frame's worth of bits (the fdcan2 frame is excluded).
    const oneFrame = canSamplesFromTrace({ trace: [trace[0]], canChannels: channels, peripherals });
    expect(samples.length).toBe(oneFrame.length);
  });

  it('drops frames at or below the cleared sequence baseline', () => {
    const trace = [traceFrame({ seq: 5 }), traceFrame({ seq: 6, id: 0x7e8 })];
    // Clear after seq 5 -> only seq 6 remains.
    const after = canSamplesFromTrace({ trace, canChannels: channels, peripherals, clearedSeq: 5 });
    const onlySix = canSamplesFromTrace({ trace: [trace[1]], canChannels: channels, peripherals });
    expect(after.length).toBe(onlySix.length);
  });

  it('returns nothing when every frame is at or below the cleared baseline', () => {
    const trace = [traceFrame({ seq: 5 }), traceFrame({ seq: 6 })];
    expect(canSamplesFromTrace({ trace, canChannels: channels, peripherals, clearedSeq: 6 })).toEqual([]);
  });
});

describe('maxTraceSeq', () => {
  it('returns the highest seq among probed-peripheral frames, or -1 when none', () => {
    const trace = [traceFrame({ seq: 2 }), traceFrame({ seq: 9 }), traceFrame({ seq: 4, peripheral: 'other' })];
    expect(maxTraceSeq(trace, new Set(['fdcan1']))).toBe(9);
    expect(maxTraceSeq([], new Set(['fdcan1']))).toBe(-1);
  });
});

const request = {
  id: 0x7e0,
  extended: false,
  remote: false,
  fd: false,
  bitrateSwitch: false,
  data: [0x03, 0x22, 0xf1, 0x90],
};

describe('buildCanWaveformSamples', () => {
  it('produces one sample per wire bit with per-channel CAN_H/CAN_L levels', () => {
    const samples = buildCanWaveformSamples({
      frames: [request],
      canChannels: [
        { channel: 'CH0', pin: 'CAN_H' },
        { channel: 'CH1', pin: 'CAN_L' },
      ],
    });

    // A 4-byte classic frame is well over 40 bits on the wire.
    expect(samples.length).toBeGreaterThan(40);

    // SOF is dominant: CAN_H driven high, CAN_L driven low.
    const first = samples[0];
    expect(first.channels.find((c) => c.channel === 'CH0')?.value).toBe(1);
    expect(first.channels.find((c) => c.channel === 'CH1')?.value).toBe(0);

    // Time advances one unit per bit.
    expect(samples[1].t).toBe(samples[0].t + 1);

    // Channels carry a source label so the UI is honest about provenance.
    expect(first.channels.find((c) => c.channel === 'CH0')?.source).toMatch(/can/i);
  });

  it('separates consecutive frames with recessive idle bits', () => {
    const samples = buildCanWaveformSamples({
      frames: [request, { ...request, id: 0x7e8 }],
      canChannels: [{ channel: 'CH0', pin: 'CAN_H' }],
      idleBitsBetween: 5,
    });
    // Idle is recessive -> CAN_H low (0). At least one all-idle stretch exists.
    const ch0 = samples.map((s) => s.channels.find((c) => c.channel === 'CH0')?.value);
    expect(ch0).toContain(0);
    // Two frames' worth of bits plus the idle gap.
    expect(samples.length).toBeGreaterThan(90);
  });

  it('returns no samples when there are no frames', () => {
    expect(buildCanWaveformSamples({ frames: [], canChannels: [{ channel: 'CH0', pin: 'CAN_H' }] })).toEqual([]);
  });
});
