import type { SimulatorBridge, UartTraceSnapshot } from '@labwired/ui';
import { useEffect, useMemo, useRef, useState } from 'react';
import type { UartDecoderBinding } from './logicAnalyzerConnections';
import { toHex } from './iolinkDecode';
import { rowsForUartTrace } from './uartTraceDecode';

export interface UartAnalyzerProps {
  bridge: SimulatorBridge | null;
  running: boolean;
  binding: UartDecoderBinding;
  pollMs?: number;
}

export function UartAnalyzer({ bridge, running, binding, pollMs = 200 }: UartAnalyzerProps) {
  const [snapshots, setSnapshots] = useState<UartTraceSnapshot[]>([]);
  const bridgeRef = useRef(bridge);
  bridgeRef.current = bridge;

  useEffect(() => {
    let cancelled = false;
    const poll = () => {
      const b = bridgeRef.current;
      if (!b) return;
      try {
        const trace = b.uartTraceSnapshot();
        if (!cancelled) setSnapshots(trace);
      } catch {
        /* bridge may be mid-teardown between Run/Stop; ignore one tick */
      }
    };

    if (!running) return;
    poll();
    const id = window.setInterval(poll, pollMs);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, [running, pollMs, bridge]);

  const rows = useMemo(() => rowsForUartTrace(snapshots, binding), [snapshots, binding]);
  const channelLabel = binding.channels
    .map((channel) => `${channel.channel}:${channel.peripheral}.${channel.role.toUpperCase()}`)
    .join('  ');

  return (
    <div className="flex h-full min-h-0 flex-col text-[12px] text-fg-primary">
      <div className="flex items-center justify-between border-b border-border px-3 py-1.5 font-mono text-[11px] text-fg-secondary">
        <span>{rows.length} UART byte{rows.length === 1 ? '' : 's'}</span>
        <span>{channelLabel}</span>
      </div>

      {rows.length === 0 ? (
        <div className="flex flex-1 items-center justify-center px-6 text-center text-[12px] text-fg-tertiary">
          {running ? 'Waiting for UART bytes on the selected lines.' : 'No UART bytes yet. Press Run.'}
        </div>
      ) : (
        <div className="min-h-0 flex-1 overflow-auto">
          <table className="w-full border-collapse font-mono text-[11px]">
            <thead className="sticky top-0 bg-bg-base text-fg-secondary">
              <tr className="text-left">
                <th className="px-3 py-1.5 font-medium">#</th>
                <th className="px-3 py-1.5 font-medium">CH</th>
                <th className="px-3 py-1.5 font-medium">UART</th>
                <th className="px-3 py-1.5 font-medium">Dir</th>
                <th className="px-3 py-1.5 font-medium">Byte</th>
              </tr>
            </thead>
            <tbody>
              {rows
                .slice()
                .reverse()
                .map((row) => (
                  <tr key={row.key} className="border-t border-border/60 hover:bg-bg-canvas">
                    <td className="px-3 py-1 text-fg-tertiary">{row.seq}</td>
                    <td className="px-3 py-1 text-fg-secondary">{row.channel}</td>
                    <td className="px-3 py-1 text-fg-secondary">{row.peripheral}</td>
                    <td className="px-3 py-1 font-semibold text-fg-primary">{row.direction.toUpperCase()}</td>
                    <td className="px-3 py-1 font-semibold text-fg-primary">{toHex([row.byte])}</td>
                  </tr>
                ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
