// Simulation control dock — Run/Pause/Step/Reset + telemetry (runtime/PC/cycles)
// + state pill. Moved from the playground into @labwired/ui, inline-styled and
// --lw-* themed (self-contained: pure props, no bridge/diagram dependency).
import { useEffect, type CSSProperties } from 'react';

export type SimState = 'idle' | 'building' | 'running' | 'paused' | 'halted';

export interface SimDockProps {
  state: SimState;
  runtimeMs?: number;
  cycles?: number;
  pc?: number;
  onRun: () => void;
  onPause: () => void;
  onStep: () => void;
  onReset: () => void;
}

const C = {
  base: 'var(--lw-bg-base, #0A0B0F)',
  surface: 'var(--lw-bg-surface, #13151B)',
  border: 'var(--lw-border, #262A33)',
  accent: 'var(--lw-accent, #5B9DFF)',
  accentHover: 'var(--lw-accent-hover, #7DB1FF)',
  magenta: 'var(--lw-accent-2, #e83e8c)',
  fgPrimary: 'var(--lw-fg-primary, #F2F4F9)',
  fgSecondary: 'var(--lw-fg-secondary, #9098A8)',
  fgTertiary: 'var(--lw-fg-tertiary, #5A6178)',
};

const fmtCycles = (n: number): string =>
  n < 1_000 ? `${n}` : n < 1_000_000 ? `${(n / 1e3).toFixed(n < 1e4 ? 1 : 0)}K`
  : n < 1_000_000_000 ? `${(n / 1e6).toFixed(n < 1e7 ? 2 : 1)}M` : `${(n / 1e9).toFixed(1)}B`;
const fmtPc = (pc: number) => `0x${pc.toString(16).toUpperCase().padStart(8, '0')}`;
const fmtRt = (ms: number) => {
  const s = Math.max(0, Math.floor(ms / 1000));
  return `${String(Math.floor(s / 60)).padStart(2, '0')}:${String(s % 60).padStart(2, '0')}`;
};
const STATE_LABEL: Record<SimState, string> = { idle: 'Idle', building: 'Building', running: 'Running', paused: 'Paused', halted: 'Halted' };

export function SimDock({ state, runtimeMs = 0, cycles, pc, onRun, onPause, onStep, onReset }: SimDockProps) {
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null;
      if (t instanceof HTMLInputElement || t instanceof HTMLTextAreaElement || t?.isContentEditable) return;
      if (e.key === ' ') { e.preventDefault(); state === 'running' ? onPause() : onRun(); }
      else if (e.key.toLowerCase() === 's' && state === 'paused') { e.preventDefault(); onStep(); }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [state, onRun, onPause, onStep]);

  const isRunning = state === 'running';
  const isPaused = state === 'paused';
  const dock: CSSProperties = {
    height: 48, padding: '0 16px', display: 'flex', alignItems: 'center', gap: 12, maxWidth: '96vw', minWidth: 520,
    borderRadius: 12, border: `1px solid ${C.border}`, background: C.surface, backdropFilter: 'blur(12px)', boxShadow: '0 8px 30px rgba(0,0,0,0.4)',
  };
  const primaryBtn: CSSProperties = {
    height: 32, padding: '0 16px', borderRadius: 999, border: 'none', cursor: 'pointer', fontSize: 13, fontWeight: 500,
    display: 'flex', alignItems: 'center', gap: 8, background: isRunning ? C.magenta : C.accent, color: C.base,
  };
  const ghostBtn = (disabled?: boolean): CSSProperties => ({
    height: 32, width: 32, borderRadius: 999, border: 'none', background: 'rgba(255,255,255,0.05)',
    color: C.fgSecondary, cursor: disabled ? 'not-allowed' : 'pointer', opacity: disabled ? 0.4 : 1,
  });
  const telem: CSSProperties = { fontFamily: 'ui-monospace, monospace', fontSize: 11, color: C.fgTertiary };
  const sep: CSSProperties = { width: 1, height: 16, background: C.border };

  return (
    <div style={dock} role="toolbar" aria-label="Simulation controls">
      <button type="button" onClick={isRunning ? onPause : onRun} aria-label={isRunning ? 'Pause' : 'Run'} style={primaryBtn}>
        <span aria-hidden>{isRunning ? '⏸' : '▶'}</span>{isRunning ? 'Pause' : 'Run'}
      </button>
      <button type="button" onClick={onStep} disabled={!isPaused} aria-label="Step" style={ghostBtn(!isPaused)}>⏵</button>
      <button type="button" onClick={onReset} aria-label="Reset" style={ghostBtn()}>↻</button>
      <div style={{ flex: 1 }} />
      {runtimeMs > 0 && <span style={telem}><span style={{ color: C.fgSecondary }}>{fmtRt(runtimeMs)}</span></span>}
      {pc !== undefined && pc > 0 && (<><span style={sep} aria-hidden /><span style={telem}>PC <span style={{ color: C.fgSecondary }}>{fmtPc(pc)}</span></span></>)}
      {cycles !== undefined && cycles > 0 && (<><span style={sep} aria-hidden /><span style={telem}><span style={{ color: C.fgSecondary }}>{fmtCycles(cycles)}</span> cycles</span></>)}
      <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
        <span style={{ width: 8, height: 8, borderRadius: 999, background: isRunning ? C.magenta : C.fgTertiary }} aria-hidden />
        <span style={{ color: C.fgSecondary, fontSize: 12 }}>{STATE_LABEL[state]}</span>
      </div>
    </div>
  );
}
