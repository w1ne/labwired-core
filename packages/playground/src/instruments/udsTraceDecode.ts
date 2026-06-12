import type { UartTraceSnapshot } from '@labwired/ui';

export interface UdsTraceRow {
  key: string;
  kind: 'request' | 'positive-response' | 'negative-response' | 'data' | 'status';
  service: string;
  detail: string;
}

function traceText(snapshots: UartTraceSnapshot[]): string {
  const bytes = snapshots
    .flatMap((snapshot) => snapshot.events)
    .filter((event) => event.direction === 'tx')
    .sort((a, b) => a.seq - b.seq)
    .map((event) => event.byte & 0xff);

  return String.fromCharCode(...bytes);
}

export function rowsForUdsTrace(snapshots: UartTraceSnapshot[]): UdsTraceRow[] {
  const text = traceText(snapshots);
  const rows: UdsTraceRow[] = [];

  if (text.includes('UDS_REQ_22_F190')) {
    rows.push({
      key: 'uds:req:22-f190',
      kind: 'request',
      service: '0x22',
      detail: 'ReadDataByIdentifier 0xF190',
    });
  }

  if (text.includes('UDS_RESP_62_F190')) {
    rows.push({
      key: 'uds:resp:62-f190',
      kind: 'positive-response',
      service: '0x62',
      detail: 'Positive response for DID 0xF190',
    });
  }

  const vin = text.match(/VIN=([A-Z0-9-]+)/)?.[1];
  if (vin) {
    rows.push({
      key: `uds:vin:${vin}`,
      kind: 'data',
      service: 'VIN',
      detail: vin,
    });
  }

  if (text.includes('UDS_OK')) {
    rows.push({
      key: 'uds:ok',
      kind: 'status',
      service: 'OK',
      detail: 'UDS smoke exchange completed',
    });
  }

  return rows;
}
