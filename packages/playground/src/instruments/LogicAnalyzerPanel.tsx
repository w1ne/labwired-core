import type { Diagram, SimulatorBridge } from '@labwired/ui';
import { useEffect, useMemo, useRef, useState } from 'react';
import { IoLinkAnalyzer } from './IoLinkAnalyzer';
import { UartAnalyzer } from './UartAnalyzer';
import {
  captureLogicAnalyzerSample,
  getDecoderAvailability,
  type LogicAnalyzerSample,
} from './logicAnalyzerCapture';
import {
  getIolinkDecoderBinding,
  getLogicAnalyzerChannelBindings,
  getUartDecoderBinding,
} from './logicAnalyzerConnections';

export interface LogicAnalyzerPanelProps {
  diagram: Diagram;
  analyzerId: string;
  bridge: SimulatorBridge | null;
  running: boolean;
  decoder: string;
  onDecoderChange: (decoder: string) => void;
  pollMs?: number;
}

const DECODERS = [
  { id: 'raw', label: 'Raw' },
  { id: 'iolink', label: 'IO-Link' },
  { id: 'uart', label: 'UART' },
  { id: 'spi', label: 'SPI' },
] as const;

const MAX_SAMPLES = 240;

export function LogicAnalyzerPanel({
  diagram,
  analyzerId,
  bridge,
  running,
  decoder,
  onDecoderChange,
  pollMs = 80,
}: LogicAnalyzerPanelProps) {
  const [armed, setArmed] = useState(true);
  const [samples, setSamples] = useState<LogicAnalyzerSample[]>([]);
  const bindings = getLogicAnalyzerChannelBindings(diagram, analyzerId);
  const iolink = getIolinkDecoderBinding(diagram, analyzerId);
  const uart = getUartDecoderBinding(diagram, analyzerId);
  const availability = getDecoderAvailability(diagram, analyzerId);
  const decoderId = DECODERS.some((candidate) => candidate.id === decoder) ? decoder : 'raw';
  const bridgeRef = useRef(bridge);
  bridgeRef.current = bridge;

  useEffect(() => {
    setSamples([]);
  }, [analyzerId, diagram]);

  useEffect(() => {
    const capture = () => {
      const b = bridgeRef.current;
      const sample = captureLogicAnalyzerSample({
        diagram,
        analyzerId,
        nowMs: performance.now(),
        getPeripheralSnapshot: (name) => b?.getPeripheralSnapshot(name) ?? null,
      });
      setSamples((prev) => [...prev.slice(-(MAX_SAMPLES - 1)), sample]);
    };

    if (!running || !armed) return;
    capture();
    const id = window.setInterval(capture, pollMs);
    return () => window.clearInterval(id);
  }, [analyzerId, armed, diagram, pollMs, running, bridge]);

  const latest = samples[samples.length - 1] ?? null;
  const sampleRate = useMemo(() => {
    if (samples.length < 2) return '0 Sa/s';
    const dt = samples[samples.length - 1].t - samples[0].t;
    if (dt <= 0) return '0 Sa/s';
    return `${Math.round(((samples.length - 1) * 1000) / dt)} Sa/s`;
  }, [samples]);

  const copyCsv = () => {
    const header = ['t_ms', 'CH0', 'CH1', 'CH2', 'CH3'].join(',');
    const body = samples
      .map((sample) => [
        sample.t.toFixed(1),
        ...['CH0', 'CH1', 'CH2', 'CH3'].map((channel) => {
          const value = sample.channels.find((candidate) => candidate.channel === channel)?.value;
          return value === null || value === undefined ? '' : String(value);
        }),
      ].join(','))
      .join('\n');
    try {
      void navigator.clipboard?.writeText(`${header}\n${body}\n`);
    } catch {
      /* clipboard unavailable; no-op */
    }
  };

  return (
    <div className="flex h-full min-h-0 flex-col text-[12px] text-fg-primary">
      <div className="flex items-center justify-between gap-2 border-b border-border px-3 py-2">
        <div className="flex items-center gap-1">
          {DECODERS.map((candidate) => {
            const available =
              candidate.id === 'raw'
                ? availability.raw
                : candidate.id === 'iolink'
                  ? availability.iolink
                  : candidate.id === 'uart'
                    ? availability.uart
                    : availability.spi;
            return (
              <button
                key={candidate.id}
                type="button"
                disabled={!available}
                onClick={() => onDecoderChange(candidate.id)}
                className={`h-7 rounded border px-2 text-[11px] font-medium ${
                  decoderId === candidate.id
                    ? 'border-accent/50 bg-accent-soft text-accent'
                    : available
                      ? 'border-border text-fg-secondary hover:text-fg-primary'
                      : 'border-border/60 text-fg-tertiary opacity-60'
                }`}
                title={available ? candidate.label : `${candidate.label} needs a compatible connected signal`}
              >
                {candidate.label}
              </button>
            );
          })}
        </div>
        <div className="flex items-center gap-2 font-mono text-[11px]">
          <span className={armed ? 'text-green-500' : 'text-fg-tertiary'}>{armed ? 'ARMED' : 'STOPPED'}</span>
          <span className="text-fg-tertiary">{samples.length} samples</span>
          <span className="text-fg-tertiary">{sampleRate}</span>
          <button
            type="button"
            className="h-7 rounded border border-border px-2 text-fg-secondary hover:text-fg-primary"
            onClick={() => setArmed((value) => !value)}
          >
            {armed ? 'Stop' : 'Arm'}
          </button>
          <button
            type="button"
            className="h-7 rounded border border-border px-2 text-fg-secondary hover:text-fg-primary"
            onClick={() => setSamples([])}
          >
            Clear
          </button>
          <button
            type="button"
            className="h-7 rounded border border-border px-2 text-fg-secondary hover:text-fg-primary"
            onClick={copyCsv}
          >
            CSV
          </button>
        </div>
      </div>

      <div className="border-b border-border px-3 py-2">
        <div className="grid grid-cols-4 gap-1">
          {bindings.map((binding) => {
            const label = binding.endpoints.length
              ? binding.endpoints.map((endpoint) => `${endpoint.part}.${endpoint.pin}`).join(', ')
              : 'not connected';
            const live = latest?.channels.find((channel) => channel.channel === binding.channel);
            return (
              <div key={binding.channel} className="min-w-0 rounded bg-bg-canvas px-2 py-1">
                <div className="flex items-center justify-between font-mono text-[10px] font-semibold text-fg-secondary">
                  <span>{binding.channel}</span>
                  <span className={live?.value === 1 ? 'text-green-500' : live?.value === 0 ? 'text-fg-primary' : 'text-fg-tertiary'}>
                    {live?.value === null || live?.value === undefined ? '-' : live.value}
                  </span>
                </div>
                <div className="truncate font-mono text-[10px] text-fg-primary" title={label}>
                  {label}
                </div>
              </div>
            );
          })}
        </div>
      </div>

      {decoderId === 'raw' ? (
        <RawWaveform samples={samples} />
      ) : decoderId === 'iolink' && iolink.connected ? (
        <>
          <div className="flex items-center justify-between border-b border-border px-3 py-1.5 font-mono text-[11px] text-fg-secondary">
            <span>
              IO-Link decoder armed
            </span>
            <span>
              {iolink.channels.map((channel) => `${channel.channel}:${channel.pin}`).join('  ')}
            </span>
          </div>
          <div className="min-h-0 flex-1">
            <IoLinkAnalyzer bridge={bridge} running={running && armed} />
          </div>
        </>
      ) : decoderId === 'uart' && uart.connected ? (
        <>
          <div className="flex items-center justify-between border-b border-border px-3 py-1.5 font-mono text-[11px] text-fg-secondary">
            <span>UART decoder armed</span>
            <span>{uart.channels.map((channel) => `${channel.channel}:${channel.peripheral}.${channel.role.toUpperCase()}`).join('  ')}</span>
          </div>
          <div className="min-h-0 flex-1">
            <UartAnalyzer bridge={bridge} running={running && armed} binding={uart} iolinkBinding={iolink} />
          </div>
        </>
      ) : (
        <div className="flex flex-1 items-center justify-center px-6 text-center text-[12px] text-fg-tertiary">
          {decoderId === 'iolink'
            ? 'Connect a channel to the IO-Link TX or RX net to decode traffic.'
            : 'This decoder needs core bitstream traces before it can decode selected lines.'}
        </div>
      )}
    </div>
  );
}

