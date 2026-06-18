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
  it('decodes a single-frame ReadDataByIdentifier exchange (any ECU id)', () => {
    const vin = Array.from(new TextEncoder().encode('LABWIRED-H563-UDS'));

    expect(
      rowsForUdsTrace([
        frame(1, 0x7e0, [0x03, 0x22, 0xf1, 0x90]),
        frame(2, 0x7e0, [0x03, 0x22, 0xf1, 0x90], 'rx'),
        frame(3, 0x7e8, [0x00, 0x14, 0x62, 0xf1, 0x90, ...vin]),
        frame(4, 0x7e8, [0x00, 0x14, 0x62, 0xf1, 0x90, ...vin], 'rx'),
      ]),
    ).toEqual([
      {
        key: 'uds:req:1',
        seq: 1,
        canId: '0x7E0',
        kind: 'request',
        service: '0x22',
        detail: 'ReadDataByIdentifier · DID 0xF190',
        payload: '22 F1 90',
      },
      {
        key: 'uds:resp:3',
        seq: 3,
        canId: '0x7E8',
        kind: 'positive-response',
        service: '0x62',
        detail: 'ReadDataByIdentifier · positive response · DID 0xF190 · "LABWIRED-H563-UDS"',
        payload: '62 F1 90 4C 41 42 57 49 52 45 44 2D 48 35 36 33 2D 55 44 53',
      },
    ]);
  });

  it('reassembles a multi-frame SecurityAccess exchange (F103 bxCAN, issue #29)', () => {
    expect(
      rowsForUdsTrace([
        frame(1, 0x111, [0x10, 0x0b, 0x27, 0x01, 0x5a, 0x11, 0x22, 0x33]),
        frame(2, 0x111, [0x10, 0x0b, 0x27, 0x01, 0x5a, 0x11, 0x22, 0x33], 'rx'),
        frame(3, 0x222, [0x30, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]),
        frame(4, 0x222, [0x30, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], 'rx'),
        frame(5, 0x111, [0x21, 0x44, 0x55, 0x66, 0x77, 0x88, 0x55, 0x55]),
        frame(6, 0x111, [0x21, 0x44, 0x55, 0x66, 0x77, 0x88, 0x55, 0x55], 'rx'),
        frame(7, 0x222, [0x06, 0x67, 0x01, 0xde, 0xad, 0xbe, 0xef, 0x00]),
        frame(8, 0x222, [0x06, 0x67, 0x01, 0xde, 0xad, 0xbe, 0xef, 0x00], 'rx'),
      ]),
    ).toEqual([
      {
        key: 'isotp:ff:1',
        seq: 1,
        canId: '0x111',
        kind: 'status',
        service: 'ISO-TP',
        detail: 'FirstFrame · 11 bytes',
        payload: '10 0B 27 01 5A 11 22 33',
      },
      {
        key: 'isotp:fc:3',
        seq: 3,
        canId: '0x222',
        kind: 'status',
        service: 'ISO-TP',
        detail: 'FlowControl · ClearToSend',
        payload: '30 08 00 00 00 00 00 00',
      },
      {
        key: 'uds:req:5',
        seq: 5,
        canId: '0x111',
        kind: 'request',
        service: '0x27',
        detail: 'SecurityAccess · sub 0x01',
        payload: '27 01 5A 11 22 33 44 55 66 77 88',
      },
      {
        key: 'uds:resp:7',
        seq: 7,
        canId: '0x222',
        kind: 'positive-response',
        service: '0x67',
        detail: 'SecurityAccess · positive response · sub 0x01',
        payload: '67 01 DE AD BE EF',
      },
    ]);
  });

  it('decodes a negative response with a named NRC', () => {
    expect(
      rowsForUdsTrace([frame(1, 0x7e8, [0x03, 0x7f, 0x27, 0x35])]),
    ).toEqual([
      {
        key: 'uds:nrc:1',
        seq: 1,
        canId: '0x7E8',
        kind: 'negative-response',
        service: '0x7F',
        detail: 'SecurityAccess rejected · NRC 0x35 invalidKey',
        payload: '7F 27 35',
      },
    ]);
  });
});
