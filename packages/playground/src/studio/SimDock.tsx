import { useEffect } from 'react';
import clsx from 'clsx';

export type SimState = 'idle' | 'building' | 'running' | 'paused' | 'halted';

export interface SimDockProps {
  state: SimState;
  runtimeMs: number;
  onRun: () => void;
  onPause: () => void;
  onStep: () => void;
  onReset: () => void;
}

const STATE_LABEL: Record<SimState, string> = {
  idle: 'Idle',
  building: 'Building',
  running: 'Running',
  paused: 'Paused',
  halted: 'Halted',
};

function formatRuntime(ms: number): string {
  const totalSeconds = Math.max(0, Math.floor(ms / 1000));
  const mm = String(Math.floor(totalSeconds / 60)).padStart(2, '0');
  const ss = String(totalSeconds % 60).padStart(2, '0');
  return `${mm}:${ss}`;
}

export function SimDock({ state, runtimeMs, onRun, onPause, onStep, onReset }: SimDockProps) {
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

  return (
    <div
      className="lw-glass absolute bottom-4 left-1/2 -translate-x-1/2 z-20 h-12 px-4 flex items-center gap-3 min-w-[480px]"
      role="toolbar"
      aria-label="Simulation controls"
    >
      <button
        type="button"
        onClick={isRunning ? onPause : onRun}
        aria-label={isRunning ? 'Pause' : 'Run'}
        className={clsx(
          'h-8 px-3 rounded-button font-medium transition-colors duration-micro flex items-center gap-2',
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
        className="h-8 w-8 rounded-button border border-border text-fg-secondary hover:text-fg-primary disabled:opacity-40 disabled:cursor-not-allowed"
      >
        ⏵
      </button>
      <button
        type="button"
        onClick={onReset}
        aria-label="Reset"
        className="h-8 w-8 rounded-button border border-border text-fg-secondary hover:text-fg-primary"
      >
        ↻
      </button>
      <div className="flex-1" />
      <span className="text-fg-secondary font-mono text-[12px]">{formatRuntime(runtimeMs)}</span>
      <div className="w-px h-5 bg-border" />
      <div className="flex items-center gap-2">
        <span
          className={clsx('w-2 h-2 rounded-full', isRunning ? 'bg-magenta animate-pulse' : 'bg-fg-tertiary')}
          aria-hidden
        />
        <span className="text-fg-secondary text-[12px]">{STATE_LABEL[state]}</span>
      </div>
    </div>
  );
}
