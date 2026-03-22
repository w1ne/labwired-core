import { CSSProperties, useMemo } from 'react';

export interface MemoryInspectorProps {
  /** Memory data bytes. */
  data: Uint8Array;
  /** Base address of the first byte. */
  baseAddress: number;
  /** Bytes per row. Default: 16. */
  bytesPerRow?: number;
  style?: CSSProperties;
}

export function MemoryInspector({
  data,
  baseAddress,
  bytesPerRow = 16,
  style,
}: MemoryInspectorProps) {
  const rows = useMemo(() => {
    const result: { addr: number; hex: string[]; ascii: string }[] = [];
    for (let i = 0; i < data.length; i += bytesPerRow) {
      const slice = data.slice(i, i + bytesPerRow);
      const hex = Array.from(slice).map((b) => b.toString(16).toUpperCase().padStart(2, '0'));
      const ascii = Array.from(slice)
        .map((b) => (b >= 0x20 && b < 0x7f ? String.fromCharCode(b) : '.'))
        .join('');
      result.push({ addr: baseAddress + i, hex, ascii });
    }
    return result;
  }, [data, baseAddress, bytesPerRow]);

  return (
    <div style={{
      background: 'var(--lw-dark-bg, #1e1e28)',
      border: 'var(--lw-border, 2px solid #000)',
      borderRadius: 'var(--lw-radius-sm, 8px)',
      padding: '0.75rem',
      fontFamily: 'var(--lw-font-mono, monospace)',
      fontSize: '0.7rem',
      overflow: 'auto',
      maxHeight: 200,
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
        Memory
      </div>
      <table style={{ borderCollapse: 'collapse', width: '100%' }}>
        <tbody>
          {rows.map((row) => (
            <tr key={row.addr}>
              <td style={{ color: '#569cd6', paddingRight: '1rem', whiteSpace: 'nowrap' }}>
                0x{row.addr.toString(16).toUpperCase().padStart(8, '0')}
              </td>
              <td style={{ color: 'var(--lw-dark-text, #d4d4d4)', whiteSpace: 'nowrap' }}>
                {row.hex.join(' ')}
              </td>
              <td style={{ color: 'var(--lw-green, #27c93f)', paddingLeft: '1rem', whiteSpace: 'nowrap' }}>
                {row.ascii}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
