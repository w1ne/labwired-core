// Compact card rendered inside a non-active ChipShape's body. Click
// the card to focus the chip; the X in the top-right corner removes
// it from the registry (and frees its SimulatorBridge).
import { useChips } from './ChipSession';
import type { ChipSession } from './ChipSession';

export function ChipCard({ session }: { session: ChipSession }) {
  const { setActiveChipId, removeChip } = useChips();
  const hasBridge = session.bridge !== null;
  const hasSource = !!session.source;
  return (
    <div className="lw-chip-card-root">
      <button
        type="button"
        onClick={() => setActiveChipId(session.chipId)}
        className="lw-chip-card-focus"
      >
        <div className="lw-chip-card-title">{session.chipId}</div>
        <div className="lw-chip-card-board">{session.board.name}</div>
        <div className="lw-chip-card-spacer" />
        <div className="lw-chip-card-status">
          <span
            className="lw-chip-card-led"
            style={{
              background: hasBridge ? '#33dd66' : hasSource ? '#e8c842' : '#555',
            }}
          />
          {hasBridge ? 'running' : hasSource ? 'source ready' : 'empty'}
        </div>
        <div className="lw-chip-card-hint">click to focus →</div>
      </button>
      <button
        type="button"
        onClick={(e) => {
          e.stopPropagation();
          removeChip(session.chipId);
        }}
        className="lw-chip-card-remove"
        aria-label={`Remove ${session.chipId}`}
        title={`Remove ${session.chipId}`}
      >
        ×
      </button>
    </div>
  );
}
