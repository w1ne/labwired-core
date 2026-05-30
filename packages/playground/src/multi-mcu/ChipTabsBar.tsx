// VS Code-style chip-switcher bar that lives INSIDE the bottom
// dev drawer (passed as DevDrawer's `header` slot). One tab per
// MCU with PCB thumbnail + chipId + board; clicking switches the
// active chip; X removes it. Sticks to the drawer's top edge no
// matter how the user resizes the drawer height.
import { useChips } from './ChipsProvider';
import { McuThumb } from './McuThumb';
import './chip-tabs-bar.css';

export function ChipTabsBar() {
  const { order, sessions, activeChipId, setActiveChipId, removeChip } = useChips();
  return (
    <div className="lw-chip-tabs" role="tablist" aria-label="MCU instances">
      {order.map((chipId) => {
        const session = sessions[chipId];
        if (!session) return null;
        const isActive = chipId === activeChipId;
        return (
          <div
            key={chipId}
            role="tab"
            aria-selected={isActive}
            data-active={isActive ? 'true' : 'false'}
            className="lw-chip-tab"
          >
            <button
              type="button"
              className="lw-chip-tab-focus"
              onClick={() => setActiveChipId(chipId)}
            >
              <span className="lw-chip-tab-thumb">
                <McuThumb session={session} width={28} height={18} />
              </span>
              <span className="lw-chip-tab-id">{session.chipId}</span>
              <span className="lw-chip-tab-board">{session.board.name}</span>
            </button>
            {chipId !== 'chip-default' && (
              <button
                type="button"
                className="lw-chip-tab-remove"
                aria-label={`Remove ${chipId}`}
                title={`Remove ${chipId}`}
                onClick={(e) => {
                  e.stopPropagation();
                  removeChip(chipId);
                }}
              >
                ×
              </button>
            )}
          </div>
        );
      })}
    </div>
  );
}
