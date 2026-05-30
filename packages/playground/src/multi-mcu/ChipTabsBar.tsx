// Compact chip switcher that sits inline on the LEFT of the
// drawer's existing Serial/Registers/Trace/Memory/Source/YAML
// tab strip. Each "chip" is a small pill — chipId only — with the
// active one highlighted in magenta. Followed by a vertical
// divider that separates the chip switcher from the dev tabs.
//
// Single-chip sessions render nothing (chip is implied; saves
// horizontal space).
import { useChips } from './ChipsProvider';
import './chip-tabs-bar.css';

export function ChipTabsBar() {
  const { order, activeChipId, setActiveChipId, removeChip, setPropertiesOpen } = useChips();
  return (
    <div className="lw-chip-switch" role="tablist" aria-label="MCU instances">
      {/* Close-drawer button on the LEFT so it stays put even when
          the tab strip scrolls horizontally. */}
      <button
        type="button"
        className="lw-chip-switch-close"
        onClick={() => setPropertiesOpen(false)}
        aria-label="Close properties drawer"
        title="Hide properties"
      >
        ×
      </button>
      {order.length > 1 && order.map((chipId) => {
        const isActive = chipId === activeChipId;
        return (
          <span key={chipId} className="lw-chip-pill" data-active={isActive ? 'true' : 'false'}>
            <button
              type="button"
              className="lw-chip-pill-focus"
              role="tab"
              aria-selected={isActive}
              onClick={() => setActiveChipId(chipId)}
            >
              {chipId}
            </button>
            {chipId !== 'chip-default' && (
              <button
                type="button"
                className="lw-chip-pill-remove"
                aria-label={`Remove ${chipId}`}
                onClick={(e) => {
                  e.stopPropagation();
                  removeChip(chipId);
                }}
              >
                ×
              </button>
            )}
          </span>
        );
      })}
      {order.length > 1 && <span className="lw-chip-switch-divider" aria-hidden />}
    </div>
  );
}
