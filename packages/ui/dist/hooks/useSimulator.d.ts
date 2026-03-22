import { SimulatorBridge, SimulatorConfig, WasmModule } from '../wasm/simulator-bridge';
export interface UseSimulatorOptions {
    /** Pre-loaded WASM module. */
    wasmModule: WasmModule;
    /** Config for config-driven init. If omitted, uses legacy mode with firmware only. */
    config?: SimulatorConfig;
    /** Firmware bytes for legacy mode (ignored if config is provided). */
    firmware?: Uint8Array;
}
export interface UseSimulatorResult {
    bridge: SimulatorBridge | null;
    loading: boolean;
    error: string | null;
}
/**
 * Hook to load the WASM simulator and create a SimulatorBridge.
 * Handles async initialization and cleanup.
 */
export declare function useSimulator(options: UseSimulatorOptions): UseSimulatorResult;
