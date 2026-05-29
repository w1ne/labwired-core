// Bridges App.tsx's single-chip state into the ChipsProvider registry.
// One-way: when the active chip's bridge / board changes in App, mirror
// it into chipSessions[activeChipId]. The compact ChipCard reads from
// this mirror so it shows live status.
//
// Phase 2b is intentionally one-way. Phase 3 inverts ownership so App
// reads from chipSessions, which is what enables per-chip focus
// switching without losing per-chip state.
import { useEffect } from 'react';
import type { SimulatorBridge } from '@labwired/ui';
import { useChips } from './ChipSession';
import type { BoardConfig } from '../bundled-configs';

export function ChipBridgeSync({
  bridge,
  board,
}: {
  bridge: SimulatorBridge | null;
  board: BoardConfig;
}) {
  const chips = useChips();
  useEffect(() => {
    chips.setBridge(chips.activeChipId, bridge);
  }, [bridge, chips]);
  useEffect(() => {
    chips.setBoard(chips.activeChipId, board);
  }, [board, chips]);
  return null;
}
