import { describe, expect, it } from 'vitest';
import { rowsForUartTrace } from './uartTraceDecode';
import type { UartDecoderBinding } from './logicAnalyzerConnections';
import type { UartTraceSnapshot } from '@labwired/ui';

const binding: UartDecoderBinding = {
  connected: true,
  channels: [
    { channel: 'CH0', peripheral: 'uart2', role: 'tx', pin: 'PA2' },
    { channel: 'CH1', peripheral: 'uart2', role: 'rx', pin: 'PA3' },
  ],
};

describe('uartTraceDecode', () => {
  it('maps non-consuming core UART trace events to selected analyzer channels', () => {
    const snapshots: UartTraceSnapshot[] = [
      {
        peripheral: 'uart2',
        events: [
          { seq: 1, direction: 'tx', byte: 0x41 },
          { seq: 2, direction: 'rx', byte: 0x33 },
        ],
      },
      {
        peripheral: 'uart1',
        events: [{ seq: 1, direction: 'tx', byte: 0xff }],
      },
    ];

    expect(rowsForUartTrace(snapshots, binding)).toEqual([
      { key: 'uart2:1:tx:65', seq: 1, channel: 'CH0', peripheral: 'uart2', direction: 'tx', byte: 0x41 },
      { key: 'uart2:2:rx:51', seq: 2, channel: 'CH1', peripheral: 'uart2', direction: 'rx', byte: 0x33 },
    ]);
  });
});
