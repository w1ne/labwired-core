// Gates the bottom dev drawer (Serial / Registers / Trace / Memory /
// Source / YAML) so it only renders when the user has explicitly
// opened a chip's properties — by clicking an MCU tile in the
// McuStrip or invoking Properties via the command palette.
//
// Adds a small close button (X) in the top-right corner of the
// drawer area so the user can dismiss it cleanly without needing
// the McuStrip.
import type { ReactNode } from 'react';
import { useChips } from './ChipsProvider';
import './properties-gate.css';

export function PropertiesGate({ children }: { children: ReactNode }) {
  const { propertiesOpen, setPropertiesOpen, activeChipId, sessions } = useChips();
  if (!propertiesOpen) return null;
  const session = sessions[activeChipId];
  return (
    <>
      <div className="lw-properties-close-row">
        <span className="lw-properties-close-label">
          {session ? `${session.chipId} · ${session.board.name}` : 'properties'}
        </span>
        <button
          type="button"
          onClick={() => setPropertiesOpen(false)}
          aria-label="Close properties"
          className="lw-properties-close"
        >
          ×
        </button>
      </div>
      {children}
    </>
  );
}
