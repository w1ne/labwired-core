import { useMemo } from 'react';
import { COMPONENT_REGISTRY } from '@labwired/ui';
import type { CommandItem } from './CommandPalette';
import type { BoardConfig } from '../bundled-configs';
import { STARTER_LABS } from './ChipRow';

export interface CommandPaletteContext {
  boards: BoardConfig[];
  onLoadBoard: (board: BoardConfig) => void;
  onPickLab: (labId: string) => void;
  onAddComponent: (type: string) => void;
  onRun: () => void;
  onShare: () => void;
  onReset: () => void;
  onToggleDev: () => void;
}

export function useCommandPaletteItems(ctx: CommandPaletteContext): CommandItem[] {
  return useMemo(() => {
    const items: CommandItem[] = [];

    for (const [type, def] of COMPONENT_REGISTRY.entries()) {
      if (type === 'mcu' || type.startsWith('boards/')) continue;
      items.push({
        id: `comp:${type}`,
        bucket: 'Components',
        label: def?.label ?? type,
        hint: 'drop on canvas',
        action: () => ctx.onAddComponent(type),
      });
    }

    for (const board of ctx.boards) {
      items.push({
        id: `board:${board.boardId}`,
        bucket: 'Boards',
        label: board.name,
        hint: board.arch,
        action: () => ctx.onLoadBoard(board),
      });
    }

    for (const lab of STARTER_LABS) {
      items.push({
        id: `lab:${lab.id}`,
        bucket: 'Examples',
        label: lab.name,
        hint: lab.locked ? lab.comingIn : 'open',
        action: () => ctx.onPickLab(lab.id),
      });
    }

    items.push(
      { id: 'act:run', bucket: 'Actions', label: 'Run simulation', hint: 'Space', action: ctx.onRun },
      { id: 'act:reset', bucket: 'Actions', label: 'Reset simulation', action: ctx.onReset },
      { id: 'act:share', bucket: 'Actions', label: 'Share project', action: ctx.onShare },
      { id: 'act:dev', bucket: 'Actions', label: 'Toggle Dev mode', action: ctx.onToggleDev },
    );

    return items;
  }, [ctx]);
}
