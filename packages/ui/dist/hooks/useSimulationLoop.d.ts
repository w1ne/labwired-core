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
/**
 * Hook that drives the simulation loop via requestAnimationFrame.
 * Polls state from the bridge each frame when running.
 */
export declare function useSimulationLoop(options: UseSimulationLoopOptions): UseSimulationLoopResult;
