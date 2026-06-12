import { describe, expect, it } from 'vitest';
import { makeStarterDiagram } from './App';
import { BOARD_CONFIGS } from './bundled-configs';

describe('makeStarterDiagram', () => {
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
