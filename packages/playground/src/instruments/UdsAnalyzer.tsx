import type { FdcanTraceFrame, SimulatorBridge } from '@labwired/ui';
import { useEffect, useMemo, useRef, useState } from 'react';
import type { UdsDecoderBinding } from './logicAnalyzerConnections';
import { rowsForUdsTrace } from './udsTraceDecode';

export interface UdsAnalyzerProps {
  bridge: SimulatorBridge | null;
  running: boolean;
  binding: UdsDecoderBinding;
  pollMs?: number;
}

export function UdsAnalyzer({ bridge, running, binding, pollMs = 200 }: UdsAnalyzerProps) {
  const tracedPeripherals = useMemo(
    () => new Set(binding.channels.map((channel) => channel.peripheral)),
    [binding.channels],
  );
  const filterFrames = (trace: FdcanTraceFrame[]) =>
    trace.filter((frame) => tracedPeripherals.has(frame.peripheral));
  const [frames, setFrames] = useState<FdcanTraceFrame[]>(() => filterFrames(bridge?.fdcanTraceSnapshot() ?? []));
  const bridgeRef = useRef(bridge);
  bridgeRef.current = bridge;

  useEffect(() => {
    let cancelled = false;
    const poll = () => {
      const b = bridgeRef.current;
      if (!b) return;
      try {
        const trace = filterFrames(b.fdcanTraceSnapshot());
        if (!cancelled) setFrames(trace);
      } catch {
        /* bridge may be mid-teardown between Run/Stop; ignore one tick */
      }
    };

    poll();
    if (!running) return;
    const id = window.setInterval(poll, pollMs);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, [running, pollMs, bridge, tracedPeripherals]);

  const rows = useMemo(() => rowsForUdsTrace(frames), [frames]);
  const channelLabel = binding.channels
    .map((channel) => `${channel.channel}:${channel.part}.${channel.pin}`)
    .join('  ');

  return (
    <div className="flex h-full min-h-0 flex-col text-[12px] text-fg-primary">
      <div className="flex items-center justify-between gap-3 border-b border-border px-3 py-1.5 font-mono text-[11px] text-fg-secondary">
        <span>{rows.length} UDS event{rows.length === 1 ? '' : 's'} from {frames.length} CAN frame{frames.length === 1 ? '' : 's'}</span>
        <span className="truncate" title={channelLabel}>{channelLabel}</span>
      </div>
      <div className="border-b border-border px-3 py-2 text-[11px] text-fg-secondary">
        Source: simulator CAN frame trace from the probed CAN_H/CAN_L bus net.
      </div>

      {rows.length === 0 ? (
        <div className="flex flex-1 items-center justify-center px-6 text-center text-[12px] text-fg-tertiary">
          {running ? 'Waiting for CAN frames on the probed CAN bus.' : 'No CAN frames yet. Press Run.'}
        </div>
      ) : (
        <div className="min-h-0 flex-1 overflow-auto">
          <table className="w-full border-collapse font-mono text-[11px]">
            <thead className="sticky top-0 bg-bg-base text-fg-secondary">
              <tr className="text-left">
                <th className="px-3 py-1.5 font-medium">Kind</th>
                <th className="px-3 py-1.5 font-medium">CAN ID</th>
                <th className="px-3 py-1.5 font-medium">SID</th>
                <th className="px-3 py-1.5 font-medium">Decoded</th>
                <th className="px-3 py-1.5 font-medium">Payload</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((row) => (
                <tr key={row.key} className="border-t border-border/60 hover:bg-bg-canvas">
                  <td className="px-3 py-1 font-semibold text-fg-primary">{row.kind}</td>
                  <td className="px-3 py-1 text-fg-secondary">{row.canId}</td>
                  <td className="px-3 py-1 text-fg-secondary">{row.service}</td>
                  <td className="px-3 py-1 text-fg-primary">{row.detail}</td>
                  <td className="px-3 py-1 text-fg-secondary">{row.payload}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
