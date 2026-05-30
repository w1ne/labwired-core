// Per-chip simulation registry for "both chips live on one canvas".
//
// Each MCU part on the (single, shared) diagram gets its own SimulatorBridge.
// The SELECTED MCU part is the "foreground": App's existing bridge/running/
// config state mirrors it, so the main useSimulationLoop drives it and the
// inspector reflects it. Every OTHER running chip ticks in the background so
// it keeps advancing and accumulating serial — that's what lets two chips
// talk over the shared virtual-air BLE registry while you watch one.
//
// This mirrors ChipBridgeSync's snapshot/restore, but keyed by part id and
// WITHOUT swapping the diagram (both chips stay on the canvas).
import { useEffect, useRef, useState } from 'react';
import type { MutableRefObject } from 'react';
import type { SimulatorBridge } from '@labwired/ui';
import type { BoardConfig } from '../bundled-configs';

export interface ChipSim {
  bridge: SimulatorBridge | null;
  running: boolean;
  config: unknown;
  board: BoardConfig | null;
  /** Accumulated serial output while this chip runs off-screen (background). */
  uart: string;
}

const BACKGROUND_CYCLES_PER_FRAME = 200_000;
const FRAME_INTERVAL_MS = 16;

function emptySim(): ChipSim {
  return { bridge: null, running: false, config: null, board: null, uart: '' };
}

interface Options {
  /** Part id of the selected MCU — the foreground chip. */
  foregroundPartId: string;
  /** All MCU part ids currently on the canvas — drives removal cleanup. */
  mcuPartIds: string[];
  /** Current foreground state (App mirror). */
  bridge: SimulatorBridge | null;
  running: boolean;
  config: unknown;
  board: BoardConfig | null;
  /**
   * The foreground chip's live serial (the main loop owns its UART drain).
   * Committed into the chip's buffer when it drops to the background so its
   * history continues seamlessly.
   */
  foregroundUart: string;
  /** Foreground setters — used to restore a chip when it becomes foreground. */
  setBridge: (b: SimulatorBridge | null) => void;
  setRunning: (r: boolean) => void;
  setConfig: (c: any) => void;
  /** Resets the main loop's single-bridge UART buffer on a foreground switch. */
  clearUart: () => void;
}

export function usePerChipSims(opts: Options): {
  sims: MutableRefObject<Map<string, ChipSim>>;
  /** Bumps as background chips advance, so consumers re-render to show serial. */
  version: number;
} {
  const { foregroundPartId, bridge, running, config, board } = opts;
  const sims = useRef<Map<string, ChipSim>>(new Map());
  const prevId = useRef<string>(foregroundPartId);
  const [version, setVersion] = useState(0);

  // Latest setters/values without retriggering effects on every identity change.
  const optsRef = useRef(opts);
  optsRef.current = opts;

  // Mirror the foreground chip into the registry while NOT mid-switch, so a
  // later snapshot/background-tick sees its live bridge/running/config. UART is
  // NOT mirrored here — the foreground's live serial stays in the main loop and
  // is committed on switch (below).
  useEffect(() => {
    if (prevId.current !== foregroundPartId) return;
    const prev = sims.current.get(foregroundPartId) ?? emptySim();
    sims.current.set(foregroundPartId, { ...prev, bridge, running, config, board });
  }, [foregroundPartId, bridge, running, config, board]);

  // On a foreground switch: commit the old chip's live serial into its buffer
  // (so background ticking continues it), snapshot its sim, restore the new
  // chip. The diagram is untouched — both chips remain on the canvas.
  useEffect(() => {
    if (prevId.current === foregroundPartId) return;
    const old = prevId.current;
    const oldSim = sims.current.get(old) ?? emptySim();
    // Drain the outgoing bridge's pending bytes here so the main loop can't
    // leak them into the incoming chip's freshly-cleared buffer.
    let tail = '';
    if (bridge) {
      try {
        tail = new TextDecoder().decode(bridge.drainUartOutput());
      } catch {
        /* bridge may be mid-teardown */
      }
    }
    sims.current.set(old, {
      ...oldSim,
      bridge,
      running,
      config,
      board,
      uart: oldSim.uart + optsRef.current.foregroundUart + tail,
    });
    prevId.current = foregroundPartId;

    const next = sims.current.get(foregroundPartId) ?? emptySim();
    const o = optsRef.current;
    o.setBridge(next.bridge);
    o.setRunning(next.running);
    o.setConfig(next.config);
    o.clearUart();
  }, [foregroundPartId, bridge, running, config, board]);

  // Removal cleanup: when an MCU part is deleted from the canvas, dispose its
  // bridge and drop its buffers so nothing is orphaned. The foreground is left
  // alone (selection re-derivation moves focus off a deleted chip first, then
  // the next reconcile prunes it).
  const mcuKey = opts.mcuPartIds.join('|');
  useEffect(() => {
    const live = new Set(optsRef.current.mcuPartIds);
    for (const [partId, sim] of sims.current) {
      if (live.has(partId) || partId === prevId.current) continue;
      try {
        sim.bridge?.dispose?.();
      } catch {
        /* bridge may already be torn down */
      }
      sims.current.delete(partId);
    }
  }, [mcuKey]);

  // Background ticker: advance every running chip that ISN'T the foreground
  // (the main loop drives the foreground) and drain its UART into its buffer,
  // so a backgrounded chip keeps running and its serial keeps streaming.
  useEffect(() => {
    const decoder = new TextDecoder();
    let frame = 0;
    const id = window.setInterval(() => {
      let advanced = false;
      for (const [partId, sim] of sims.current) {
        if (partId === prevId.current) continue;
        if (!sim.running || !sim.bridge) continue;
        try {
          sim.bridge.stepBatch(BACKGROUND_CYCLES_PER_FRAME);
          const bytes = sim.bridge.drainUartOutput();
          if (bytes.length) sim.uart += decoder.decode(bytes);
          advanced = true;
        } catch {
          /* swallow — one stuck bridge mustn't take down the others */
        }
      }
      // Throttled re-render (~every 10 frames ≈ 160ms) so background serial
      // shows even when the foreground sim is idle (no loop re-render).
      if (advanced && ++frame % 10 === 0) setVersion((v) => v + 1);
    }, FRAME_INTERVAL_MS);
    return () => window.clearInterval(id);
  }, []);

  return { sims, version };
}
