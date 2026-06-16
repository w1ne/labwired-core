import { describe, expect, it } from 'vitest';
import { PLAYGROUND_BOARD_CATALOG } from '@labwired/board-config';
import { pickerBoards } from './bundled-configs';

describe('shared Playground board catalog', () => {
  it('matches the real Playground board picker entries', () => {
    const expected = pickerBoards().map((board) => ({
      id: board.boardId,
      name: board.name,
      description: board.description,
      board: board.chipId,
      target: board.chipId,
      mcu_component_type: board.mcuComponentType,
    }));
    const actual = PLAYGROUND_BOARD_CATALOG.map((board) => ({
      id: board.id,
      name: board.name,
      description: board.description,
      board: board.board,
      target: board.target,
      mcu_component_type: board.mcu_component_type,
    }));

    expect(actual).toEqual(expected);
  });
});
