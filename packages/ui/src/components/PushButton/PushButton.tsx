import { CSSProperties, useCallback } from 'react';

export interface PushButtonProps {
  /** Binding ID from board_io config. */
  id: string;
  /** Whether the button is currently pressed. */
  pressed: boolean;
  /** Called when the user presses/releases the button. */
  onToggle: (id: string, pressed: boolean) => void;
  /** Label shown next to the button. */
  label?: string;
  /** Size in pixels. Default: 28. */
  size?: number;
  style?: CSSProperties;
}

export function PushButton({ id, pressed, onToggle, label, size = 28, style }: PushButtonProps) {
  const handleMouseDown = useCallback(() => onToggle(id, true), [id, onToggle]);
  const handleMouseUp = useCallback(() => onToggle(id, false), [id, onToggle]);

  return (
    <div style={{ display: 'inline-flex', alignItems: 'center', gap: 8, ...style }}>
      <svg
        width={size}
        height={size}
        viewBox="0 0 28 28"
        style={{ cursor: 'pointer' }}
        onMouseDown={handleMouseDown}
        onMouseUp={handleMouseUp}
        onMouseLeave={() => pressed && onToggle(id, false)}
      >
        <rect
          x="2" y="2" width="24" height="24" rx="4"
          fill={pressed ? 'var(--lw-gray, #444)' : 'var(--lw-bg-alt, #f8f9fa)'}
          stroke="var(--lw-black, #000)"
          strokeWidth="2"
        />
        <rect
          x="6" y="6" width="16" height="16" rx="2"
          fill={pressed ? 'var(--lw-black, #000)' : 'var(--lw-gray-light, #888)'}
          stroke="var(--lw-black, #000)"
          strokeWidth="1"
        />
      </svg>
      {label && (
        <span style={{
          fontFamily: 'var(--lw-font-mono, monospace)',
          fontSize: '0.7rem',
          color: 'var(--lw-gray, #444)',
        }}>
          {label}
        </span>
      )}
    </div>
  );
}
