import { CSSProperties } from 'react';

export interface LedProps {
  /** Whether the LED is active (lit). */
  active: boolean;
  /** LED color when active. Default: '#ff3333' (red). */
  color?: string;
  /** Size in pixels. Default: 20. */
  size?: number;
  /** Label shown below the LED. */
  label?: string;
  style?: CSSProperties;
}

export function Led({ active, color = '#ff3333', size = 20, label, style }: LedProps) {
  const darkColor = darken(color, 0.6);

  return (
    <div style={{ display: 'inline-flex', flexDirection: 'column', alignItems: 'center', gap: 4, ...style }}>
      <svg width={size} height={size} viewBox="0 0 20 20">
        <defs>
          {active && (
            <radialGradient id={`led-glow-${color}`}>
              <stop offset="0%" stopColor={color} stopOpacity="0.8" />
              <stop offset="100%" stopColor={color} stopOpacity="0" />
            </radialGradient>
          )}
        </defs>
        {active && (
          <circle cx="10" cy="10" r="10" fill={`url(#led-glow-${color})`} opacity="0.5" />
        )}
        <circle
          cx="10"
          cy="10"
          r="7"
          fill={active ? color : darkColor}
          stroke="var(--lw-black, #000)"
          strokeWidth="1.5"
        />
        {active && (
          <circle cx="8" cy="8" r="2" fill="rgba(255,255,255,0.4)" />
        )}
      </svg>
      {label && (
        <span style={{
          fontFamily: 'var(--lw-font-mono, monospace)',
          fontSize: '0.65rem',
          color: 'var(--lw-gray, #444)',
        }}>
          {label}
        </span>
      )}
    </div>
  );
}

function darken(hex: string, amount: number): string {
  const num = parseInt(hex.replace('#', ''), 16);
  const r = Math.max(0, ((num >> 16) & 0xff) * (1 - amount));
  const g = Math.max(0, ((num >> 8) & 0xff) * (1 - amount));
  const b = Math.max(0, (num & 0xff) * (1 - amount));
  return `rgb(${Math.round(r)}, ${Math.round(g)}, ${Math.round(b)})`;
}
