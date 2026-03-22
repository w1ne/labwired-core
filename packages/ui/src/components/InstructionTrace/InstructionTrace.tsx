import { CSSProperties, useEffect, useRef } from 'react';

export interface TraceEntry {
  pc: number;
  disassembly: string;
}

export interface InstructionTraceProps {
  /** Trace entries to display (most recent last). */
  entries: TraceEntry[];
  /** Maximum number of entries to keep visible. Default: 50. */
  maxEntries?: number;
  style?: CSSProperties;
}

export function InstructionTrace({ entries, maxEntries = 50, style }: InstructionTraceProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const visible = entries.slice(-maxEntries);

  // Auto-scroll to bottom on new entries
  useEffect(() => {
    if (containerRef.current) {
      containerRef.current.scrollTop = containerRef.current.scrollHeight;
    }
  }, [entries.length]);

  return (
    <div
      ref={containerRef}
      style={{
        background: 'var(--lw-dark-bg, #1e1e28)',
        border: 'var(--lw-border, 2px solid #000)',
        borderRadius: 'var(--lw-radius-sm, 8px)',
        padding: '0.75rem',
        fontFamily: 'var(--lw-font-mono, monospace)',
        fontSize: '0.7rem',
        overflow: 'auto',
        maxHeight: 200,
        ...style,
      }}
    >
      <div style={{
        fontFamily: 'var(--lw-font-heading, sans-serif)',
        fontWeight: 700,
        fontSize: '0.7rem',
        textTransform: 'uppercase',
        letterSpacing: '0.05em',
        color: 'var(--lw-gray-light, #888)',
        marginBottom: '0.5rem',
      }}>
        Instruction Trace
      </div>
      {visible.map((entry, i) => (
        <div key={i} style={{ display: 'flex', gap: '1rem' }}>
          <span style={{ color: '#569cd6', minWidth: 80 }}>
            0x{entry.pc.toString(16).toUpperCase().padStart(8, '0')}
          </span>
          <span style={{ color: 'var(--lw-cyan, #0ff)' }}>
            {entry.disassembly}
          </span>
        </div>
      ))}
    </div>
  );
}
