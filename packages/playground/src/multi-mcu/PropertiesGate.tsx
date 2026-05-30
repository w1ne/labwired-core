// Wraps the bottom dev drawer with:
//   - visibility gate (chips.propertiesOpen),
//   - desktop close button (mobile uses MobileMultiChipView's back).
//
// The chip-switcher tabs are NOT here — they live inside the
// DevDrawer itself via its `header` slot (see ChipTabsBar), so
// they stay glued to the drawer's top edge no matter how the
// user resizes its height.
import { useEffect, useState, type ReactNode } from 'react';
import { useChips } from './ChipsProvider';
import './properties-gate.css';

export function PropertiesGate({ children }: { children: ReactNode }) {
  const { propertiesOpen, setPropertiesOpen } = useChips();
  const isMobile = useIsMobile();
  if (!propertiesOpen) return null;
  return (
    <>
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
