// Drawer header — chip identity on the LEFT of the dev tab strip.
// Just the chip's name. The close button lives in the RIGHT slot
// (DevDrawer's headerRight prop) so neither end drifts when the
// drawer is resized.
import { useChips } from './ChipsProvider';
import './chip-tabs-bar.css';

export function ChipTabsBar() {
  const { sessions, activeChipId } = useChips();
  const session = sessions[activeChipId];
  if (!session) return null;
  return (
    <div className="lw-chip-header" aria-label="Active chip properties header">
      <span className="lw-chip-header-id">{session.name}</span>
      <span className="lw-chip-switch-divider" aria-hidden />
    </div>
  );
}

export function DrawerCloseButton() {
  const { setPropertiesOpen } = useChips();
  return (
    <button
      type="button"
      className="lw-chip-switch-close"
      onClick={() => setPropertiesOpen(false)}
      aria-label="Close properties drawer"
      title="Hide properties"
    >
      ×
    </button>
  );
}
