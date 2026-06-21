// packages/playground/src/lab-notes.test.ts
import { describe, it, expect } from 'vitest';
import { BOARD_CONFIGS } from './bundled-configs';
import { makeStarterDiagram, LAB_NOTES } from './App';
import { diagramToConfig } from '@labwired/board-config';

const visibleLabs = BOARD_CONFIGS.filter((c) => c.kind === 'lab' && !c.hidden);

describe('lab description notes', () => {
  it('every visible lab seeds exactly one note part with non-empty text', () => {
    for (const cfg of visibleLabs) {
      const diagram = makeStarterDiagram(cfg);
      const notes = diagram.parts.filter((p) => p.type === 'note');
      expect(notes, `${cfg.boardId} note count`).toHaveLength(1);
      expect(notes[0].attrs.text?.trim().length, `${cfg.boardId} note text`).toBeGreaterThan(0);
    }
  });

  it('LAB_NOTES has an entry for every visible lab and no orphan keys', () => {
    const labIds = new Set(visibleLabs.map((c) => c.boardId));
    for (const id of labIds) expect(LAB_NOTES[id], `missing note for ${id}`).toBeDefined();
    for (const key of Object.keys(LAB_NOTES)) expect(labIds.has(key), `orphan note key ${key}`).toBe(true);
  });

  it('bare (non-lab) boards seed no note', () => {
    const bare = BOARD_CONFIGS.find((c) => c.kind !== 'lab');
    if (bare) {
      const diagram = makeStarterDiagram(bare);
      expect(diagram.parts.some((p) => p.type === 'note')).toBe(false);
    }
  });

  it('a note never contributes a board_io binding', () => {
    const cfg = visibleLabs[0];
    const diagram = makeStarterDiagram(cfg);
    // Call diagramToConfig with the same arguments used in App.tsx (see run-config.ts).
    const generated = diagramToConfig(diagram, cfg.chipYaml);
    const json = JSON.stringify(generated ?? {});
    expect(json.includes('"note"')).toBe(false);
  });
});
