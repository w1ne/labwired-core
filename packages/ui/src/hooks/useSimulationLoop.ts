import { useState, useEffect, useRef, useCallback } from 'react';
import { SimulatorBridge, BoardIoState } from '../wasm/simulator-bridge';
import type { DisplayBuffer } from '../editor/types';

/** A simulated display device polled by the loop. `partId` must match the
 *  diagram part id AND the `board_io.id` in the system YAML. */
export interface DisplayBinding {
  partId: string;
  kind: 'ssd1680_tricolor_290';
  /**
   * GxEPD2 (the de-facto Arduino-ESP32 SSD1680 library, used by AgentDeck)
   * inverts the source bitmap before SPI write — its "no red" source byte
   * (0xFF) lands as 0x00 in the sim's red plane. The hand-rolled Rust
   * firmware in `examples/epaper-tricolor-lab/` skips that inversion and
   * writes the un-inverted form directly. The React SSD1680 component is
   * tuned to the un-inverted convention (bit 0 == red), so GxEPD2 boards
   * need this flag to XOR the red plane back to the renderer's expected
   * polarity. Defaults to false.
   */
  invertRedPlane?: boolean;
}

export interface SimulationState {
  /** Current program counter. */
  pc: number;
  /** Total cycles executed. */
  cycles: number;
  /** Board IO states (LED on/off, button pressed, etc.). */
  boardIoStates: BoardIoState[];
  /** Live display framebuffers, keyed by part id. */
  displayBuffers: Record<string, DisplayBuffer>;
  /** Accumulated UART output as a string. */
  uartOutput: string;
  /** Current disassembly at PC. */
  disassembly: string;
}

export interface UseSimulationLoopOptions {
  /** The simulator bridge instance. */
  bridge: SimulatorBridge | null;
  /** Whether the simulation is running. */
  running: boolean;
  /**
   * Initial cycles-per-frame batch size. The loop auto-tunes from this seed
   * toward a 14 ms wall-clock budget per frame (target 60 fps with 2 ms of
   * headroom for `pollState` + paint). Min clamp 1 000 cycles, max 4 000 000;
   * the auto-tune doubles when frame time is under 8 ms and halves when it
   * goes over 14 ms. Default seed: 50 000 — fast-enough first-frame response
   * on lab demos, scales up automatically on heavier firmware like AgentDeck.
   */
  cyclesPerFrame?: number;
  /** Display devices to poll per frame (generation-gated). */
  displays?: DisplayBinding[];
}

/** Auto-tune bounds. */
const CYCLES_MIN = 1_000;
const CYCLES_MAX = 4_000_000;
/** Frame-budget targets (ms). Under LOW: scale up. Over HIGH: scale down. */
const FRAME_BUDGET_LOW = 8;
const FRAME_BUDGET_HIGH = 14;

export interface UseSimulationLoopResult {
  state: SimulationState;
  /** Step a single instruction (for single-step mode). */
  stepOnce: () => void;
  /** Reset UART output buffer. */
  clearUart: () => void;
}

const INITIAL_STATE: SimulationState = {
  pc: 0,
  cycles: 0,
  boardIoStates: [],
  displayBuffers: {},
  uartOutput: '',
  disassembly: '',
};

/**
 * Hook that drives the simulation loop via requestAnimationFrame.
 * Polls state from the bridge each frame when running.
 */
