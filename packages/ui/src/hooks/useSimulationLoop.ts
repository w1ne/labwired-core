import { useState, useEffect, useRef, useCallback } from 'react';
import { SimulatorBridge, BoardIoState } from '../wasm/simulator-bridge';
import type { DisplayBuffer } from '../editor/types';

/** A simulated display device polled by the loop. `partId` must match the
 *  diagram part id AND the `board_io.id` in the system YAML. */
function bytesEqual(a: Uint8Array, b: Uint8Array): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
  return true;
}

export interface DisplayBinding {
  partId: string;
  kind: 'ssd1680_tricolor_290' | 'uc8151d_tricolor_290' | 'pcd8544';
  /**
   * GxEPD2 (the de-facto Arduino-ESP32 SSD1680 library)
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
   * on lab demos, scales up automatically on heavier Arduino-ESP32 firmware.
   */
  cyclesPerFrame?: number;
  /** Display devices to poll per frame (generation-gated). */
  displays?: DisplayBinding[];
}

/** Auto-tune bounds. */
const CYCLES_MIN = 1_000;
// Bumped 4M → 16M after Phase 1.1+1.2 interpreter speedups (#120, #123)
// made the per-cycle cost lower. At 4M the auto-tune was clamping before
// using the new headroom; 16M lets the loop push frames closer to the
// 14ms budget on faster firmware. Browser RAF caps effective batches at
// ~16M anyway (anything past stays on the GC path).
const CYCLES_MAX = 16_000_000;
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
          } else if (d.kind === 'uc8151d_tricolor_290') {
            const gen = b.getUc8151dRefreshGeneration(d.partId);
            if (gen === null) continue;
            const last = displayGenRef.current[d.partId];
            if (last !== undefined && last === gen) continue;
            const data = b.getUc8151dFramebuffer(d.partId);
            if (data === null) continue;
            let bytes = data;
            if (d.invertRedPlane && data.length >= 9472) {
              bytes = new Uint8Array(data);
              for (let i = 4736; i < 9472; i++) bytes[i] = data[i] ^ 0xff;
            }
            displayGenRef.current[d.partId] = gen;
            displayBufRef.current[d.partId] = {
              kind: 'uc8151d_tricolor_290',
              generation: gen,
              data: bytes,
            };
            displaysChanged = true;
          } else if (d.kind === 'pcd8544') {
            // No refresh-generation accessor for the PCD8544 — fetch the small
            // (504-byte) framebuffer every poll and synthesise a generation
            // that bumps only when the pixels actually change, so the component
            // re-encodes its <image> at most once per real frame.
            const data = b.getPcd8544Framebuffer(d.partId);
            if (data === null) continue;
            const prev = displayBufRef.current[d.partId];
            if (prev && prev.kind === 'pcd8544' && bytesEqual(prev.data, data)) continue;
            const gen = (displayGenRef.current[d.partId] ?? 0) + 1;
            displayGenRef.current[d.partId] = gen;
            displayBufRef.current[d.partId] = {
              kind: 'pcd8544',
              generation: gen,
              data,
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
      } catch (e) {
        // A faulting step halts the loop. Surface it — swallowing this
        // silently makes a stuck simulation look like an unexplained freeze.
        console.error('[LabWired] simulation step threw; halting run loop:', e);
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
