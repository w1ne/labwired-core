import type { FdcanTraceFrame } from '@labwired/ui';
import type { LogicAnalyzerSample } from './logicAnalyzerCapture';
import { canLineLevel, serializeCanFrame, type CanWireFrame } from './canWire';

function toCanWireFrame(frame: FdcanTraceFrame): CanWireFrame {
  return {
    id: frame.id,
    extended: frame.extended,
    remote: frame.remote,
    fd: frame.fd,
    bitrateSwitch: frame.bitrate_switch,
    data: frame.data,
  };
}

/** Highest seq among frames on the probed peripherals, or -1 if there are none. */
export function maxTraceSeq(trace: FdcanTraceFrame[], peripherals: Set<string>): number {
  return trace.reduce((max, f) => (peripherals.has(f.peripheral) ? Math.max(max, f.seq) : max), -1);
}

export interface CanSamplesFromTraceOptions {
  trace: FdcanTraceFrame[];
  canChannels: CanWaveformChannel[];
  peripherals: Set<string>;
  /** Frames at or below this seq are hidden (set by the Clear button). Default -1. */
  clearedSeq?: number;
}

/**
 * Build the CAN_H/CAN_L waveform from a live FDCAN trace snapshot: keep frames
 * from the probed peripherals that arrived after the cleared baseline, in seq
 * order, and reconstruct their on-wire bits.
 */
export function canSamplesFromTrace({
  trace,
  canChannels,
  peripherals,
  clearedSeq = -1,
}: CanSamplesFromTraceOptions): LogicAnalyzerSample[] {
  const frames = trace
    .filter((f) => peripherals.has(f.peripheral) && f.seq > clearedSeq)
    .slice()
    .sort((a, b) => a.seq - b.seq)
    .map(toCanWireFrame);
  return buildCanWaveformSamples({ frames, canChannels });
}

export interface CanWaveformChannel {
  channel: string;
  pin: 'CAN_H' | 'CAN_L';
}

export interface BuildCanWaveformOptions {
  frames: CanWireFrame[];
  canChannels: CanWaveformChannel[];
  /** Recessive idle bits inserted between consecutive frames (default 8). */
  idleBitsBetween?: number;
}

/**
 * Reconstruct the CAN_H/CAN_L logic-analyzer waveform from captured frames.
 * Each on-wire bit becomes one sample; idle gaps (recessive) separate frames so
 * the lanes read like a real differential bus capture.
 */
export function buildCanWaveformSamples({
  frames,
  canChannels,
  idleBitsBetween = 8,
}: BuildCanWaveformOptions): LogicAnalyzerSample[] {
  if (frames.length === 0) return [];

  const samples: LogicAnalyzerSample[] = [];
  let t = 0;

  const emitBit = (bit: 0 | 1) => {
    samples.push({
      t,
      channels: canChannels.map(({ channel, pin }) => ({
        channel,
        value: canLineLevel(bit, pin),
        source: `${pin} (CAN bus)`,
      })),
    });
    t += 1;
  };

  frames.forEach((frame, index) => {
    if (index > 0) {
      for (let i = 0; i < idleBitsBetween; i += 1) emitBit(1); // recessive idle
    }
    for (const { bit } of serializeCanFrame(frame).bits) emitBit(bit);
  });

  return samples;
}
