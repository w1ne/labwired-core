// Visibility gate for the bottom dev drawer. The close button
// itself now lives inside <ChipTabsBar> (passed via DevDrawer's
// `header` slot) so it tracks the drawer's resizable height —
// see comment in chip-tabs-bar.css. Mobile dismiss is via the
// back arrow in MobileMultiChipView's drawer header.
import type { ReactNode } from 'react';
import { useChips } from './ChipsProvider';

export function PropertiesGate({ children }: { children: ReactNode }) {
  const { propertiesOpen } = useChips();
  if (!propertiesOpen) return null;
  return <>{children}</>;
}
