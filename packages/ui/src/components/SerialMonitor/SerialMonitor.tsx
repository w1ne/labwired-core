import { CSSProperties, useEffect, useRef, useState, useCallback } from 'react';

export interface SerialMonitorProps {
  /** UART output text. */
  output: string;
  /** Called when the user wants to clear the output. */
  onClear?: () => void;
  /** Called when user sends data (TX). If provided, shows input field. */
  onSend?: (data: string) => void;
  style?: CSSProperties;
}

export function SerialMonitor({ output, onClear, onSend, style }: SerialMonitorProps) {
  const preRef = useRef<HTMLPreElement>(null);
  const [txInput, setTxInput] = useState('');

  const handleSend = useCallback(() => {
    if (!txInput || !onSend) return;
    onSend(txInput);
    setTxInput('');
  }, [txInput, onSend]);

  // Auto-scroll to bottom
  useEffect(() => {
    if (preRef.current) {
      preRef.current.scrollTop = preRef.current.scrollHeight;
    }
  }, [output]);

  return (
    <div style={{
      background: 'var(--lw-dark-bg, #1e1e28)',
      border: 'var(--lw-border, 2px solid #000)',
      borderRadius: 'var(--lw-radius-sm, 8px)',
      display: 'flex',
      flexDirection: 'column',
      overflow: 'hidden',
      ...style,
    }}>
      <div style={{
        display: 'flex',
        justifyContent: 'space-between',
        alignItems: 'center',
        padding: '0.5rem 0.75rem',
        borderBottom: '1px solid var(--lw-dark-border, #333)',
      }}>
        <span style={{
          fontFamily: 'var(--lw-font-heading, sans-serif)',
          fontWeight: 700,
          fontSize: '0.7rem',
          textTransform: 'uppercase',
          letterSpacing: '0.05em',
          color: 'var(--lw-gray-light, #888)',
        }}>
          Serial Monitor
        </span>
        {onClear && (
          <button
            onClick={onClear}
            style={{
              background: 'transparent',
              border: 'none',
              color: 'var(--lw-gray-light, #888)',
              cursor: 'pointer',
              fontFamily: 'var(--lw-font-mono, monospace)',
              fontSize: '0.65rem',
              padding: '2px 6px',
              boxShadow: 'none',
              textTransform: 'none',
            }}
          >
            Clear
          </button>
        )}
      </div>
      <pre
        ref={preRef}
        style={{
          margin: 0,
          padding: '0.75rem',
          fontFamily: 'var(--lw-font-mono, monospace)',
          fontSize: '0.75rem',
          color: 'var(--lw-green, #27c93f)',
          lineHeight: 1.5,
          overflow: 'auto',
          flex: 1,
          minHeight: 80,
          whiteSpace: 'pre-wrap',
          wordBreak: 'break-all',
        }}
      >
        {output || <span style={{ color: 'var(--lw-gray-light, #888)', fontStyle: 'italic' }}>No output yet...</span>}
      </pre>
      {onSend && (
        <div style={{
          display: 'flex',
          gap: '4px',
          padding: '4px 8px',
          borderTop: '1px solid var(--lw-dark-border, #333)',
        }}>
          <input
            type="text"
            value={txInput}
            onChange={(e) => setTxInput(e.target.value)}
            onKeyDown={(e) => { if (e.key === 'Enter') handleSend(); }}
            placeholder="Type to send..."
            style={{
              flex: 1,
              fontFamily: 'var(--lw-font-mono, monospace)',
              fontSize: '0.75rem',
              padding: '4px 8px',
              border: '1px solid rgba(255,255,255,0.15)',
              borderRadius: '3px',
              background: 'rgba(255,255,255,0.05)',
              color: '#fff',
              outline: 'none',
            }}
          />
          <button
            onClick={handleSend}
            style={{
              fontFamily: 'var(--lw-font-mono, monospace)',
              fontSize: '0.65rem',
              padding: '4px 10px',
              border: '1px solid var(--lw-pink, #e83e8c)',
              borderRadius: '3px',
              background: 'var(--lw-pink, #e83e8c)',
              color: '#fff',
              cursor: 'pointer',
              boxShadow: 'none',
              textTransform: 'none',
            }}
          >
            Send
          </button>
        </div>
      )}
    </div>
  );
}
