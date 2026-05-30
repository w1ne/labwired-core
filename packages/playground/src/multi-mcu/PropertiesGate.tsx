// Wraps the bottom Serial/Registers/Trace/Memory/Source/YAML
// drawer with:
//   - a chip tab row at the very top (VS Code-style file tabs)
//     that lets the user pick which MCU's properties to inspect;
//     each tab also has an X to remove the chip,
//   - a close button on desktop (mobile dismiss is via back arrow
//     in MobileMultiChipView's drawer header).
//
// The chip tabs replace the floating McuStrip — one source of
// truth for chip switching: the drawer that owns the properties
// also owns the switcher. Adding new MCUs still goes through the
// ⌘K command palette.
import { useEffect, useState, type ReactNode } from 'react';
import { useChips } from './ChipsProvider';
import { McuThumb } from './McuThumb';
import './properties-gate.css';

export function PropertiesGate({ children }: { children: ReactNode }) {
  const { propertiesOpen, setPropertiesOpen, activeChipId, setActiveChipId, sessions, order, removeChip } = useChips();
  const isMobile = useIsMobile();
  if (!propertiesOpen) return null;
  return (
    <>
      {/* Desktop close pill (mobile drawer has its own back). */}
      {!isMobile && (
        <button
          type="button"
          onClick={() => setPropertiesOpen(false)}
          aria-label="Close properties"
          className="lw-properties-close"
        >
          ×
        </button>
      )}
      {/* Chip tab row — VS Code style. One tab per MCU. Click
          switches the active chip whose properties are shown in
          the drawer below. Each tab carries its PCB thumbnail so
          the user can match the canvas to the drawer at a glance. */}
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
      {children}
    </>
  );
}

function useIsMobile() {
  const [isMobile, setIsMobile] = useState(() =>
    typeof window === 'undefined'
      ? false
      : window.matchMedia('(max-width: 767px)').matches,
  );
  useEffect(() => {
    if (typeof window === 'undefined') return;
    const mq = window.matchMedia('(max-width: 767px)');
    const handler = (e: MediaQueryListEvent) => setIsMobile(e.matches);
    mq.addEventListener('change', handler);
    return () => mq.removeEventListener('change', handler);
  }, []);
  return isMobile;
}
