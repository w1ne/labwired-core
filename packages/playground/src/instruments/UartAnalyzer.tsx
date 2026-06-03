import type { IolinkXfer, SimulatorBridge } from '@labwired/ui';
import { useEffect, useMemo, useRef, useState } from 'react';
import type { IolinkDecoderBinding, UartDecoderBinding } from './logicAnalyzerConnections';
import { annotatePdChanges, kindLabel, toHex } from './iolinkDecode';

export interface UartAnalyzerProps {
  bridge: SimulatorBridge | null;
  running: boolean;
  binding: UartDecoderBinding;
  iolinkBinding: IolinkDecoderBinding;
  pollMs?: number;
}

export function UartAnalyzer({ bridge, running, binding, iolinkBinding, pollMs = 200 }: UartAnalyzerProps) {
  const [rows, setRows] = useState<IolinkXfer[]>([]);
  const bridgeRef = useRef(bridge);
  bridgeRef.current = bridge;

  useEffect(() => {
    let cancelled = false;
    const poll = () => {
      const b = bridgeRef.current;
      if (!b) return;
      try {
        const trace = b.iolinkTraceSnapshot();
        if (!cancelled) setRows(trace);
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

  const annotatedRows = useMemo(() => annotatePdChanges(rows), [rows]);
  const hasIolinkTraceSource = iolinkBinding.connected;
  const channelLabel = binding.channels
    .map((channel) => `${channel.channel}:${channel.peripheral}.${channel.role.toUpperCase()}`)
    .join('  ');

  if (!hasIolinkTraceSource) {
    return (
      <div className="flex h-full min-h-0 flex-col text-[12px] text-fg-primary">
        <div className="border-b border-border px-3 py-1.5 font-mono text-[11px] text-fg-secondary">
          {channelLabel}
        </div>
        <div className="flex flex-1 items-center justify-center px-6 text-center text-[12px] text-fg-tertiary">
          UART line selected. A non-consuming core UART trace source is required for decoded bytes.
        </div>
      </div>
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col text-[12px] text-fg-primary">
      <div className="flex items-center justify-between border-b border-border px-3 py-1.5 font-mono text-[11px] text-fg-secondary">
        <span>{rows.length} UART frame{rows.length === 1 ? '' : 's'}</span>
        <span>{channelLabel}</span>
      </div>

      {annotatedRows.length === 0 ? (
        <div className="flex flex-1 items-center justify-center px-6 text-center text-[12px] text-fg-tertiary">
          {running ? 'Waiting for UART bytes on the selected lines.' : 'No UART bytes yet. Press Run.'}
        </div>
      ) : (
        <div className="min-h-0 flex-1 overflow-auto">
          <table className="w-full border-collapse font-mono text-[11px]">
            <thead className="sticky top-0 bg-bg-base text-fg-secondary">
              <tr className="text-left">
                <th className="px-3 py-1.5 font-medium">#</th>
                <th className="px-3 py-1.5 font-medium">TX bytes</th>
                <th className="px-3 py-1.5 font-medium">RX bytes</th>
                <th className="px-3 py-1.5 font-medium">PD in</th>
                <th className="px-3 py-1.5 font-medium">Type</th>
                <th className="px-3 py-1.5 font-medium">Link</th>
              </tr>
            </thead>
            <tbody>
              {annotatedRows
                .slice()
                .reverse()
                .map(({ row, pdInChanged }) => (
                  <tr
                    key={row.seq}
                    className={`border-t border-border/60 hover:bg-bg-canvas ${pdInChanged ? 'bg-amber-500/10' : ''}`}
                  >
                    <td className="px-3 py-1 text-fg-tertiary">{row.seq}</td>
                    <td className="px-3 py-1 text-fg-secondary">{toHex(row.raw_master)}</td>
                    <td className="px-3 py-1 font-semibold text-fg-primary">{toHex(row.raw_device)}</td>
                    <td className={`px-3 py-1 font-semibold ${pdInChanged ? 'text-amber-300' : 'text-fg-primary'}`}>
                      {toHex(row.pd_in)}
                      {pdInChanged && (
                        <span className="ml-2 rounded border border-amber-400/40 px-1 text-[9px] text-amber-300">CHG</span>
                      )}
                    </td>
                    <td className="px-3 py-1">{kindLabel(row.kind)}</td>
                    <td className="px-3 py-1">{row.link_state === 'operate' ? 'OPERATE' : 'STARTUP'}</td>
                  </tr>
                ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
