import { describe, expect, it } from 'vitest';
import { getPlaygroundBoard, listPlaygroundBoards, PLAYGROUND_BOARD_CATALOG } from '../src/boards';

describe('playground board catalog', () => {
  it('exposes real Playground ids and does not expose invented aliases', () => {
    const ids = PLAYGROUND_BOARD_CATALOG.map((board) => board.id);

    expect(ids).toContain('stm32f103-blinky');
    expect(ids).toContain('nucleo-f401re');
    expect(ids).toContain('nucleo-h563zi');
    expect(ids).not.toContain('stm32l476-blinky');
  });

  it('resolves and filters board entries for hosted MCP', () => {
    expect(getPlaygroundBoard('stm32f103-blinky')).toMatchObject({
      board: 'stm32f103',
      target: 'stm32f103',
      mcu_component_type: 'stm32-dev',
    });
    expect(listPlaygroundBoards('h563').map((board) => board.id)).toEqual(['nucleo-h563zi']);
  });
});
