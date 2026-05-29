// Phase 2b + 3: per-chip session registry.
//
// Each ChipSession holds the per-chip state that, when switched into
// App's local state, makes that chip "active" — its SimulatorBridge,
// board, source code, and the resolved YAML/firmware config used to
// instantiate the bridge.
//
// Phase 3 adds focus switching: clicking an inactive ChipCard now
// pauses the current chip, snapshots its state into the registry, and
// restores the target chip's state into App. The user keeps both
// chips' code/firmware while editing only one at a time.
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

/// localStorage key for the lightweight per-chip registry (chipId +
/// boardId + activeChipId). The bridge / source / config aren't
/// persisted — they're transient runtime state. The tldraw canvas
/// snapshot persists separately via its own `persistenceKey`.
const PERSISTENCE_KEY = 'lw-chips-registry-v1';

interface PersistedChipRegistry {
  order: string[];
  activeChipId: string;
  boardIdByChip: Record<string, string>;
}

function loadRegistry(): PersistedChipRegistry | null {
  if (typeof window === 'undefined') return null;
  try {
    const raw = window.localStorage.getItem(PERSISTENCE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as PersistedChipRegistry;
    if (!Array.isArray(parsed.order) || !parsed.activeChipId) return null;
    return parsed;
  } catch {
    return null;
  }
}

function saveRegistry(reg: PersistedChipRegistry) {
  if (typeof window === 'undefined') return;
  try {
    window.localStorage.setItem(PERSISTENCE_KEY, JSON.stringify(reg));
  } catch {
    /* quota / private mode — skip */
  }
}

export interface ChipSession {
  chipId: string;
  bridge: SimulatorBridge | null;
  board: BoardConfig;
  source: string | null;
  // Opaque sim config blob (yaml + firmware bytes) — used by App.tsx
  // when re-instantiating a bridge. Kept as `unknown` here so the
  // registry doesn't depend on the playground's internal types.
  config: unknown;
}

interface ChipsContext {
  sessions: Record<string, ChipSession>;
  order: string[];
  activeChipId: string;
  setActiveChipId: (id: string) => void;
  setSession: (chipId: string, partial: Partial<Omit<ChipSession, 'chipId'>>) => void;
  addChip: (chipId?: string, board?: BoardConfig) => string;
  /// Remove a chip from the registry. chip-default is protected
  /// (it's the always-present root chip); attempting to remove it
  /// is a no-op. Removing the currently active chip falls focus
  /// back to chip-default.
  removeChip: (chipId: string) => void;
  /// Whether the floating inspector window is currently open.
  /// Lifted into the context so any chip-card click can reopen it
  /// after the user dismissed it with the X.
  inspectorOpen: boolean;
  setInspectorOpen: (open: boolean) => void;
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
  const [sessions, setSessions] = useState<Record<string, ChipSession>>(() => {
    const persisted = loadRegistry();
    if (persisted) {
      const out: Record<string, ChipSession> = {};
      for (const id of persisted.order) {
        const boardId = persisted.boardIdByChip[id];
        const board = BOARD_CONFIGS.find((c) => c.boardId === boardId) ?? initialBoard;
        out[id] = { chipId: id, bridge: null, board, source: null, config: null };
      }
      // Ensure chip-default always exists even if persisted state lost it.
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
  const [inspectorOpen, setInspectorOpen] = useState<boolean>(true);

  // Mirror to localStorage on any structural change.
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
        // Cheap shallow change check so mirror effects that re-write
        // identical values don't re-render every consumer.
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
    (chipId?: string, board?: BoardConfig) => {
      let id = chipId ?? '';
      if (!id) {
        let n = 1;
        while (sessions[`chip-${n}`]) n += 1;
        id = `chip-${n}`;
      }
      // Default new chips to nRF52840 so two-chip setups auto-spawn
      // the BLE air edge — that's the discoverable demo path. Fall
      // back to the active chip's board, then the first catalog
      // entry if nRF52840 isn't present.
      const nrf = BOARD_CONFIGS.find((c) => c.boardId === 'nrf52840-dk');
      const resolvedBoard = board ?? nrf ?? initialBoard ?? BOARD_CONFIGS[0];
      setSessions((prev) => ({
        ...prev,
        [id]: {
          chipId: id,
          bridge: null,
          board: resolvedBoard,
          source: null,
          config: null,
        },
      }));
      setOrder((prev) => (prev.includes(id) ? prev : [...prev, id]));
      return id;
    },
    [sessions, initialBoard],
  );

  const removeChip = useCallback(
    (chipId: string) => {
      if (chipId === DEFAULT_CHIP_ID) return;
      setSessions((prev) => {
        if (!prev[chipId]) return prev;
        const { [chipId]: _gone, ...rest } = prev;
        // Free the WASM-side bridge if one was attached so we don't
        // leak a SimulatorBridge for the deleted chip.
        try {
          _gone.bridge?.dispose?.();
        } catch {
          /* bridge may have already been torn down */
        }
        return rest;
      });
      setOrder((prev) => prev.filter((id) => id !== chipId));
      setActiveChipId((prev) => (prev === chipId ? DEFAULT_CHIP_ID : prev));
    },
    [],
  );

  const value = useMemo<ChipsContext>(
    () => ({
      sessions,
      order,
      activeChipId,
      setActiveChipId,
      setSession,
      addChip,
      removeChip,
      inspectorOpen,
      setInspectorOpen,
    }),
    [sessions, order, activeChipId, setSession, addChip, removeChip, inspectorOpen],
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
