// Horizontal tile strip showing every MCU running in the session.
// Lives at the top of the studio area, below the TopChrome. Each
// tile = one independent SimulatorBridge:
//   - thumbnail (PCB + chip silhouette + status LED)
//   - chipId + board name
//   - click to focus that MCU (snapshots current, restores target)
//   - hover X to remove (chip-default protected)
// "+" tile at the end creates a new MCU.
//
// The shared virtual-air registry on the Rust side means N MCUs
// can co-exist and exchange BLE packets without any TS-side glue.
import { useChips } from './ChipsProvider';
import type { ChipSession } from './ChipsProvider';
import { McuThumb } from './McuThumb';
import './mcu-strip.css';

export function McuStrip() {
  const { order, sessions, activeChipId, setActiveChipId, addChip, removeChip } = useChips();
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
            onRemove={chipId === 'chip-default' ? undefined : () => removeChip(chipId)}
          />
        );
      })}
      <button
        type="button"
        className="lw-mcu-add"
        onClick={() => addChip()}
        aria-label="Add MCU"
        title="Drop another MCU into the session — runs an independent binary, shares BLE air"
      >
        +
      </button>
    </div>
  );
}

function McuTile({
  session,
  isActive,
  onFocus,
  onRemove,
}: {
  session: ChipSession;
  isActive: boolean;
  onFocus: () => void;
  onRemove?: () => void;
}) {
  return (
    <div
      className="lw-mcu-tile"
      data-active={isActive ? 'true' : 'false'}
    >
      <button
        type="button"
        className="lw-mcu-tile-focus"
        onClick={onFocus}
        aria-label={`Focus ${session.chipId} (${session.board.name})`}
      >
        <div className="lw-mcu-tile-thumb">
          <McuThumb session={session} width={68} height={42} />
        </div>
        <div className="lw-mcu-tile-meta">
          <div className="lw-mcu-tile-id">{session.chipId}</div>
          <div className="lw-mcu-tile-board">{session.board.name}</div>
        </div>
      </button>
      {onRemove && (
        <button
          type="button"
          className="lw-mcu-tile-remove"
          onClick={onRemove}
          aria-label={`Remove ${session.chipId}`}
          title="Remove this MCU"
        >
          ×
        </button>
      )}
    </div>
  );
}
