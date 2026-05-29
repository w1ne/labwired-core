// Ticks every non-active chip's SimulatorBridge per frame so the
// shared virtual-air registry on the Rust side sees all transmitters
// — required for cross-instance BLE.
import { useEffect, useRef } from 'react';
import { useChips } from './ChipsProvider';

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
          /* swallow — one stuck bridge mustn't take down the others */
        }
      }
    }, FRAME_INTERVAL_MS);
    return () => window.clearInterval(id);
  }, [enabled]);
}
