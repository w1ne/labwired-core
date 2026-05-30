// BLE packet analyzer — the first instrument in the playground's universal
// analyzer toolset. It polls the simulator's shared virtual-air trace (every
// chip in the WASM instance pushes onto the same ring) and renders each frame
// as a decoded protocol row. Because the air registry is process-global, ONE
// live bridge is enough to observe traffic from every radio on the canvas — so
// this panel works whether you're watching the sensor or the collector.
//
// The captured bytes are the WHITENED on-air frame (what a real sniffer sees);
// the decoder de-whitens them with the frame's IV to recover the logical
// [S0, LENGTH, payload], so the sensor's incrementing Reading is human-visible.
// The raw on-air hex is kept in a tooltip for the literal sniffer view.
import { useEffect, useRef, useState } from 'react';
import type { SimulatorBridge } from '@labwired/ui';
import { decodeBleTrace, type BleTransaction } from './bleDecode';

export interface BleAnalyzerProps {
  /** Any live bridge — they all read the same shared air. */
  bridge: SimulatorBridge | null;
  /** Whether the sim is running; drives the poll cadence. */
  running: boolean;
  /** Poll interval while running, in ms. */
  pollMs?: number;
}

export function BleAnalyzer({ bridge, running, pollMs = 200 }: BleAnalyzerProps) {
  const [rows, setRows] = useState<BleTransaction[]>([]);
  const bridgeRef = useRef(bridge);
  bridgeRef.current = bridge;

  useEffect(() => {
    let cancelled = false;
    const poll = () => {
      const b = bridgeRef.current;
      if (!b) return;
      try {
        const trace = b.airTraceSnapshot();
        if (!cancelled) setRows(decodeBleTrace(trace));
      } catch {
        /* bridge may be mid-teardown between Run/Stop; ignore one tick */
      }
    };
    poll(); // immediate read so a stopped sim still shows its last frames
    if (!running) return;
    const id = window.setInterval(poll, pollMs);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, [running, pollMs, bridge]);

  return (
    <div className="flex flex-col h-full min-h-0 text-fg-primary text-[12px]">
      <div className="flex items-center justify-between px-3 py-2 border-b border-border">
        <span className="font-semibold tracking-tight">Packet Analyzer · BLE air</span>
        <span className="text-fg-tertiary font-mono text-[11px]">
          {rows.length} frame{rows.length === 1 ? '' : 's'}
        </span>
      </div>

      {rows.length === 0 ? (
        <div className="flex-1 flex items-center justify-center px-4 text-center text-fg-tertiary text-[12px]">
          {running
            ? 'Listening on the virtual air… Run a BLE transmitter to see frames.'
            : 'No frames captured yet. Add the BLE Sensor + Collector and press Run.'}
        </div>
      ) : (
        <div className="flex-1 min-h-0 overflow-auto">
          <table className="w-full border-collapse font-mono text-[11px]">
            <thead className="sticky top-0 bg-bg-base text-fg-secondary">
              <tr className="text-left">
                <th className="px-3 py-1.5 font-medium">#</th>
                <th className="px-3 py-1.5 font-medium">Freq</th>
                <th className="px-3 py-1.5 font-medium">PHY</th>
                <th className="px-3 py-1.5 font-medium">Address</th>
                <th className="px-3 py-1.5 font-medium">Len</th>
                <th className="px-3 py-1.5 font-medium">Reading</th>
                <th className="px-3 py-1.5 font-medium">Payload (de-whitened)</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((r, i) => (
                <tr
                  key={`${i}-${r.rawHex}`}
                  className="border-t border-border/60 hover:bg-bg-canvas"
                  title={`On-air (whitened): ${r.rawHex}`}
                >
                  <td className="px-3 py-1 text-fg-tertiary">{rows.length - i}</td>
                  <td className="px-3 py-1">{r.freqMhz} MHz</td>
                  <td className="px-3 py-1">{r.phy}</td>
                  <td className="px-3 py-1 text-fg-secondary">{r.address}</td>
                  <td className="px-3 py-1">{r.length ?? '–'}</td>
                  <td className="px-3 py-1 text-fg-primary font-semibold">{r.reading ?? '–'}</td>
                  <td className="px-3 py-1 text-fg-secondary whitespace-nowrap">{r.hex}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
