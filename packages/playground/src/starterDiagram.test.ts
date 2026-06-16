import { describe, expect, it } from 'vitest';
import { makeStarterDiagram, prepareSharedProjectForPlayground, resolveSharedBoardConfig } from './App';
import { BOARD_CONFIGS } from './bundled-configs';

describe('makeStarterDiagram', () => {
  it('opens generic STM32F103 agent shares on the existing Blinky catalog board', () => {
    const config = resolveSharedBoardConfig({ version: 1, board: 'stm32f103-blinky', parts: [], wires: [] });

    expect(config.boardId).toBe('stm32f103-blinky');
    expect(config.name).toBe('STM32F103 Blinky');
    expect(config.mcuComponentType).toBe('stm32-dev');
  });

  it('upgrades a generic shared MCU part to the selected board component', () => {
    const project = prepareSharedProjectForPlayground({
      version: 1,
      board: 'stm32f103-blinky',
      parts: [
        { id: 'mcu', type: 'mcu', x: 140, y: 140, rotate: 0, attrs: {} },
        { id: 'led1', type: 'led', x: 290, y: 140, rotate: 0, attrs: { color: 'green' } },
      ],
      wires: [{ from: { part: 'mcu', pin: 'PA5' }, to: { part: 'led1', pin: 'A' }, color: '#e83e8c' }],
    });

    expect(project.board.boardId).toBe('stm32f103-blinky');
    expect(project.diagram.board).toBe('stm32f103');
    expect(project.diagram.parts.find((part) => part.id === 'mcu')?.type).toBe('stm32-dev');
  });

  it('opens the STM32H5 UDS ECU lab with a diagnostic tester and UDS analyzer', () => {
    const config = BOARD_CONFIGS.find((candidate) => candidate.boardId === 'stm32h5-uds-ecu');
    expect(config).toBeTruthy();

    const diagram = makeStarterDiagram(config!);

    expect(diagram.parts.map((part) => part.type)).toEqual(
      expect.arrayContaining(['nucleo-h563zi', 'can-transceiver', 'can-diagnostic-tool', 'logic-analyzer']),
    );
    expect(diagram.parts.find((part) => part.type === 'logic-analyzer')?.attrs.decoder).toBe('uds');
    expect(diagram.wires).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          from: { part: 'mcu', pin: 'PD1' },
          to: { part: 'can_xcvr', pin: 'TXD' },
        }),
        expect.objectContaining({
          from: { part: 'mcu', pin: 'PD0' },
          to: { part: 'can_xcvr', pin: 'RXD' },
        }),
        expect.objectContaining({
          from: { part: 'can_xcvr', pin: 'CAN_H' },
          to: { part: 'uds_tester', pin: 'CAN_H' },
        }),
        expect.objectContaining({
          from: { part: 'can_xcvr', pin: 'CAN_L' },
          to: { part: 'uds_tester', pin: 'CAN_L' },
        }),
        expect.objectContaining({
          from: { part: 'uds_probe', pin: 'CH0' },
          to: { part: 'uds_tester', pin: 'CAN_H' },
        }),
        expect.objectContaining({
          from: { part: 'uds_probe', pin: 'CH1' },
          to: { part: 'uds_tester', pin: 'CAN_L' },
        }),
      ]),
    );
  });
});
