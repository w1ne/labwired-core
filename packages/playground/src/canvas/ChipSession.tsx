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
    [DEFAULT_CHIP_ID]: {
      chipId: DEFAULT_CHIP_ID,
      bridge: null,
      board: initialBoard,
      source: null,
      config: null,
    },
  }));
  const [order, setOrder] = useState<string[]>(() => [DEFAULT_CHIP_ID]);
  const [activeChipId, setActiveChipId] = useState<string>(DEFAULT_CHIP_ID);

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
      const resolvedBoard = board ?? BOARD_CONFIGS[0] ?? initialBoard;
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

  const value = useMemo<ChipsContext>(
    () => ({ sessions, order, activeChipId, setActiveChipId, setSession, addChip }),
    [sessions, order, activeChipId, setSession, addChip],
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
