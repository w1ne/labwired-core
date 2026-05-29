// Compact card rendered inside a non-active ChipShape's body. Phase 2b
// shows multi-chip on the canvas; per-chip focus switching (which lets
// you edit a different chip's code without losing your work on the
// current one) lands in Phase 3 once App state is lifted into the
// ChipSession registry.
import type { ChipSession } from './ChipSession';

export function ChipCard({ session }: { session: ChipSession }) {
  const hasBridge = session.bridge !== null;
  return (
    <div
      style={{
        width: '100%',
        height: '100%',
        padding: 14,
        color: 'rgba(255, 255, 255, 0.85)',
        fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace',
        display: 'flex',
        flexDirection: 'column',
        gap: 8,
        pointerEvents: 'none',
        userSelect: 'none',
      }}
    >
      <div style={{ fontSize: 13, fontWeight: 600, opacity: 0.95 }}>{session.chipId}</div>
      <div style={{ fontSize: 11, opacity: 0.55 }}>{session.board.name}</div>
      <div style={{ flex: 1 }} />
      <div style={{ fontSize: 10, opacity: 0.55, display: 'flex', alignItems: 'center', gap: 6 }}>
        <span
          style={{
            display: 'inline-block',
            width: 8,
            height: 8,
            borderRadius: '50%',
            background: hasBridge ? '#33dd66' : '#555',
          }}
        />
        {hasBridge ? 'running' : 'idle (firmware editor: Phase 3)'}
      </div>
    </div>
  );
}