function RawWaveform({ samples }: { samples: LogicAnalyzerSample[] }) {
  if (samples.length === 0) {
    return (
      <div className="flex flex-1 items-center justify-center px-6 text-center text-[12px] text-fg-tertiary">
        No samples captured yet.
      </div>
    );
  }

  const width = 540;
  const rowHeight = 52;
  const height = rowHeight * 4;
  const channels = ['CH0', 'CH1', 'CH2', 'CH3'];
  const xFor = (index: number) => (samples.length <= 1 ? 44 : 44 + (index * (width - 58)) / (samples.length - 1));

  return (
    <div className="min-h-0 flex-1 overflow-auto bg-bg-base">
      <svg viewBox={`0 0 ${width} ${height}`} className="h-full min-h-[220px] w-full font-mono text-[10px]">
        {channels.map((channel, channelIndex) => {
          const yBase = channelIndex * rowHeight + 34;
          const points = samples
            .map((sample, sampleIndex) => {
              const value = sample.channels.find((candidate) => candidate.channel === channel)?.value;
              if (value === null || value === undefined) return null;
              return `${xFor(sampleIndex)},${value ? yBase - 22 : yBase}`;
            })
            .filter((point): point is string => point !== null)
            .join(' ');

          return (
            <g key={channel}>
              <text x={12} y={yBase - 8} fill="#8a98a8">{channel}</text>
              <line x1={44} y1={yBase} x2={width - 12} y2={yBase} stroke="#2a3442" strokeWidth={1} />
              <line x1={44} y1={yBase - 22} x2={width - 12} y2={yBase - 22} stroke="#223040" strokeWidth={1} strokeDasharray="3 4" />
              {points ? (
                <polyline points={points} fill="none" stroke="#38bdf8" strokeWidth={2} strokeLinejoin="round" strokeLinecap="round" />
              ) : (
                <text x={52} y={yBase - 8} fill="#566272">not connected</text>
              )}
            </g>
          );
        })}
      </svg>
    </div>
  );
}
