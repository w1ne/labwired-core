import { useEffect } from 'react';
import clsx from 'clsx';

export type SimState = 'idle' | 'building' | 'running' | 'paused' | 'halted';

export interface SimDockProps {
  state: SimState;
  /** Reserved for future use; not currently rendered (cycles is the real metric). */
  runtimeMs?: number;
  cycles?: number;
  pc?: number;
  onRun: () => void;
  onPause: () => void;
  onStep: () => void;
  onReset: () => void;
}

function formatCycles(n: number): string {
  if (n < 1_000) return n.toString();
  if (n < 1_000_000) return `${(n / 1_000).toFixed(n < 10_000 ? 1 : 0)}K`;
  if (n < 1_000_000_000) return `${(n / 1_000_000).toFixed(n < 10_000_000 ? 2 : 1)}M`;
  return `${(n / 1_000_000_000).toFixed(1)}B`;
}

function formatPc(pc: number): string {
  return `0x${pc.toString(16).toUpperCase().padStart(8, '0')}`;
}

const STATE_LABEL: Record<SimState, string> = {
  idle: 'Idle',
  building: 'Building',
  running: 'Running',
  paused: 'Paused',
  halted: 'Halted',
};

export function SimDock({ state, cycles, pc, onRun, onPause, onStep, onReset }: SimDockProps) {
  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      // Skip when focus is in an editable element
      const target = event.target as HTMLElement | null;
      if (
        target instanceof HTMLInputElement ||
        target instanceof HTMLTextAreaElement ||
        target?.isContentEditable
      ) {
        return;
      }
      if (event.key === ' ') {
        event.preventDefault();
        if (state === 'running') {
          onPause();
        } else {
          onRun();
        }
      } else if (event.key.toLowerCase() === 's' && state === 'paused') {
        event.preventDefault();
        onStep();
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [state, onRun, onPause, onStep]);

  const isRunning = state === 'running';
  const isPaused = state === 'paused';

  const showCycles = cycles !== undefined && cycles > 0;
  const showPc = pc !== undefined && pc > 0;

  return (
    <div
      className="lw-glass absolute bottom-4 left-1/2 -translate-x-1/2 z-20 h-12 px-4 flex items-center gap-3 min-w-[560px]"
      role="toolbar"
      aria-label="Simulation controls"
    >
      <button
        type="button"
        onClick={isRunning ? onPause : onRun}
        aria-label={isRunning ? 'Pause' : 'Run'}
        style={{ borderRadius: 999 }}
        className={clsx(
          'h-8 px-4 font-medium text-[13px] transition-all duration-micro flex items-center gap-2 outline-none',
          isRunning ? 'bg-magenta text-bg-base hover:opacity-90' : 'bg-accent text-bg-base hover:bg-accent-hover'
        )}
      >
        <span aria-hidden>{isRunning ? '⏸' : '▶'}</span>
        {isRunning ? 'Pause' : 'Run'}
      </button>
      <button
        type="button"
        onClick={onStep}
        disabled={!isPaused}
        aria-label="Step"
        style={{ borderRadius: 999 }}
        className="h-8 w-8 bg-white/[0.05] hover:bg-white/[0.10] text-fg-secondary hover:text-fg-primary disabled:opacity-40 disabled:cursor-not-allowed outline-none border-0"
      >
        ⏵
      </button>
      <button
        type="button"
        onClick={onReset}
        aria-label="Reset"
        style={{ borderRadius: 999 }}
        className="h-8 w-8 bg-white/[0.05] hover:bg-white/[0.10] text-fg-secondary hover:text-fg-primary outline-none border-0"
      >
        ↻
      </button>
      <div className="flex-1" />
      {showPc && (
        <span
          className="text-fg-tertiary font-mono text-[11px]"
          title="Program counter — exact silicon-parity instruction address"
        >
          PC <span className="text-fg-secondary">{formatPc(pc!)}</span>
        </span>
      )}
      {showCycles && (
        <>
          <div className="w-px h-4 bg-border" aria-hidden />
          <span
            className="text-fg-tertiary font-mono text-[11px]"
            title="Cycles executed — deterministic, reproducible across runs"
          >
            <span className="text-fg-secondary">{formatCycles(cycles!)}</span> cycles
          </span>
        </>
      )}
      <a
        href="https://github.com/w1ne/labwired#-agent-first-architecture"
        target="_blank"
        rel="noopener noreferrer"
        className="hidden md:inline-flex items-center gap-1 text-[10px] font-medium text-fg-tertiary hover:text-accent transition-colors duration-micro uppercase tracking-[0.06em]"
        title="Cycle-accurate vs. real silicon — read the HIL-displacement showcase"
      >
        <span aria-hidden className="text-ok">✓</span>
        Cycle-accurate
      </a>
      <div className="w-px h-4 bg-border" aria-hidden />
      <div className="flex items-center gap-2 shrink-0">
        <span
          className={clsx('w-2 h-2 rounded-full', isRunning ? 'bg-magenta animate-pulse' : 'bg-fg-tertiary')}
          aria-hidden
        />
        <span className="text-fg-secondary text-[12px]">{STATE_LABEL[state]}</span>
      </div>
    </div>
  );
}
