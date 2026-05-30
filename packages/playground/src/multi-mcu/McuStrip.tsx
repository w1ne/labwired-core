// MCU strip — one tile per chip in the session. Each tile shows
// the chip's PCB thumbnail + name + board, with two actions:
//   - the tile body switches the active chip (selects it on the
//     canvas; canvas board image swaps to that chip's),
//   - "Properties" opens the bottom drawer FOR that chip — the
//     drawer is owned by the chip, not a global panel.
//
// Tiles render only when there are 2+ MCUs; single-chip sessions
// keep the chrome quiet (chip is implied).
//
// Adding a new MCU goes via ⌘K → "Add component: <board>".
import { useChips } from './ChipsProvider';
import type { ChipSession } from './ChipsProvider';
import { McuThumb } from './McuThumb';
import './mcu-strip.css';

export function McuStrip() {
  const { order, sessions, activeChipId, setActiveChipId, setPropertiesOpen, removeChip } = useChips();
  if (order.length <= 1) return null;
  return (
    <div className="lw-mcu-strip" role="toolbar" aria-label="MCU instances">
      {order.map((chipId) => {
        const session = sessions[chipId];
        if (!session) return null;
        const isActive = chipId === activeChipId;
        return (
          <McuTile
            key={chipId}
            session={session}
            isActive={isActive}
            onFocus={() => setActiveChipId(chipId)}
            onOpenProperties={() => {
              setActiveChipId(chipId);
              setPropertiesOpen(true);
            }}
            onRemove={chipId === 'chip-default' ? undefined : () => removeChip(chipId)}
          />
        );
      })}
    </div>
  );
}

function McuTile({
  session,
  isActive,
  onFocus,
  onOpenProperties,
  onRemove,
}: {
  session: ChipSession;
  isActive: boolean;
  onFocus: () => void;
  onOpenProperties: () => void;
  onRemove?: () => void;
}) {
  return (
    <div className="lw-mcu-tile" data-active={isActive ? 'true' : 'false'}>
      <button type="button" className="lw-mcu-tile-focus" onClick={onFocus}>
        <span className="lw-mcu-tile-thumb">
          <McuThumb session={session} width={56} height={36} />
        </span>
        <span className="lw-mcu-tile-meta">
          <span className="lw-mcu-tile-id">{session.chipId}</span>
          <span className="lw-mcu-tile-board">{session.board.name}</span>
        </span>
      </button>
      <button
        type="button"
        className="lw-mcu-tile-props"
        onClick={onOpenProperties}
        title={`Open ${session.chipId} properties`}
      >
        Properties
      </button>
      {onRemove && (
        <button
          type="button"
          className="lw-mcu-tile-remove"
          aria-label={`Remove ${session.chipId}`}
          title={`Remove ${session.chipId}`}
          onClick={onRemove}
        >
          ×
        </button>
      )}
    </div>
  );
}
