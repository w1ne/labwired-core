import { describe, expect, it } from 'vitest';
import type { Diagram, Wire } from '@labwired/ui';
import {
  getIolinkDecoderBinding,
  getLogicAnalyzerChannelBindings,
  getUdsDecoderBinding,
} from './logicAnalyzerConnections';

const w = (fromPart: string, fromPin: string, toPart: string, toPin: string): Wire => ({
  from: { part: fromPart, pin: fromPin },
  to: { part: toPart, pin: toPin },
  color: '#888888',
});

const diagram: Diagram = {
  version: 1,
  board: 'stm32l476',
  parts: [
    { id: 'mcu', type: 'nucleo-l476rg', x: 0, y: 0, rotate: 0, attrs: {} },
    { id: 'iolink_master', type: 'iolink-master', x: 520, y: 300, rotate: 0, attrs: {} },
    { id: 'analyzer', type: 'logic-analyzer', x: 360, y: 300, rotate: 0, attrs: { decoder: 'auto' } },
  ],
  wires: [
    w('mcu', 'PA2', 'iolink_master', 'RX'),
    w('mcu', 'PA3', 'iolink_master', 'TX'),
    w('analyzer', 'CH0', 'mcu', 'PA2'),
    w('analyzer', 'CH1', 'iolink_master', 'TX'),
  ],
};

describe('logic analyzer connection inference', () => {
  it('resolves channel bindings across shared signal nets', () => {
    const bindings = getLogicAnalyzerChannelBindings(diagram, 'analyzer');
    expect(bindings.find((binding) => binding.channel === 'CH0')?.endpoints).toEqual(
      expect.arrayContaining([
        { part: 'mcu', pin: 'PA2' },
        { part: 'iolink_master', pin: 'RX' },
      ]),
    );
  });

  it('arms the IO-Link decoder when a channel taps TX or RX', () => {
    expect(getIolinkDecoderBinding(diagram, 'analyzer')).toEqual({
      connected: true,
      channels: [
        { channel: 'CH0', pin: 'RX' },
        { channel: 'CH1', pin: 'TX' },
      ],
    });
  });

  it('arms the UDS decoder when a channel taps the diagnostic CAN tool bus', () => {
    const udsDiagram: Diagram = {
      version: 1,
      board: 'stm32h563',
      parts: [
        { id: 'mcu', type: 'nucleo-h563zi', x: 0, y: 0, rotate: 0, attrs: {} },
        { id: 'can_xcvr', type: 'can-transceiver', x: 360, y: 170, rotate: 0, attrs: {} },
        { id: 'uds_tester', type: 'can-diagnostic-tool', x: 520, y: 170, rotate: 0, attrs: {} },
        { id: 'uds_probe', type: 'logic-analyzer', x: 760, y: 170, rotate: 0, attrs: { decoder: 'uds' } },
      ],
      wires: [
        w('mcu', 'PD1', 'can_xcvr', 'TXD'),
        w('mcu', 'PD0', 'can_xcvr', 'RXD'),
        w('can_xcvr', 'CAN_H', 'uds_tester', 'CAN_H'),
        w('can_xcvr', 'CAN_L', 'uds_tester', 'CAN_L'),
        w('uds_probe', 'CH0', 'uds_tester', 'CAN_H'),
        w('uds_probe', 'CH1', 'uds_tester', 'CAN_L'),
      ],
    };

    expect(getUdsDecoderBinding(udsDiagram, 'uds_probe')).toEqual({
      connected: true,
      channels: [
        { channel: 'CH0', part: 'can_xcvr', pin: 'CAN_H', peripheral: 'fdcan1' },
        { channel: 'CH0', part: 'uds_tester', pin: 'CAN_H', peripheral: 'fdcan1' },
        { channel: 'CH1', part: 'can_xcvr', pin: 'CAN_L', peripheral: 'fdcan1' },
        { channel: 'CH1', part: 'uds_tester', pin: 'CAN_L', peripheral: 'fdcan1' },
      ],
    });
  });

  it('does not arm UDS when the CAN transceiver is not wired back to FDCAN RX/TX', () => {
    const udsDiagram: Diagram = {
      version: 1,
      board: 'stm32h563',
      parts: [
        { id: 'mcu', type: 'nucleo-h563zi', x: 0, y: 0, rotate: 0, attrs: {} },
        { id: 'can_xcvr', type: 'can-transceiver', x: 360, y: 170, rotate: 0, attrs: {} },
        { id: 'uds_tester', type: 'can-diagnostic-tool', x: 520, y: 170, rotate: 0, attrs: {} },
        { id: 'uds_probe', type: 'logic-analyzer', x: 760, y: 170, rotate: 0, attrs: { decoder: 'uds' } },
      ],
      wires: [
        w('mcu', 'PD1', 'can_xcvr', 'TXD'),
        w('can_xcvr', 'CAN_H', 'uds_tester', 'CAN_H'),
        w('can_xcvr', 'CAN_L', 'uds_tester', 'CAN_L'),
        w('uds_probe', 'CH0', 'uds_tester', 'CAN_H'),
      ],
    };

    expect(getUdsDecoderBinding(udsDiagram, 'uds_probe')).toEqual({
      connected: false,
      channels: [],
    });
  });
});
