import type { FdcanTraceFrame } from '@labwired/ui';

export interface UdsTraceRow {
  key: string;
  seq: number;
  canId: string;
  kind: 'request' | 'positive-response' | 'negative-response' | 'data' | 'status';
  service: string;
  detail: string;
  payload: string;
}

function hexByte(byte: number): string {
  return (byte & 0xff).toString(16).toUpperCase().padStart(2, '0');
}

function hexId(id: number): string {
  return `0x${(id >>> 0).toString(16).toUpperCase()}`;
}

function payloadHex(bytes: number[]): string {
  return bytes.map(hexByte).join(' ');
}

function isotpPayload(frame: FdcanTraceFrame): number[] | null {
  const data = frame.data.map((byte) => byte & 0xff);
  if (data.length === 0) return null;

  const pci = data[0];
  const frameType = pci >> 4;
  if (frameType === 0) {
    const nibbleLen = pci & 0x0f;
    if (nibbleLen === 0) {
      const len = data[1] ?? 0;
      return data.slice(2, 2 + len);
    }
    return data.slice(1, 1 + nibbleLen);
  }

  if (frameType === 1) {
    const len = ((pci & 0x0f) << 8) | (data[1] ?? 0);
    return data.slice(2, Math.min(data.length, 2 + len));
  }

  return null;
}

export function rowsForUdsTrace(frames: FdcanTraceFrame[]): UdsTraceRow[] {
  const rows: UdsTraceRow[] = [];
  const seen = new Set<string>();

  for (const frame of frames.slice().sort((a, b) => a.seq - b.seq)) {
    const payload = isotpPayload(frame);
    if (!payload || payload.length === 0) continue;

    const sid = payload[0];
    const did = payload.length >= 3 ? ((payload[1] << 8) | payload[2]) : null;
    const baseKey = `${frame.id}:${sid}:${did ?? ''}:${payloadHex(payload)}`;
    if (seen.has(baseKey)) continue;
    seen.add(baseKey);

    if (frame.id === 0x7e0 && sid === 0x22 && did === 0xf190) {
      rows.push({
        key: `uds:req:${frame.seq}:22-f190`,
        seq: frame.seq,
        canId: hexId(frame.id),
        kind: 'request',
        service: '0x22',
        detail: 'ReadDataByIdentifier 0xF190',
        payload: payloadHex(frame.data),
      });
      continue;
    }

    if (frame.id === 0x7e8 && sid === 0x62 && did === 0xf190) {
      rows.push({
        key: `uds:resp:${frame.seq}:62-f190`,
        seq: frame.seq,
        canId: hexId(frame.id),
        kind: 'positive-response',
        service: '0x62',
        detail: 'Positive response for DID 0xF190',
        payload: payloadHex(frame.data),
      });
      const vin = String.fromCharCode(...payload.slice(3)).replace(/\0+$/, '');
      if (vin.length > 0) {
        rows.push({
          key: `uds:vin:${frame.seq}:${vin}`,
          seq: frame.seq,
          canId: hexId(frame.id),
          kind: 'data',
          service: 'VIN',
          detail: vin,
          payload: payloadHex(payload.slice(3)),
        });
      }
      rows.push({
        key: `uds:ok:${frame.seq}`,
        seq: frame.seq,
        canId: hexId(frame.id),
        kind: 'status',
        service: 'OK',
        detail: 'UDS exchange decoded from CAN frames',
        payload: '',
      });
      continue;
    }

    if (frame.id === 0x7e8 && sid === 0x7f) {
      rows.push({
        key: `uds:nrc:${frame.seq}:${payloadHex(payload)}`,
        seq: frame.seq,
        canId: hexId(frame.id),
        kind: 'negative-response',
        service: '0x7F',
        detail: `Negative response to 0x${hexByte(payload[1] ?? 0)} NRC 0x${hexByte(payload[2] ?? 0)}`,
        payload: payloadHex(frame.data),
      });
    }
  }

  return rows;
}
