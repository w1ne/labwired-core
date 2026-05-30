// Drawer header content: clear identity of the chip whose
// properties the drawer is showing, plus a close button.
//
// Replaces the older multi-chip pill switcher — chip switching
// now happens via the McuStrip tile's "Properties" button (one
// chip owns one drawer). The drawer here is unambiguously
// labelled with the chip it represents.
import { useChips } from './ChipsProvider';
import { McuThumb } from './McuThumb';
import './chip-tabs-bar.css';

export function ChipTabsBar() {
  const { sessions, activeChipId, setPropertiesOpen } = useChips();
  const session = sessions[activeChipId];
  if (!session) return null;
  return (
    <div className="lw-chip-header" aria-label="Active chip properties header">
      <button
        type="button"
        className="lw-chip-switch-close"
        onClick={() => setPropertiesOpen(false)}
        aria-label="Close properties drawer"
        title="Hide properties"
      >
        ×
      </button>
      <span className="lw-chip-header-thumb">
        <McuThumb session={session} width={32} height={20} />
      </span>
      <span className="lw-chip-header-label">
        <span className="lw-chip-header-prop">Properties of</span>
        <span className="lw-chip-header-id">{session.chipId}</span>
        <span className="lw-chip-header-sep">·</span>
        <span className="lw-chip-header-board">{session.board.name}</span>
      </span>
      <span className="lw-chip-switch-divider" aria-hidden />
    </div>
  );
}
