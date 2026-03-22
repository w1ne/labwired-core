import { useState, useEffect, useRef } from 'react';
import {
  SimulatorBridge,
  SimulatorConfig,
  WasmModule,
} from '../wasm/simulator-bridge';

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
export function useSimulator(options: UseSimulatorOptions): UseSimulatorResult {
  const [bridge, setBridge] = useState<SimulatorBridge | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const bridgeRef = useRef<SimulatorBridge | null>(null);

  useEffect(() => {
    let cancelled = false;

    async function init() {
      setLoading(true);
      setError(null);

      try {
        let b: SimulatorBridge;
        if (options.config) {
          b = await SimulatorBridge.fromConfig(options.wasmModule, options.config);
        } else if (options.firmware) {
          b = await SimulatorBridge.fromFirmware(options.wasmModule, options.firmware);
        } else {
          throw new Error('Either config or firmware must be provided');
        }

        if (!cancelled) {
          bridgeRef.current = b;
          setBridge(b);
          setLoading(false);
        } else {
          b.dispose();
        }
      } catch (e) {
        if (!cancelled) {
          setError(e instanceof Error ? e.message : String(e));
          setLoading(false);
        }
      }
    }

    init();

    return () => {
      cancelled = true;
      if (bridgeRef.current) {
        bridgeRef.current.dispose();
        bridgeRef.current = null;
      }
    };
  }, [options.wasmModule, options.config, options.firmware]);

  return { bridge, loading, error };
}
