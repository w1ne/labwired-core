// BLE packet analyzer — polls the simulator's shared virtual-air trace and
// renders each frame as a decoded protocol row. Moved from the playground into
// @labwired/ui, inline-styled + --lw-* themed (self-contained: bridge + running).
import { useEffect, useRef, useState, type CSSProperties } from 'react';
import type { SimulatorBridge } from '../../wasm/simulator-bridge';
import { decodeBleTrace, type BleTransaction } from './bleDecode';

export interface BleAnalyzerProps {
  bridge: SimulatorBridge | null;
  running: boolean;
  pollMs?: number;
}

const C = {
  base: 'var(--lw-bg-base, #0A0B0F)',
  canvas: 'var(--lw-bg-canvas, #0E1015)',
  border: 'var(--lw-border, #262A33)',
  fgPrimary: 'var(--lw-fg-primary, #F2F4F9)',
  fgSecondary: 'var(--lw-fg-secondary, #9098A8)',
  fgTertiary: 'var(--lw-fg-tertiary, #5A6178)',
  ok: '#22c55e',
  bad: '#ef4444',
};

const th: CSSProperties = { padding: '6px 12px', fontWeight: 500, textAlign: 'left' };
const td: CSSProperties = { padding: '4px 12px' };

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
        /* bridge mid-teardown; skip a tick */
      }
    };
    poll();
    if (!running) return;
    const id = window.setInterval(poll, pollMs);
    return () => { cancelled = true; window.clearInterval(id); };
  }, [running, pollMs, bridge]);

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%', minHeight: 0, color: C.fgPrimary, fontSize: 12 }}>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'flex-end', padding: '6px 12px', borderBottom: `1px solid ${C.border}` }}>
        <span style={{ color: C.fgTertiary, fontFamily: 'ui-monospace, monospace', fontSize: 11 }}>
          {rows.length} frame{rows.length === 1 ? '' : 's'}
        </span>
      </div>
      {rows.length === 0 ? (
        <div style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', padding: 16, textAlign: 'center', color: C.fgTertiary }}>
          {running ? 'Listening on the virtual air… Run a BLE transmitter to see frames.' : 'No frames captured yet. Add a BLE transmitter and press Run.'}
        </div>
      ) : (
        <div style={{ flex: 1, minHeight: 0, overflow: 'auto' }}>
          <table style={{ width: '100%', borderCollapse: 'collapse', fontFamily: 'ui-monospace, monospace', fontSize: 11 }}>
            <thead style={{ position: 'sticky', top: 0, background: C.base, color: C.fgSecondary }}>
              <tr>
                {['#', 'Freq', 'PHY', 'Address', 'Len', 'CRC', 'Reading', 'Payload (de-whitened)'].map((h) => (
                  <th key={h} style={th}>{h}</th>
                ))}
              </tr>
            </thead>
            <tbody>
              {rows.map((r, i) => (
                <tr key={`${i}-${r.rawHex}`} style={{ borderTop: `1px solid ${C.border}` }} title={`On-air (whitened): ${r.rawHex}`}>
                  <td style={{ ...td, color: C.fgTertiary }}>{rows.length - i}</td>
                  <td style={td}>{r.freqMhz} MHz</td>
                  <td style={td}>{r.phy}</td>
                  <td style={{ ...td, color: C.fgSecondary }}>{r.address}</td>
                  <td style={td}>{r.length ?? '–'}</td>
                  <td style={{ ...td, fontWeight: 600 }}>
                    {r.crcOk === null ? <span style={{ color: C.fgTertiary }}>–</span> : r.crcOk ? <span style={{ color: C.ok }}>OK</span> : <span style={{ color: C.bad }}>BAD</span>}
                  </td>
                  <td style={{ ...td, color: C.fgPrimary, fontWeight: 600 }}>{r.reading ?? '–'}</td>
                  <td style={{ ...td, color: C.fgSecondary, whiteSpace: 'nowrap' }}>{r.hex}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
