// The per-chip control surface: Run/Pause · Upload · Restart + a status
// dot. A chip is just a component whose property panel carries a control
// surface — these are the INTRINSIC verbs that live "inside" the chip
// (firmware lifecycle), distinct from external instruments you wire to it.
//
// Rendered in two places (Stage 1): the anchored on-canvas toolbar next to
// a selected chip, and the inspector/drawer header. `variant` only tweaks
// sizing so the same component serves both.
import { useRef } from 'react';
import clsx from 'clsx';
import type { SimState } from '../studio/SimDock';

const ACCEPT = '.elf,.bin,.hex,.uf2,application/octet-stream';

const STATUS_COLOR: Record<SimState, string> = {
  idle: 'bg-fg-tertiary',
  building: 'bg-amber-400 animate-pulse',
  running: 'bg-green-400',
  paused: 'bg-amber-400',
  halted: 'bg-red-400',
};

export interface ChipControlsProps {
  state: SimState;
  /** Run/Pause/Restart enabled — true once the chip has a runnable config. */
  canRun: boolean;
  /** Upload enabled — true for any MCU (booting an ELF is how a chip starts). */
  canUpload?: boolean;
  onRun: () => void;
  onPause: () => void;
  onRestart: () => void;
  onUpload: (file: File) => void;
  /** Tooltip shown on the disabled Run/Restart controls when `canRun` is false. */
  disabledReason?: string;
  variant?: 'toolbar' | 'header';
}

export function ChipControls({
  state,
  canRun,
  canUpload = true,
  onRun,
  onPause,
  onRestart,
  onUpload,
  disabledReason,
  variant = 'toolbar',
}: ChipControlsProps) {
  const fileInputRef = useRef<HTMLInputElement>(null);
  const isRunning = state === 'running';
  const isBusy = state === 'building';
  const reason = canRun ? undefined : disabledReason;

  const btn = clsx(
    'inline-flex items-center justify-center rounded-md border border-border',
    'text-fg-secondary hover:text-fg-primary hover:bg-bg-elevated',
    'disabled:opacity-40 disabled:cursor-not-allowed disabled:hover:bg-transparent',
    'transition-colors',
    variant === 'toolbar' ? 'h-7 w-7 text-[13px]' : 'h-6 w-6 text-[12px]',
  );

  return (
    <div
      className={clsx(
        'flex items-center gap-1',
        variant === 'toolbar' &&
          'rounded-lg border border-border bg-bg-surface/95 px-1.5 py-1 shadow-lg backdrop-blur-sm',
      )}
      role="toolbar"
      aria-label="Chip controls"
    >
      <span
        className={clsx('mr-0.5 h-2 w-2 shrink-0 rounded-full', STATUS_COLOR[state])}
        aria-label={`Status: ${state}`}
        title={`Status: ${state}`}
      />

      {/* Run / Pause toggle */}
      <button
        type="button"
        className={btn}
        disabled={!canRun || isBusy}
        title={reason ?? (isRunning ? 'Pause' : 'Run firmware')}
        aria-label={isRunning ? 'Pause' : 'Run firmware'}
        onClick={isRunning ? onPause : onRun}
      >
        {isRunning ? '⏸' : '▶'}
      </button>

      {/* Upload firmware — always available per chip (this is how a chip boots) */}
      <button
        type="button"
        className={btn}
        disabled={!canUpload}
        title="Upload firmware (.elf / .bin / .hex / .uf2)"
        aria-label="Upload firmware"
        onClick={() => fileInputRef.current?.click()}
      >
        ⬆
      </button>

      {/* Restart the chip (re-launch its sim from reset) */}
      <button
        type="button"
        className={btn}
        disabled={!canRun}
        title={reason ?? 'Restart chip'}
        aria-label="Restart chip"
        onClick={onRestart}
      >
        ↻
      </button>

      <input
        ref={fileInputRef}
        type="file"
        accept={ACCEPT}
        className="hidden"
        onChange={(e) => {
          const file = e.target.files?.[0];
          if (file) onUpload(file);
          // Reset so re-selecting the same file fires onChange again.
          e.target.value = '';
        }}
      />
    </div>
  );
}
