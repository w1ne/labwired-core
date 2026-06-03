import { describe, expect, it } from 'vitest';
import type { Diagram, Wire } from '@labwired/ui';
import {
  captureLogicAnalyzerSample,
  getDecoderAvailability,
  readGpioSnapshotPin,
} from './logicAnalyzerCapture';

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
    { id: 'led', type: 'led', x: 300, y: 80, rotate: 0, attrs: {} },
    { id: 'iolink_master', type: 'iolink-master', x: 520, y: 300, rotate: 0, attrs: {} },
    { id: 'analyzer', type: 'logic-analyzer', x: 360, y: 300, rotate: 0, attrs: { decoder: 'auto' } },
  ],
  wires: [
    w('mcu', 'PA5', 'led', 'A'),
    w('mcu', 'PA2', 'iolink_master', 'RX'),
    w('analyzer', 'CH0', 'led', 'A'),
    w('analyzer', 'CH1', 'mcu', 'PA2'),
  ],
};

describe('logic analyzer capture', () => {
  it('reads a GPIO line level from a STM32 modern GPIO snapshot', () => {
    expect(readGpioSnapshotPin({ moder: 0b01 << 10, odr: 1 << 5, idr: 0 }, 5)).toBe(1);
    expect(readGpioSnapshotPin({ moder: 0, odr: 0, idr: 1 << 5 }, 5)).toBe(1);
  });

  it('captures connected analyzer channels from live peripheral snapshots', () => {
    const sample = captureLogicAnalyzerSample({
      diagram,
      analyzerId: 'analyzer',
      nowMs: 125,
      getPeripheralSnapshot: (name) => (name === 'gpioa' ? { moder: 0b01 << 10, odr: 1 << 5, idr: 0 } : null),
    });

    expect(sample).toEqual({
      t: 125,
      channels: [
        { channel: 'CH0', value: 1, source: 'gpioa.5' },
        { channel: 'CH1', value: 0, source: 'gpioa.2' },
        { channel: 'CH2', value: null, source: null },
        { channel: 'CH3', value: null, source: null },
      ],
    });
  });

  it('reports IO-Link decoder availability from selected signal nets', () => {
    expect(getDecoderAvailability(diagram, 'analyzer')).toEqual({
      raw: true,
      iolink: true,
      uart: true,
      spi: false,
    });
  });
});
