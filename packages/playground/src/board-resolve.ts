import { type BoardConfig } from './bundled-configs';

/**
 * Resolve which BoardConfig a diagram part represents.
 *
 * Order:
 *   1. attrs.boardId — explicit, set on MCU parts in multi-board labs so two
 *      parts sharing an mcuComponentType (e.g. both 'nrf52840-dk') run their
 *      own firmware. This makes the BLE sensor + collector distinguishable on
 *      one canvas.
 *   2. id === 'mcu' -> the workspace's primary board.
 *   3. first board whose mcuComponentType matches the part's type.
 *   4. null.
 */
export function resolveBoardForPart(
  part: { id: string; type: string; attrs?: Record<string, unknown> | null },
  primaryBoard: BoardConfig,
  boards: readonly BoardConfig[],
): BoardConfig | null {
  const boardId = part.attrs?.boardId;
  if (typeof boardId === 'string') {
    const byId = boards.find((b) => b.boardId === boardId);
    if (byId) return byId;
  }
  if (part.id === 'mcu') return primaryBoard;
  return boards.find((b) => b.mcuComponentType === part.type) ?? null;
}
