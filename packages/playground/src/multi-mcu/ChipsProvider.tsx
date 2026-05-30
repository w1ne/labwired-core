// Multi-binary session registry. Each entry is one independent
// SimulatorBridge (= one WasmSimulator inside the shared WASM
// module). N entries can co-exist and tick concurrently — required
// for cross-instance BLE (the radio's virtual-air registry is a
// process-static OnceLock<Mutex<VirtualAir>> on the Rust side, so
// all live bridges automatically share the air).
//
// Persistence: order + active + per-chip boardId saved to
// localStorage. Bridge/source/config are runtime-only.
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
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
  source: string | null;
  config: unknown;
}

interface ChipsContext {
  sessions: Record<string, ChipSession>;
  order: string[];
  activeChipId: string;
  setActiveChipId: (id: string) => void;
  setSession: (chipId: string, partial: Partial<Omit<ChipSession, 'chipId'>>) => void;
  addChip: (board?: BoardConfig) => string;
  removeChip: (chipId: string) => void;
  /// True iff the user has explicitly opened the active chip's
  /// properties (the bottom Serial/Registers/Trace/Memory/Source/
  /// YAML drawer). Default false — the drawer is hidden until the
  /// user picks a chip to inspect.
  propertiesOpen: boolean;
  setPropertiesOpen: (open: boolean) => void;
}

const Ctx = createContext<ChipsContext | null>(null);

const DEFAULT_CHIP_ID = 'chip-default';
const PERSISTENCE_KEY = 'lw-mcu-registry-v1';

interface PersistedRegistry {
  order: string[];
  activeChipId: string;
  boardIdByChip: Record<string, string>;
}

function loadRegistry(): PersistedRegistry | null {
  if (typeof window === 'undefined') return null;
  try {
    const raw = window.localStorage.getItem(PERSISTENCE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as PersistedRegistry;
    if (!Array.isArray(parsed.order) || !parsed.activeChipId) return null;
    return parsed;
  } catch {
    return null;
  }
}

function saveRegistry(reg: PersistedRegistry) {
  if (typeof window === 'undefined') return;
  try {
    window.localStorage.setItem(PERSISTENCE_KEY, JSON.stringify(reg));
  } catch {
    /* quota — skip */
  }
}

export function ChipsProvider({
  children,
  initialBoard,
}: {
  children: ReactNode;
  initialBoard: BoardConfig;
}) {
  const [sessions, setSessions] = useState<Record<string, ChipSession>>(() => {
    const persisted = loadRegistry();
    if (persisted) {
      const out: Record<string, ChipSession> = {};
      for (const id of persisted.order) {
        const boardId = persisted.boardIdByChip[id];
        const board = BOARD_CONFIGS.find((c) => c.boardId === boardId) ?? initialBoard;
        out[id] = { chipId: id, bridge: null, board, source: null, config: null };
      }
      if (!out[DEFAULT_CHIP_ID]) {
        out[DEFAULT_CHIP_ID] = {
          chipId: DEFAULT_CHIP_ID,
          bridge: null,
          board: initialBoard,
          source: null,
          config: null,
        };
      }
      return out;
    }
    return {
      [DEFAULT_CHIP_ID]: {
        chipId: DEFAULT_CHIP_ID,
        bridge: null,
        board: initialBoard,
        source: null,
        config: null,
      },
    };
  });
  const [order, setOrder] = useState<string[]>(() => {
    const persisted = loadRegistry();
    if (persisted && persisted.order.length > 0) {
      return persisted.order.includes(DEFAULT_CHIP_ID)
        ? persisted.order
        : [DEFAULT_CHIP_ID, ...persisted.order];
    }
    return [DEFAULT_CHIP_ID];
  });
  const [activeChipId, setActiveChipId] = useState<string>(() => {
    const persisted = loadRegistry();
    if (persisted && persisted.order.includes(persisted.activeChipId)) {
      return persisted.activeChipId;
    }
    return DEFAULT_CHIP_ID;
  });
  // Properties are a per-chip attribute. The drawer is hidden by
  // default; user opens it by clicking a chip's Properties button
  // in the McuStrip (same on desktop and mobile). The drawer then
  // shows that chip's Serial / Registers / Trace / Memory / Source
  // / YAML.
  const [propertiesOpen, setPropertiesOpen] = useState(false);

  useEffect(() => {
    const boardIdByChip: Record<string, string> = {};
    for (const id of order) {
      const s = sessions[id];
      if (s) boardIdByChip[id] = s.board.boardId;
    }
    saveRegistry({ order, activeChipId, boardIdByChip });
  }, [order, activeChipId, sessions]);

  const setSession = useCallback(
    (chipId: string, partial: Partial<Omit<ChipSession, 'chipId'>>) => {
      setSessions((prev) => {
        const existing = prev[chipId];
        if (!existing) return prev;
        let changed = false;
        for (const k of Object.keys(partial) as Array<keyof typeof partial>) {
          if (existing[k as keyof ChipSession] !== partial[k]) {
            changed = true;
            break;
          }
        }
        if (!changed) return prev;
        return { ...prev, [chipId]: { ...existing, ...partial } };
      });
    },
    [],
  );

  const addChip = useCallback(
    (board?: BoardConfig) => {
      // Default new MCUs to nRF52840 so the BLE demo path is one
      // click away (both chips share virtual_air on the Rust side).
      const nrf = BOARD_CONFIGS.find((c) => c.boardId === 'nrf52840-dk');
      const resolvedBoard = board ?? nrf ?? initialBoard ?? BOARD_CONFIGS[0];
      let n = 1;
      while (sessions[`chip-${n}`]) n += 1;
      const id = `chip-${n}`;
      setSessions((prev) => ({
        ...prev,
        [id]: { chipId: id, bridge: null, board: resolvedBoard, source: null, config: null },
      }));
      setOrder((prev) => (prev.includes(id) ? prev : [...prev, id]));
      return id;
    },
    [sessions, initialBoard],
  );

  const removeChip = useCallback((chipId: string) => {
    if (chipId === DEFAULT_CHIP_ID) return;
    setSessions((prev) => {
      if (!prev[chipId]) return prev;
      const { [chipId]: gone, ...rest } = prev;
      try {
        gone.bridge?.dispose?.();
      } catch {
        /* bridge may have been torn down */
      }
      return rest;
    });
    setOrder((prev) => prev.filter((id) => id !== chipId));
    setActiveChipId((prev) => (prev === chipId ? DEFAULT_CHIP_ID : prev));
  }, []);

  const value = useMemo<ChipsContext>(
    () => ({
      sessions,
      order,
      activeChipId,
      setActiveChipId,
      setSession,
      addChip,
      removeChip,
      propertiesOpen,
      setPropertiesOpen,
    }),
    [sessions, order, activeChipId, setSession, addChip, removeChip, propertiesOpen],
  );

  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useChips(): ChipsContext {
  const v = useContext(Ctx);
  if (!v) throw new Error('useChips must be used inside <ChipsProvider>');
  return v;
}

export { DEFAULT_CHIP_ID };
