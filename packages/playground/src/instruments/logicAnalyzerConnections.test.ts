import { describe, expect, it } from 'vitest';
import type { Diagram, Wire } from '@labwired/ui';
import { getIolinkDecoderBinding, getLogicAnalyzerChannelBindings } from './logicAnalyzerConnections';

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
});
