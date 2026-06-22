// IO-Link Analyzer — taps the simulated IO-Link master and renders the live
// master↔device protocol as per-cycle rows. Moved into @labwired/ui, inline-styled
// + --lw-* themed (self-contained: bridge + running; decode is pure).
import { useEffect, useMemo, useRef, useState, type CSSProperties } from 'react';
import type { IolinkXfer, SimulatorBridge } from '../../wasm/simulator-bridge';
import { PHASES, annotatePdChanges, ckState, errorCount, filterErrorsOnly, kindLabel, linkPhaseIndex, toCsv, toHex } from './iolinkDecode';

export interface IoLinkAnalyzerProps {
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
  ok: '#22c55e', bad: '#ef4444', amber: '#fcd34d',
};
const th: CSSProperties = { padding: '6px 12px', fontWeight: 500, textAlign: 'left' };
const td: CSSProperties = { padding: '4px 12px' };
const btn = (active?: boolean): CSSProperties => ({ padding: '2px 8px', borderRadius: 4, border: `1px solid ${C.border}`, background: 'transparent', color: active ? C.bad : C.fgSecondary, cursor: 'pointer' });

export function IoLinkAnalyzer({ bridge, running, pollMs = 200 }: IoLinkAnalyzerProps) {
  const [rows, setRows] = useState<IolinkXfer[]>([]);
  const [errorsOnly, setErrorsOnly] = useState(false);
  const [expanded, setExpanded] = useState<number | null>(null);
  const bridgeRef = useRef(bridge);
  bridgeRef.current = bridge;

  useEffect(() => {
    let cancelled = false;
    const poll = () => {
      const b = bridgeRef.current;
      if (!b) return;
      try { const t = b.iolinkTraceSnapshot(); if (!cancelled) setRows(t); } catch { /* mid-teardown */ }
    };
    if (!running) return;
    poll();
    const id = window.setInterval(poll, pollMs);
    return () => { cancelled = true; window.clearInterval(id); };
  }, [running, pollMs, bridge]);

  const phaseIdx = rows.length ? linkPhaseIndex(rows[rows.length - 1].link_state) : -1;
  const errs = errorCount(rows);
  const com = rows.length ? rows[rows.length - 1].com.toUpperCase() : '—';
  const view = useMemo(() => (errorsOnly ? filterErrorsOnly(rows) : rows), [rows, errorsOnly]);
  const annotatedView = useMemo(() => annotatePdChanges(view), [view]);
  const copy = () => { try { void navigator.clipboard?.writeText(toCsv(rows)); } catch { /* no clipboard */ } };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%', minHeight: 0, color: C.fgPrimary, fontSize: 12 }}>
      <div style={{ display: 'flex', gap: 4, padding: '8px 12px', borderBottom: `1px solid ${C.border}` }}>
        {PHASES.map((p, i) => {
          const on = phaseIdx >= 0 && i === phaseIdx;
          const past = phaseIdx >= 0 && i < phaseIdx;
          return (
            <div key={p} style={{ flex: 1, textAlign: 'center', borderRadius: 4, padding: '2px 4px', fontFamily: 'ui-monospace, monospace', fontSize: 10, fontWeight: on ? 700 : 400, background: on ? C.ok : C.canvas, color: on ? '#000' : past ? C.ok : C.fgTertiary }}>
              {p}
            </div>
          );
        })}
      </div>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', padding: '6px 12px', borderBottom: `1px solid ${C.border}`, fontFamily: 'ui-monospace, monospace', fontSize: 11 }}>
        <span style={{ color: C.fgTertiary }}>
          {rows.length} frame{rows.length === 1 ? '' : 's'} · {com}
          {errs > 0 && <span style={{ color: C.bad }}> · {errs} CRC err</span>}
        </span>
        <span style={{ display: 'flex', gap: 8 }}>
          <button type="button" style={btn(errorsOnly)} onClick={() => setErrorsOnly((v) => !v)}>errors only</button>
          <button type="button" style={btn()} onClick={copy}>⧉ Copy</button>
        </span>
      </div>
      {view.length === 0 ? (
        <div style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', padding: 16, textAlign: 'center', color: C.fgTertiary }}>
          {running ? 'Waiting for IO-Link traffic… ensure a master is wired and running.' : 'No transactions yet. Add an IO-Link master and press Run.'}
        </div>
      ) : (
        <div style={{ flex: 1, minHeight: 0, overflow: 'auto' }}>
          <table style={{ width: '100%', borderCollapse: 'collapse', fontFamily: 'ui-monospace, monospace', fontSize: 11 }}>
            <thead style={{ position: 'sticky', top: 0, background: C.base, color: C.fgSecondary }}>
              <tr>{['#', 'Type', 'PD out', 'PD in', 'CK', 'Link'].map((h) => <th key={h} style={th}>{h}</th>)}</tr>
            </thead>
            <tbody>
              {annotatedView.slice().reverse().map(({ row: r, pdInChanged }) => {
                const ck = ckState(r);
                const isExpanded = expanded === r.seq;
                return (
                  <FragmentRow key={r.seq} r={r} ck={ck} pdInChanged={pdInChanged} expanded={isExpanded} onToggle={() => setExpanded(isExpanded ? null : r.seq)} />
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}

function FragmentRow({ r, ck, pdInChanged, expanded, onToggle }: { r: IolinkXfer; ck: 'ok' | 'bad' | 'na'; pdInChanged: boolean; expanded: boolean; onToggle: () => void }) {
  const rowBg = ck === 'bad' ? 'rgba(239,68,68,0.10)' : pdInChanged ? 'rgba(245,158,11,0.10)' : 'transparent';
  return (
    <>
      <tr style={{ borderTop: `1px solid ${C.border}`, background: rowBg, cursor: 'pointer' }} onClick={onToggle}>
        <td style={{ ...td, color: C.fgTertiary }}>{r.seq}</td>
        <td style={td}>{kindLabel(r.kind)}</td>
        <td style={{ ...td, color: C.fgSecondary }}>{toHex(r.pd_out)}</td>
        <td style={{ ...td, fontWeight: 600, color: pdInChanged ? C.amber : C.fgPrimary }}>
          {toHex(r.pd_in)}
          {pdInChanged && <span style={{ marginLeft: 8, borderRadius: 3, border: '1px solid rgba(251,191,36,0.4)', padding: '0 4px', fontSize: 9, color: C.amber }}>CHG</span>}
        </td>
        <td style={{ ...td, fontWeight: 600 }}>
          {ck === 'na' ? <span style={{ color: C.fgTertiary }}>—</span> : ck === 'ok' ? <span style={{ color: C.ok }}>✓</span> : <span style={{ color: C.bad }}>✗</span>}
        </td>
        <td style={td}>{r.link_state === 'operate' ? 'OPERATE' : 'STARTUP'}</td>
      </tr>
      {expanded && (
        <tr style={{ background: C.canvas }}>
          <td colSpan={6} style={{ ...td, color: C.fgSecondary }}>
            <div>M→D: {toHex(r.raw_master)}</div>
            <div>D→M: {toHex(r.raw_device)}</div>
          </td>
        </tr>
      )}
    </>
  );
}
