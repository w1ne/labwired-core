import { CSSProperties, useMemo } from 'react';

export interface RegisterGridProps {
  /** Register name->value map. */
  registers: Map<string, number>;
  /** Highlight the PC register. */
  pc?: number;
  style?: CSSProperties;
}

export function RegisterGrid({ registers, style }: RegisterGridProps) {
  const entries = useMemo(() => Array.from(registers.entries()), [registers]);

  return (
    <div style={{
      background: 'var(--lw-dark-bg, #1e1e28)',
      border: 'var(--lw-border, 2px solid #000)',
      borderRadius: 'var(--lw-radius-sm, 8px)',
      padding: '0.75rem',
      fontFamily: 'var(--lw-font-mono, monospace)',
      fontSize: '0.75rem',
      overflow: 'auto',
      ...style,
    }}>
      <div style={{
        fontFamily: 'var(--lw-font-heading, sans-serif)',
        fontWeight: 700,
        fontSize: '0.7rem',
        textTransform: 'uppercase',
        letterSpacing: '0.05em',
        color: 'var(--lw-gray-light, #888)',
        marginBottom: '0.5rem',
      }}>
        Registers
      </div>
      <div style={{
        display: 'grid',
        gridTemplateColumns: 'repeat(2, 1fr)',
        gap: '2px 1rem',
      }}>
        {entries.map(([name, value]) => (
          <div key={name} style={{ display: 'flex', justifyContent: 'space-between' }}>
            <span style={{ color: 'var(--lw-cyan, #0ff)' }}>{name}</span>
            <span style={{ color: 'var(--lw-dark-text, #d4d4d4)' }}>
              0x{value.toString(16).toUpperCase().padStart(8, '0')}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}
