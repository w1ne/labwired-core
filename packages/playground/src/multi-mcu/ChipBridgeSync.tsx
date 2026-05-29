// Bridges App.tsx's single-MCU state into the ChipsProvider registry
// + handles atomic snapshot/restore on activeChipId change so
// switching MCUs preserves each one's bridge + board + source +
// config without races.
import { useEffect, useRef } from 'react';
import type { SimulatorBridge } from '@labwired/ui';
import { useChips } from './ChipsProvider';
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
  const stateRef = useRef({ bridge, board, source, config });
  stateRef.current = { bridge, board, source, config };

  // Mirror: keep the active chip's session in sync with App state.
  // Skip the first render after activeChipId changes — the switch
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
    // Atomic snapshot of OLD chip + restore of NEW chip in the same
    // effect so mirror writes can't bleed the old state into the
    // new session under the new activeChipId.
    chips.setSession(oldId, {
      bridge: stateRef.current.bridge,
      board: stateRef.current.board,
      source: stateRef.current.source,
      config: stateRef.current.config,
    });
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
