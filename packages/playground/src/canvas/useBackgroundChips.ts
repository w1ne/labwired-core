// Ticks the SimulatorBridge of every non-active chip session every
// frame, with a small per-chip budget. Required for Phase 4's BLE air
// to work — the cross-instance virtual-air registry (Rust `static
// OnceLock<Mutex<VirtualAir>>`) only routes packets if both TX and
// RX bridges are actually executing.
//
// Active chip is excluded — it's already driven by App.tsx's main
// useSimulationLoop. Non-active chips with `bridge === null` (e.g.,
// freshly added chips with no firmware loaded yet) are skipped.
import { useEffect, useRef } from 'react';
import { useChips } from './ChipSession';

const BACKGROUND_CYCLES_PER_FRAME = 50_000;
const FRAME_INTERVAL_MS = 16;

export function useBackgroundChips(enabled: boolean) {
  const chips = useChips();
  const chipsRef = useRef(chips);
  chipsRef.current = chips;

  useEffect(() => {
    if (!enabled) return;
    const id = window.setInterval(() => {
      const { sessions, activeChipId, order } = chipsRef.current;
      for (const chipId of order) {
        if (chipId === activeChipId) continue;
        const session = sessions[chipId];
        if (!session?.bridge) continue;
        try {
          session.bridge.stepBatch(BACKGROUND_CYCLES_PER_FRAME);
        } catch {
          // Swallow per-chip errors so one stuck background chip
          // doesn't take down the active sim.
        }
      }
    }, FRAME_INTERVAL_MS);
    return () => window.clearInterval(id);
  }, [enabled]);
}
