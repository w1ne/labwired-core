// Phase 2b: per-chip session registry.
//
// Each ChipSession owns one SimulatorBridge (= one WasmSimulator inside
// the shared WASM module). The registry holds N sessions keyed by
// chipId; the canvas renders one ChipShape per session.
//
// The "active" chipId selects which session drives the existing
// single-chip studio UI (StudioShell, code editor, peripherals). All
// non-active sessions still tick every frame so that the cross-instance
// virtual-air registry in the RADIO peripheral (Rust-side `static
// OnceLock<Mutex<VirtualAir>>`) sees both transmitters — that's the
// load-bearing semantics for Phase 4's BLE-on-canvas demo.
import {
  createContext,
  useCallback,
  useContext,
  useMemo,
  useState,
  type ReactNode,
} from 'react';
import type { SimulatorBridge } from '@labwired/ui';
import { BOARD_CONFIGS, type BoardConfig } from '../bundled-configs';

export interface ChipSession {
  chipId: string;
  bridge: SimulatorBridge | null;
  board: BoardConfig;
}

interface ChipsContext {
  sessions: Record<string, ChipSession>;
  order: string[];
  activeChipId: string;
  setActiveChipId: (id: string) => void;
  setBridge: (chipId: string, bridge: SimulatorBridge | null) => void;
  setBoard: (chipId: string, board: BoardConfig) => void;
  addChip: (chipId?: string, board?: BoardConfig) => string;
}

const Ctx = createContext<ChipsContext | null>(null);

const DEFAULT_CHIP_ID = 'chip-default';

export function ChipsProvider({
  children,
  initialBoard,
}: {
  children: ReactNode;
  initialBoard: BoardConfig;
}) {
  const [sessions, setSessions] = useState<Record<string, ChipSession>>(() => ({
    [DEFAULT_CHIP_ID]: { chipId: DEFAULT_CHIP_ID, bridge: null, board: initialBoard },
  }));
  const [order, setOrder] = useState<string[]>(() => [DEFAULT_CHIP_ID]);
  const [activeChipId, setActiveChipId] = useState<string>(DEFAULT_CHIP_ID);

  const setBridge = useCallback((chipId: string, bridge: SimulatorBridge | null) => {
    setSessions((prev) => {
      const existing = prev[chipId];
      if (!existing) return prev;
      return { ...prev, [chipId]: { ...existing, bridge } };
    });
  }, []);

  const setBoard = useCallback((chipId: string, board: BoardConfig) => {
    setSessions((prev) => {
      const existing = prev[chipId];
      if (!existing) return prev;
      return { ...prev, [chipId]: { ...existing, board } };
    });
  }, []);

  const addChip = useCallback(
    (chipId?: string, board?: BoardConfig) => {
      let id = chipId ?? '';
      if (!id) {
        // Allocate next sequential id (`chip-1`, `chip-2`, …) so the
        // human-readable label matches creation order.
        let n = 1;
        while (sessions[`chip-${n}`]) n += 1;
        id = `chip-${n}`;
      }
      const resolvedBoard = board ?? BOARD_CONFIGS[0] ?? initialBoard;
      setSessions((prev) => ({
        ...prev,
        [id]: { chipId: id, bridge: null, board: resolvedBoard },
      }));
      setOrder((prev) => (prev.includes(id) ? prev : [...prev, id]));
      return id;
    },
    [sessions, initialBoard],
  );

  const value = useMemo<ChipsContext>(
    () => ({ sessions, order, activeChipId, setActiveChipId, setBridge, setBoard, addChip }),
    [sessions, order, activeChipId, setBridge, setBoard, addChip],
  );

  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useChips(): ChipsContext {
  const v = useContext(Ctx);
  if (!v) throw new Error('useChips must be used inside <ChipsProvider>');
  return v;
}

export function useChipSession(chipId: string): ChipSession | undefined {
  return useContext(Ctx)?.sessions[chipId];
}

export function useActiveChipSession(): ChipSession {
  const c = useChips();
  const s = c.sessions[c.activeChipId];
  if (!s) throw new Error(`No session for active chip ${c.activeChipId}`);
  return s;
}

export { DEFAULT_CHIP_ID };
