import { describe, expect, it } from 'vitest';
import type { UartTraceSnapshot } from '@labwired/ui';
import { rowsForUdsTrace } from './udsTraceDecode';

describe('udsTraceDecode', () => {
  it('decodes the H563 ECU firmware markers from UART trace bytes', () => {
    const text = [
      'H563-UDS-ECU',
      'UDS_REQ_22_F190',
      'UDS_RESP_62_F190',
      'VIN=LABWIRED-H563-UDS',
      'UDS_OK',
      '',
    ].join('\n');
    const snapshots: UartTraceSnapshot[] = [
      {
        peripheral: 'uart3',
        events: Array.from(new TextEncoder().encode(text), (byte, seq) => ({
          seq: seq + 1,
          direction: 'tx' as const,
          byte,
        })),
      },
    ];

    expect(rowsForUdsTrace(snapshots)).toEqual([
      {
        key: 'uds:req:22-f190',
        kind: 'request',
        service: '0x22',
        detail: 'ReadDataByIdentifier 0xF190',
      },
      {
        key: 'uds:resp:62-f190',
        kind: 'positive-response',
        service: '0x62',
        detail: 'Positive response for DID 0xF190',
      },
      {
        key: 'uds:vin:LABWIRED-H563-UDS',
        kind: 'data',
        service: 'VIN',
        detail: 'LABWIRED-H563-UDS',
      },
      {
        key: 'uds:ok',
        kind: 'status',
        service: 'OK',
        detail: 'UDS smoke exchange completed',
      },
    ]);
  });
});
