// Compact card rendered inside a non-active ChipShape's body. Click to
// make this chip the focused chip — the registry pauses the current
// chip, swaps state, and gives the clicked chip control of the
// StudioShell.
import { useChips } from './ChipSession';
import type { ChipSession } from './ChipSession';

export function ChipCard({ session }: { session: ChipSession }) {
  const { setActiveChipId } = useChips();
  const hasBridge = session.bridge !== null;
  const hasSource = !!session.source;
  return (
    <button
      type="button"
      onClick={() => setActiveChipId(session.chipId)}
      style={{
        width: '100%',
        height: '100%',
        padding: 14,
        textAlign: 'left',
        background: 'transparent',
        border: 'none',
        color: 'rgba(255, 255, 255, 0.85)',
        fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace',
        cursor: 'pointer',
        display: 'flex',
        flexDirection: 'column',
        gap: 8,
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
            background: hasBridge ? '#33dd66' : hasSource ? '#e8c842' : '#555',
          }}
        />
        {hasBridge ? 'running' : hasSource ? 'source ready' : 'empty'}
      </div>
      <div style={{ fontSize: 10, opacity: 0.4 }}>click to focus →</div>
    </button>
  );
}
