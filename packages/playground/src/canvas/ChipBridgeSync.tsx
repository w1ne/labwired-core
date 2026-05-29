// Bridges App.tsx's single-chip state into the ChipsProvider registry
// and back.
//
// Direction 1 (mirror): when App state changes while activeChipId is
// stable, mirror into chipSessions[activeChipId].
// Direction 2 (switch): when activeChipId changes (because the user
// clicked an inactive ChipCard), atomically snapshot the OLD active
// chip's App state into its session BEFORE loading the new chip's
// state into App. Doing the snapshot in the same effect as the
// restore avoids the race where the mirror effect would otherwise
// write the OLD chip's state into the NEW session.
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

  // Track the latest App-state values in a ref so the switch effect
  // can snapshot them without re-running every time bridge/board/...
  // change. The mirror effects below keep the ref fresh.
  const stateRef = useRef({ bridge, board, source, config });
  stateRef.current = { bridge, board, source, config };

  // Mirror — keep the current chip's session in sync with App state.
  // Skip the first render after activeChipId changes; the switch
  // effect owns that transition.
  const prevActiveChipId = useRef<string>(activeChipId);
  useEffect(() => {
    if (prevActiveChipId.current !== activeChipId) return;
    chips.setSession(activeChipId, { bridge });
  }, [bridge, activeChipId, chips]);
  useEffect(() => {
    if (prevActiveChipId.current !== activeChipId) return;
    chips.setSession(activeChipId, { board });
  }, [board, activeChipId, chips]);
  useEffect(() => {
    if (prevActiveChipId.current !== activeChipId) return;
    chips.setSession(activeChipId, { source });
  }, [source, activeChipId, chips]);
  useEffect(() => {
    if (prevActiveChipId.current !== activeChipId) return;
    chips.setSession(activeChipId, { config });
  }, [config, activeChipId, chips]);

  // Switch — atomic snapshot+restore on activeChipId change. Skipped
  // on initial mount so chip-default's empty session doesn't clobber
  // the freshly bootstrapped App state.
  const firstRunRef = useRef(true);
  useEffect(() => {
    if (firstRunRef.current) {
      firstRunRef.current = false;
      prevActiveChipId.current = activeChipId;
      return;
    }
    const oldId = prevActiveChipId.current;
    const newId = activeChipId;
    if (oldId === newId) return;
    // Snapshot the OLD active chip's full App-state into its session
    // before loading the new one — otherwise the mirror effects would
    // write the old state into the new session under the new
    // activeChipId.
    if (oldId) {
      chips.setSession(oldId, {
        bridge: stateRef.current.bridge,
        board: stateRef.current.board,
        source: stateRef.current.source,
        config: stateRef.current.config,
      });
    }
    prevActiveChipId.current = newId;
    const session = chips.sessions[newId];
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
