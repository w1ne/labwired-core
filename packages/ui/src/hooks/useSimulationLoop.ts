import { useState, useEffect, useRef, useCallback } from 'react';
import { SimulatorBridge, BoardIoState } from '../wasm/simulator-bridge';
import type { DisplayBuffer } from '../editor/types';

/** A simulated display device polled by the loop. `partId` must match the
 *  diagram part id AND the `board_io.id` in the system YAML. */
export interface DisplayBinding {
  partId: string;
  kind: 'ssd1680_tricolor_290';
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
  /** Cycles to execute per animation frame. Default: 5000. */
  cyclesPerFrame?: number;
  /** Display devices to poll per frame (generation-gated). */
  displays?: DisplayBinding[];
}

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
  const { bridge, running, cyclesPerFrame = 5000, displays } = options;
  const [state, setState] = useState<SimulationState>(INITIAL_STATE);
  const uartBufferRef = useRef('');
  const rafRef = useRef<number>(0);
  // Per-partId generation tracking — only re-fetch the (larger) framebuffer
  // when the panel actually refreshed, so polling 60fps doesn't thrash wasm.
  const displayGenRef = useRef<Record<string, number>>({});
  const displayBufRef = useRef<Record<string, DisplayBuffer>>({});

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
            displayGenRef.current[d.partId] = gen;
            displayBufRef.current[d.partId] = {
              kind: 'ssd1680_tricolor_290',
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

  useEffect(() => {
    if (!bridge || !running) return;

    function tick() {
      if (!bridge) return;
      try {
        bridge.stepBatch(cyclesPerFrame);
      } catch {
        // Simulation error - stop the loop
        return;
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
  }, [bridge, running, cyclesPerFrame, pollState]);

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
