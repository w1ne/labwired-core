import { CSSProperties } from 'react';

export interface SimControlsProps {
  running: boolean;
  onPlay: () => void;
  onPause: () => void;
  onStep: () => void;
  onReset: () => void;
  /** Current program counter value. */
  pc?: number;
  /** Total cycles executed. */
  cycles?: number;
  style?: CSSProperties;
}

export function SimControls({
  running,
  onPlay,
  onPause,
  onStep,
  onReset,
  pc,
  cycles,
  style,
}: SimControlsProps) {
  return (
    <div style={{
      display: 'flex',
      alignItems: 'center',
      gap: '0.75rem',
      padding: '0.5rem 1rem',
      background: 'var(--lw-bg, #fff)',
      border: 'var(--lw-border, 2px solid #000)',
      borderRadius: 'var(--lw-radius-sm, 8px)',
      boxShadow: 'var(--lw-shadow, 4px 4px 0px #000)',
      fontFamily: 'var(--lw-font-mono, monospace)',
      fontSize: '0.8rem',
      ...style,
    }}>
      {running ? (
        <ControlButton onClick={onPause} title="Pause">
          <PauseIcon />
        </ControlButton>
      ) : (
        <ControlButton onClick={onPlay} title="Run">
          <PlayIcon />
        </ControlButton>
      )}
      <ControlButton onClick={onStep} title="Step" disabled={running}>
        <StepIcon />
      </ControlButton>
      <ControlButton onClick={onReset} title="Reset">
        <ResetIcon />
      </ControlButton>

      {(pc !== undefined || cycles !== undefined) && (
        <div style={{
          marginLeft: 'auto',
          display: 'flex',
          gap: '1rem',
          color: 'var(--lw-gray, #444)',
          fontSize: '0.75rem',
        }}>
          {pc !== undefined && <span>PC: 0x{pc.toString(16).toUpperCase().padStart(8, '0')}</span>}
          {cycles !== undefined && <span>Cycles: {cycles.toLocaleString()}</span>}
        </div>
      )}
    </div>
  );
}

function ControlButton({ onClick, title, disabled, children }: {
  onClick: () => void;
  title: string;
  disabled?: boolean;
  children: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      title={title}
      disabled={disabled}
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        justifyContent: 'center',
        width: 32,
        height: 32,
        padding: 0,
        background: disabled ? 'var(--lw-bg-alt, #f8f9fa)' : 'var(--lw-black, #000)',
        color: disabled ? 'var(--lw-gray, #444)' : 'var(--lw-bg, #fff)',
        border: 'var(--lw-border, 2px solid #000)',
        borderRadius: 6,
        cursor: disabled ? 'not-allowed' : 'pointer',
        boxShadow: disabled ? 'none' : '2px 2px 0px #000',
        textTransform: 'none' as const,
        letterSpacing: 'normal',
        fontSize: '0.8rem',
      }}
    >
      {children}
    </button>
  );
}

function PlayIcon() {
  return (
    <svg width="14" height="14" viewBox="0 0 14 14" fill="currentColor">
      <polygon points="2,0 14,7 2,14" />
    </svg>
  );
}

function PauseIcon() {
  return (
    <svg width="14" height="14" viewBox="0 0 14 14" fill="currentColor">
      <rect x="1" y="0" width="4" height="14" />
      <rect x="9" y="0" width="4" height="14" />
    </svg>
  );
}

function StepIcon() {
  return (
    <svg width="14" height="14" viewBox="0 0 14 14" fill="currentColor">
      <polygon points="0,0 8,7 0,14" />
      <rect x="10" y="0" width="3" height="14" />
    </svg>
  );
}

function ResetIcon() {
  return (
    <svg width="14" height="14" viewBox="0 0 14 14" fill="currentColor">
      <path d="M7 1a6 6 0 1 0 6 6h-2a4 4 0 1 1-4-4V1L3 4l4 3V5a2 2 0 1 0 2 2h2A4 4 0 0 0 7 1z"
        transform="translate(0, 1)" />
    </svg>
  );
}
