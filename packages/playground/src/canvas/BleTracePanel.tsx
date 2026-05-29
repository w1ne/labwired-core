// Slide-in side panel that shows the last ~200 BLE TX frames seen in
// the shared virtual air. Polls `bridge.air_trace_snapshot()` every
// 250ms while open; any live bridge returns the same snapshot since
// the underlying state is a process-static ring buffer in Rust.
import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from 'react';
import { useChips } from './ChipSession';

interface AirFrameTrace {
  channel: number;
  addr_base: number;
  addr_prefix: number;
  mode: number;
  bytes: number[];
}

interface BleTracePanelCtx {
  open: () => void;
  close: () => void;
  isOpen: boolean;
}

const Ctx = createContext<BleTracePanelCtx | null>(null);

export function BleTracePanelProvider({ children }: { children: ReactNode }) {
  const [isOpen, setIsOpen] = useState(false);
  const value = useMemo<BleTracePanelCtx>(
    () => ({ open: () => setIsOpen(true), close: () => setIsOpen(false), isOpen }),
    [isOpen],
  );
  return (
    <Ctx.Provider value={value}>
      {children}
      {isOpen && <BleTracePanel onClose={() => setIsOpen(false)} />}
    </Ctx.Provider>
  );
}

export function useBleTracePanel(): BleTracePanelCtx {
  const v = useContext(Ctx);
  if (!v) throw new Error('useBleTracePanel must be used inside <BleTracePanelProvider>');
  return v;
}

function BleTracePanel({ onClose }: { onClose: () => void }) {
  const { sessions, order } = useChips();
  const [frames, setFrames] = useState<AirFrameTrace[]>([]);

  // Pick any live bridge to poll — they all see the same air ring.
  const poller = useMemo(() => {
    for (const id of order) {
      const s = sessions[id];
      if (s?.bridge) return s.bridge;
    }
    return null;
  }, [sessions, order]);

  useEffect(() => {
    if (!poller) return;
    let alive = true;
    const tick = () => {
      if (!alive) return;
      try {
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        const raw = (poller as any).sim?.air_trace_snapshot?.();
        if (Array.isArray(raw)) setFrames(raw as AirFrameTrace[]);
      } catch {
        /* swallow — bridge may have been torn down */
      }
    };
    tick();
    const id = window.setInterval(tick, 250);
    return () => {
      alive = false;
      window.clearInterval(id);
    };
  }, [poller]);

  return (
    <div className="lw-ble-trace-panel">
      <div className="lw-ble-trace-header">
        <span>BLE air · last {frames.length} frames</span>
        <button type="button" onClick={onClose} aria-label="Close BLE trace">
          ×
        </button>
      </div>
      <div className="lw-ble-trace-body">
        {frames.length === 0 ? (
          <div className="lw-ble-trace-empty">
            No frames yet. Load a BLE TX firmware into a chip and run it
            — every transmitted packet shows up here.
          </div>
        ) : (
          <ul className="lw-ble-trace-list">
            {frames.map((f, i) => (
              <li key={i} className="lw-ble-trace-row">
                <span className="lw-ble-trace-channel">ch {f.channel}</span>
                <span className="lw-ble-trace-addr">
                  {f.addr_base.toString(16).padStart(8, '0').toUpperCase()}:
                  {f.addr_prefix.toString(16).padStart(2, '0').toUpperCase()}
                </span>
                <span className="lw-ble-trace-bytes">
                  {f.bytes
                    .slice(0, 16)
                    .map((b) => b.toString(16).padStart(2, '0'))
                    .join(' ')}
                  {f.bytes.length > 16 ? ` … (+${f.bytes.length - 16})` : ''}
                </span>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}

export const useBleTraceOpener = useBleTracePanel;
