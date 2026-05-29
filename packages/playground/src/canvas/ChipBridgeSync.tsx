// Bridges App.tsx's single-chip state into the ChipsProvider registry
// and back.
//
// Direction 1 (mirror): when the active chip's bridge / board / source
// / config changes in App, mirror it into chipSessions[activeChipId].
// Direction 2 (restore): when activeChipId changes (because the user
// clicked an inactive ChipCard), reload the new chip's state into App.
//
// The mirror writes only on real value changes (setSession is shallow-
// diffed), so the post-restore re-renders don't bounce back through it.
import { useEffect, useRef } from 'react';
import type { SimulatorBridge } from '@labwired/ui';
import { useChips } from './ChipSession';
import type { BoardConfig } from '../bundled-configs';

export interface ChipBridgeSyncProps {
  bridge: SimulatorBridge | null;
  board: BoardConfig;
  source: string;
  config: unknown;
  onRestore: (state: {
    bridge: SimulatorBridge | null;
    board: BoardConfig;
    source: string | null;
    config: unknown;
  }) => void;
}

export function ChipBridgeSync({ bridge, board, source, config, onRestore }: ChipBridgeSyncProps) {
  const chips = useChips();
  const activeChipId = chips.activeChipId;

  // Mirror: keep the active chip's session in sync with App state.
  useEffect(() => {
    chips.setSession(activeChipId, { bridge });
  }, [bridge, activeChipId, chips]);
  useEffect(() => {
    chips.setSession(activeChipId, { board });
  }, [board, activeChipId, chips]);
  useEffect(() => {
    chips.setSession(activeChipId, { source });
  }, [source, activeChipId, chips]);
  useEffect(() => {
    chips.setSession(activeChipId, { config });
  }, [config, activeChipId, chips]);

  // Restore: on activeChipId *change* (not initial mount), push the
  // target session back into App state via onRestore. Skipping the
  // initial mount is important because chip-default's session is
  // initialized empty (null bridge / null source) while App is
  // simultaneously bootstrapping with the board's default code —
  // restoring on mount would clobber that initial source with null.
  const prevActiveChipId = useRef<string | null>(null);
  useEffect(() => {
    if (prevActiveChipId.current === null) {
      prevActiveChipId.current = activeChipId;
      return;
    }
    if (prevActiveChipId.current === activeChipId) return;
    prevActiveChipId.current = activeChipId;
    const session = chips.sessions[activeChipId];
    if (!session) return;
    onRestore({
      bridge: session.bridge,
      board: session.board,
      source: session.source,
      config: session.config,
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeChipId]);

  return null;
}
