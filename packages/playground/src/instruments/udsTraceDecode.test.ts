import { describe, expect, it } from 'vitest';
import type { FdcanTraceFrame } from '@labwired/ui';
import { rowsForUdsTrace } from './udsTraceDecode';

const frame = (seq: number, id: number, data: number[], direction: 'tx' | 'rx' = 'tx'): FdcanTraceFrame => ({
  seq,
  peripheral: 'fdcan1',
  direction,
  id,
  data,
  extended: false,
  fd: data.length > 8,
  bitrate_switch: data.length > 8,
  remote: false,
});

describe('udsTraceDecode', () => {
  it('decodes the H563 ECU UDS exchange from FDCAN ISO-TP frames', () => {
    const vin = Array.from(new TextEncoder().encode('LABWIRED-H563-UDS'));

    expect(rowsForUdsTrace([
      frame(1, 0x7e0, [0x03, 0x22, 0xf1, 0x90]),
      frame(2, 0x7e0, [0x03, 0x22, 0xf1, 0x90], 'rx'),
      frame(3, 0x7e8, [0x00, 0x14, 0x62, 0xf1, 0x90, ...vin]),
      frame(4, 0x7e8, [0x00, 0x14, 0x62, 0xf1, 0x90, ...vin], 'rx'),
    ])).toEqual([
      {
        key: 'uds:req:1:22-f190',
        seq: 1,
        canId: '0x7E0',
        kind: 'request',
        service: '0x22',
        detail: 'ReadDataByIdentifier 0xF190',
        payload: '03 22 F1 90',
      },
      {
        key: 'uds:resp:3:62-f190',
        seq: 3,
        canId: '0x7E8',
        kind: 'positive-response',
        service: '0x62',
        detail: 'Positive response for DID 0xF190',
        payload: '00 14 62 F1 90 4C 41 42 57 49 52 45 44 2D 48 35 36 33 2D 55 44 53',
      },
      {
        key: 'uds:vin:3:LABWIRED-H563-UDS',
        seq: 3,
        canId: '0x7E8',
        kind: 'data',
        service: 'VIN',
        detail: 'LABWIRED-H563-UDS',
        payload: '4C 41 42 57 49 52 45 44 2D 48 35 36 33 2D 55 44 53',
      },
      {
        key: 'uds:ok:3',
        seq: 3,
        canId: '0x7E8',
        kind: 'status',
        service: 'OK',
        detail: 'UDS exchange decoded from CAN frames',
        payload: '',
      },
    ]);
  });
});