export function useSimulationLoop(
  options: UseSimulationLoopOptions,
): UseSimulationLoopResult {
  const { bridge, running, cyclesPerFrame = 50_000, displays } = options;
  const [state, setState] = useState<SimulationState>(INITIAL_STATE);
  const uartBufferRef = useRef('');
  const rafRef = useRef<number>(0);
  // Per-partId generation tracking — only re-fetch the (larger) framebuffer
  // when the panel actually refreshed, so polling 60fps doesn't thrash wasm.
  const displayGenRef = useRef<Record<string, number>>({});
  const displayBufRef = useRef<Record<string, DisplayBuffer>>({});
  // Auto-tune batch size. Initial value is the prop; the loop adjusts based
  // on measured stepBatch wall time. Ref so the closure picks up the latest
  // value without re-running the effect.
  const batchRef = useRef<number>(cyclesPerFrame);

  const pollState = useCallback(
    (b: SimulatorBridge) => {
      const uartBytes = b.drainUartOutput();
      if (uartBytes.length > 0) {
        const decoder = new TextDecoder();
        uartBufferRef.current += decoder.decode(uartBytes);
      }

      // Poll display framebuffers, generation-gated.
      let displaysChanged = false;
      if (displays && displays.length > 0) {
        for (const d of displays) {
          if (d.kind === 'ssd1680_tricolor_290') {
            const gen = b.getSsd1680RefreshGeneration(d.partId);
            if (gen === null) continue;
            const last = displayGenRef.current[d.partId];
            if (last !== undefined && last === gen) continue;
            const data = b.getSsd1680Framebuffer(d.partId);
            if (data === null) continue;
            // Flip the red-plane polarity back to the renderer's convention
            // when the firmware uses GxEPD2's inverted convention. Black
            // plane is unaffected (both conventions agree there). See the
            // `invertRedPlane` doc above for the why.
            let bytes = data;
            if (d.invertRedPlane && data.length >= 9472) {
              bytes = new Uint8Array(data);
              for (let i = 4736; i < 9472; i++) bytes[i] = data[i] ^ 0xff;
            }
            displayGenRef.current[d.partId] = gen;
            displayBufRef.current[d.partId] = {
              kind: 'ssd1680_tricolor_290',
              generation: gen,
              data: bytes,
            };
            displaysChanged = true;
          }
        }
      }

      setState({
        pc: b.getPC(),
        cycles: b.totalCycles,
        boardIoStates: b.getBoardIoStates(),
        displayBuffers: displaysChanged
          ? { ...displayBufRef.current }
          : displayBufRef.current,
        uartOutput: uartBufferRef.current,
        disassembly: b.getDisassembly(),
      });
    },
    [displays],
  );

  // Reset the auto-tune seed when the prop changes (e.g., new board).
  useEffect(() => {
    batchRef.current = cyclesPerFrame;
  }, [cyclesPerFrame]);

  useEffect(() => {
    if (!bridge || !running) return;

    function tick() {
      if (!bridge) return;
      const t0 = performance.now();
      try {
        bridge.stepBatch(batchRef.current);
      } catch {
        // Simulation error - stop the loop
        return;
      }
      const elapsed = performance.now() - t0;
      // Adaptive batch sizing — keep stepBatch under the frame budget. The
      // 2× / 0.5× swing is intentionally aggressive: the inner cost is
      // dominated by Xtensa decode/execute, which is roughly linear in
      // cycles, so a too-small batch wastes the RAF call overhead and a
      // too-big batch starves the UI. Bound the next-frame batch to the
      // hard limits so a single slow tick can't permanently tank or peg us.
      if (elapsed < FRAME_BUDGET_LOW) {
        batchRef.current = Math.min(CYCLES_MAX, batchRef.current * 2);
      } else if (elapsed > FRAME_BUDGET_HIGH) {
        batchRef.current = Math.max(CYCLES_MIN, Math.floor(batchRef.current / 2));
      }
      pollState(bridge);
      rafRef.current = requestAnimationFrame(tick);
    }

    rafRef.current = requestAnimationFrame(tick);

    return () => {
      if (rafRef.current) {
        cancelAnimationFrame(rafRef.current);
      }
    };
  }, [bridge, running, pollState]);

  // Poll initial state when bridge first becomes available
  useEffect(() => {
    if (bridge && !running) {
      pollState(bridge);
    }
  }, [bridge, running, pollState]);

  const stepOnce = useCallback(() => {
    if (!bridge) return;
    bridge.stepSingle();
    pollState(bridge);
  }, [bridge, pollState]);

  const clearUart = useCallback(() => {
    uartBufferRef.current = '';
    setState((prev) => ({ ...prev, uartOutput: '' }));
  }, []);

  return { state, stepOnce, clearUart };
}
