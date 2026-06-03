// IO-Link Analyzer — taps the simulated IO-Link master and renders the live
// master↔device protocol as per-cycle transaction rows. Polls the same way as
// the Air Tracer (BleAnalyzer): a useRef'd bridge + interval while running. The
// protocol decode already happened in Rust (IolinkXfer); this only formats.
import { useEffect, useMemo, useRef, useState } from 'react';
import type { IolinkXfer, SimulatorBridge } from '@labwired/ui';
import {
  PHASES,
  annotatePdChanges,
  ckState,
  errorCount,
  filterErrorsOnly,
  kindLabel,
  linkPhaseIndex,
  toCsv,
  toHex,
} from './iolinkDecode';

export interface IoLinkAnalyzerProps {
  bridge: SimulatorBridge | null;
  running: boolean;
  pollMs?: number;
}

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

  const phaseIdx = rows.length ? linkPhaseIndex(rows[rows.length - 1].link_state) : -1;
  const errs = errorCount(rows);
  const com = rows.length ? rows[rows.length - 1].com.toUpperCase() : '—';
  const view = useMemo(() => (errorsOnly ? filterErrorsOnly(rows) : rows), [rows, errorsOnly]);
  const annotatedView = useMemo(() => annotatePdChanges(view), [view]);

  const copy = () => {
    try {
      void navigator.clipboard?.writeText(toCsv(rows));
    } catch {
      /* clipboard unavailable; no-op */
    }
  };

  return (
    <div className="flex flex-col h-full min-h-0 text-fg-primary text-[12px]">
      <div className="flex gap-1 px-3 py-2 border-b border-border">
        {PHASES.map((p, i) => (
          <div
            key={p}
            className={`flex-1 text-center rounded px-1 py-0.5 font-mono text-[10px] ${
              phaseIdx < 0
                ? 'bg-bg-canvas text-fg-tertiary'
                : i < phaseIdx
                  ? 'bg-green-900/40 text-green-400'
                  : i === phaseIdx
                    ? 'bg-green-500 text-black font-bold'
                    : 'bg-bg-canvas text-fg-tertiary'
            }`}
          >
            {p}
          </div>
        ))}
      </div>

      <div className="flex items-center justify-between px-3 py-1.5 border-b border-border font-mono text-[11px]">
        <span className="text-fg-tertiary">
          {rows.length} frame{rows.length === 1 ? '' : 's'} · {com}
          {errs > 0 && <span className="text-red-500"> · {errs} CRC err</span>}
        </span>
        <span className="flex gap-2">
          <button
            type="button"
            className={`px-2 py-0.5 rounded border border-border ${errorsOnly ? 'text-red-500' : 'text-fg-secondary'}`}
            onClick={() => setErrorsOnly((v) => !v)}
          >
            errors only
          </button>
          <button
            type="button"
            className="px-2 py-0.5 rounded border border-border text-fg-secondary"
            onClick={copy}
          >
            ⧉ Copy
          </button>
        </span>
      </div>

      {view.length === 0 ? (
        <div className="flex-1 flex items-center justify-center px-4 text-center text-fg-tertiary text-[12px]">
          {running
            ? 'Waiting for IO-Link traffic… ensure an IO-Link master is wired and running.'
            : 'No transactions yet. Add an IO-Link master and press Run.'}
        </div>
      ) : (
        <div className="flex-1 min-h-0 overflow-auto">
          <table className="w-full border-collapse font-mono text-[11px]">
            <thead className="sticky top-0 bg-bg-base text-fg-secondary">
              <tr className="text-left">
                <th className="px-3 py-1.5 font-medium">#</th>
                <th className="px-3 py-1.5 font-medium">Type</th>
                <th className="px-3 py-1.5 font-medium">PD out</th>
                <th className="px-3 py-1.5 font-medium">PD in</th>
                <th className="px-3 py-1.5 font-medium">CK</th>
                <th className="px-3 py-1.5 font-medium">Link</th>
              </tr>
            </thead>
            <tbody>
              {annotatedView
                .slice()
                .reverse()
                .map(({ row: r, pdInChanged }) => {
                  const ck = ckState(r);
                  const isExpanded = expanded === r.seq;
                  return (
                    <FragmentRow
                      key={r.seq}
                      r={r}
                      ck={ck}
                      pdInChanged={pdInChanged}
                      expanded={isExpanded}
                      onToggle={() => setExpanded(isExpanded ? null : r.seq)}
                    />
                  );
                })}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}

function FragmentRow({
  r,
  ck,
  pdInChanged,
  expanded,
  onToggle,
}: {
  r: IolinkXfer;
  ck: 'ok' | 'bad' | 'na';
  pdInChanged: boolean;
  expanded: boolean;
  onToggle: () => void;
}) {
  return (
    <>
      <tr
        className={`border-t border-border/60 hover:bg-bg-canvas cursor-pointer ${
          ck === 'bad' ? 'bg-red-500/10' : pdInChanged ? 'bg-amber-500/10' : ''
        }`}
        onClick={onToggle}
      >
        <td className="px-3 py-1 text-fg-tertiary">{r.seq}</td>
        <td className="px-3 py-1">{kindLabel(r.kind)}</td>
        <td className="px-3 py-1 text-fg-secondary">{toHex(r.pd_out)}</td>
        <td className={`px-3 py-1 font-semibold ${pdInChanged ? 'text-amber-300' : 'text-fg-primary'}`}>
          {toHex(r.pd_in)}
          {pdInChanged && <span className="ml-2 rounded border border-amber-400/40 px-1 text-[9px] text-amber-300">CHG</span>}
        </td>
        <td className="px-3 py-1 font-semibold">
          {ck === 'na' ? (
            <span className="text-fg-tertiary">—</span>
          ) : ck === 'ok' ? (
            <span className="text-green-500">✓</span>
          ) : (
            <span className="text-red-500">✗</span>
          )}
        </td>
        <td className="px-3 py-1">{r.link_state === 'operate' ? 'OPERATE' : 'STARTUP'}</td>
      </tr>
      {expanded && (
        <tr className="bg-bg-canvas/60">
          <td colSpan={6} className="px-3 py-1.5 text-fg-secondary">
            <div>M→D: {toHex(r.raw_master)}</div>
            <div>D→M: {toHex(r.raw_device)}</div>
          </td>
        </tr>
      )}
    </>
  );
}
