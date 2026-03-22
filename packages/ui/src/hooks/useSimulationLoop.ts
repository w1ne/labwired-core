import { useState, useEffect, useRef, useCallback } from 'react';
import { SimulatorBridge, BoardIoState } from '../wasm/simulator-bridge';

export interface SimulationState {
  /** Current program counter. */
  pc: number;
  /** Total cycles executed. */
  cycles: number;
  /** Board IO states (LED on/off, button pressed, etc.). */
  boardIoStates: BoardIoState[];
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
  const { bridge, running, cyclesPerFrame = 5000 } = options;
  const [state, setState] = useState<SimulationState>(INITIAL_STATE);
  const uartBufferRef = useRef('');
  const rafRef = useRef<number>(0);

  const pollState = useCallback(
    (b: SimulatorBridge) => {
      const uartBytes = b.drainUartOutput();
      if (uartBytes.length > 0) {
        const decoder = new TextDecoder();
        uartBufferRef.current += decoder.decode(uartBytes);
      }

      setState({
        pc: b.getPC(),
        cycles: b.totalCycles,
        boardIoStates: b.getBoardIoStates(),
        uartOutput: uartBufferRef.current,
        disassembly: b.getDisassembly(),
      });
    },
    [],
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
