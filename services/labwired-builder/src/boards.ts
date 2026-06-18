// Typed loader over THE board catalog (board-catalog.json) — the single source of
// truth for the hosted PlatformIO compiler, shared by labwired and proto.cat.
//
// ┌─────────────────────────────────────────────────────────────────────────┐
// │ TO ADD A BOARD: add ONE entry to board-catalog.json. That is the job.    │
// │   • /compile and /boards pick it up automatically.                       │
// │   • The Docker image auto-bakes its PlatformIO framework at build time    │
// │     (warm-cache.ts derives the bake from the catalog — no second list).   │
// └─────────────────────────────────────────────────────────────────────────┘
//
// There is NO hardcoded board list in code: the catalog is data, loaded and
// validated here. proto.cat consumes the same catalog through /compile + /boards
// (server-side resolution), so neither side keeps its own copy.

import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

export interface PioBoard {
  /** PlatformIO platform, e.g. 'ststm32'. */
  platform: string;
  /** PlatformIO board id, e.g. 'nucleo_l476rg'. */
  board: string;
  /** PlatformIO framework, e.g. 'arduino' | 'stm32cube'. Omit for bare-metal. */
  framework?: string;
  /** True when the LabWired digital twin can execute this target today.
   *  Compile is available for all mapped boards; run is gated on the sim. */
  runnable: boolean;
  /** Extra platformio.ini lines (board_build.*, board_upload.*). Ours — never
   *  agent-supplied. */
  extra?: string[];
  /** Default PlatformIO library dependencies (request lib_deps merge on top). */
  libDeps?: string[];
  /** ESP flashing chip hint (esp32 | esp32s3 | esp32c3 | …); drives the
   *  flash-image bootloader offset. Inferred from the board when omitted. */
  espChip?: string;
}

interface Catalog {
  boards: Record<string, PioBoard>;
  chipFamilies: Record<string, PioBoard>;
}

function loadCatalog(): Catalog {
  const here = dirname(fileURLToPath(import.meta.url));
  const raw = JSON.parse(readFileSync(join(here, 'board-catalog.json'), 'utf8')) as Partial<Catalog>;
  const boards = raw.boards ?? {};
  const chipFamilies = raw.chipFamilies ?? {};
  // Fail loud at startup if the data is malformed — better than a confusing miss
  // at compile time.
  for (const [id, b] of [...Object.entries(boards), ...Object.entries(chipFamilies)]) {
    if (!b || !b.platform || !b.board) {
      throw new Error(`board-catalog.json: entry "${id}" missing platform/board`);
    }
  }
  return { boards, chipFamilies };
}

const CATALOG = loadCatalog();

/** Board id → PlatformIO target (exact-match tier). */
export const PIO_BOARDS: Record<string, PioBoard> = CATALOG.boards;
/** Chip family → PlatformIO target (fallback tier). */
export const CHIP_FAMILIES: Record<string, PioBoard> = CATALOG.chipFamilies;

/** Resolve a compile target. Prefers an exact board id, then a chip family —
 *  the same precedence proto.cat uses. Returns the matched board plus the id we
 *  resolved through (for diagnostics), or null when nothing matches. */
export function resolveBoard(
  boardId: string | undefined,
  chipFamily?: string,
): { board: PioBoard; source: string } | null {
  if (boardId && PIO_BOARDS[boardId]) return { board: PIO_BOARDS[boardId], source: `board[${boardId}]` };
  if (chipFamily && CHIP_FAMILIES[chipFamily]) return { board: CHIP_FAMILIES[chipFamily], source: `chip_family[${chipFamily}]` };
  return null;
}
