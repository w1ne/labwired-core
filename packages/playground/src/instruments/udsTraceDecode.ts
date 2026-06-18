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

// ISO 14229 service ids (request side). Positive responses are id + 0x40.
const SERVICES: Record<number, string> = {
  0x10: 'DiagnosticSessionControl',
  0x11: 'ECUReset',
  0x14: 'ClearDiagnosticInformation',
  0x19: 'ReadDTCInformation',
  0x22: 'ReadDataByIdentifier',
  0x23: 'ReadMemoryByAddress',
  0x27: 'SecurityAccess',
  0x28: 'CommunicationControl',
  0x29: 'Authentication',
  0x2a: 'ReadDataByPeriodicIdentifier',
  0x2c: 'DynamicallyDefineDataIdentifier',
  0x2e: 'WriteDataByIdentifier',
  0x2f: 'InputOutputControlByIdentifier',
  0x31: 'RoutineControl',
  0x34: 'RequestDownload',
  0x35: 'RequestUpload',
  0x36: 'TransferData',
  0x37: 'RequestTransferExit',
  0x3d: 'WriteMemoryByAddress',
  0x3e: 'TesterPresent',
  0x83: 'AccessTimingParameter',
  0x84: 'SecuredDataTransmission',
  0x85: 'ControlDTCSetting',
  0x86: 'ResponseOnEvent',
  0x87: 'LinkControl',
};

const NRC: Record<number, string> = {
  0x10: 'generalReject',
  0x11: 'serviceNotSupported',
  0x12: 'subFunctionNotSupported',
  0x13: 'incorrectMessageLengthOrInvalidFormat',
  0x14: 'responseTooLong',
  0x21: 'busyRepeatRequest',
  0x22: 'conditionsNotCorrect',
  0x24: 'requestSequenceError',
  0x31: 'requestOutOfRange',
  0x33: 'securityAccessDenied',
  0x35: 'invalidKey',
  0x36: 'exceedNumberOfAttempts',
  0x37: 'requiredTimeDelayNotExpired',
  0x70: 'uploadDownloadNotAccepted',
  0x72: 'generalProgrammingFailure',
  0x78: 'requestCorrectlyReceived-ResponsePending',
  0x7e: 'subFunctionNotSupportedInActiveSession',
  0x7f: 'serviceNotSupportedInActiveSession',
};

function hexByte(byte: number): string {
  return (byte & 0xff).toString(16).toUpperCase().padStart(2, '0');
}

function hexId(id: number): string {
  return `0x${(id >>> 0).toString(16).toUpperCase()}`;
}

function payloadHex(bytes: number[]): string {
  return bytes.map(hexByte).join(' ');
}

function serviceName(sid: number): string {
  return SERVICES[sid] ?? `Service 0x${hexByte(sid)}`;
}

// Render trailing bytes as a quoted ASCII string when they look like text
// (e.g. a VIN behind ReadDataByIdentifier), otherwise nothing.
function asciiTail(bytes: number[]): string {
  if (bytes.length < 2) return '';
  const printable = bytes.every((b) => b === 0 || (b >= 0x20 && b < 0x7f));
  if (!printable) return '';
  const s = String.fromCharCode(...bytes).replace(/\0+$/, '');
  return s.length >= 2 ? s : '';
}

interface Reassembly {
  expected: number;
  buf: number[];
}

/**
 * Decode an ISO-TP / UDS exchange from a CAN frame trace — controller- and
 * id-agnostic. Reassembles multi-frame messages (FF + CF), classifies
 * request / positive / negative responses by service id, and names the
 * service and NRC. Works for any ECU, any CAN id, FDCAN or bxCAN.
 */
export function rowsForUdsTrace(frames: FdcanTraceFrame[]): UdsTraceRow[] {
  const rows: UdsTraceRow[] = [];
  const asm = new Map<number, Reassembly>(); // per-CAN-id reassembly state
  let prevKey = '';

  for (const frame of frames.slice().sort((a, b) => a.seq - b.seq)) {
    const data = frame.data.map((byte) => byte & 0xff);
    if (data.length === 0) continue;

    // Internal loopback emits each frame as tx then rx; collapse the
    // back-to-back duplicate so reassembly isn't fed twice.
    const dupKey = `${frame.id}:${payloadHex(data)}`;
    if (dupKey === prevKey) continue;
    prevKey = dupKey;

    const pci = data[0];
    const frameType = pci >> 4;
    let sdu: number[] | null = null;

    if (frameType === 0x0) {
      // Single Frame (classic nibble length, or FD escape with len in byte 1).
      const nibble = pci & 0x0f;
      if (nibble === 0) {
        const len = data[1] ?? 0;
        sdu = data.slice(2, 2 + len);
      } else {
        sdu = data.slice(1, 1 + nibble);
      }
    } else if (frameType === 0x1) {
      // First Frame — begin reassembly, note it on the timeline.
      const len = ((pci & 0x0f) << 8) | (data[1] ?? 0);
      asm.set(frame.id, { expected: len, buf: data.slice(2) });
      rows.push({
        key: `isotp:ff:${frame.seq}`,
        seq: frame.seq,
        canId: hexId(frame.id),
        kind: 'status',
        service: 'ISO-TP',
        detail: `FirstFrame · ${len} bytes`,
        payload: payloadHex(data),
      });
      continue;
    } else if (frameType === 0x2) {
      // Consecutive Frame — append until the declared length is reached.
      const state = asm.get(frame.id);
      if (!state) continue;
      state.buf.push(...data.slice(1));
      if (state.buf.length < state.expected) continue;
      sdu = state.buf.slice(0, state.expected);
      asm.delete(frame.id);
    } else if (frameType === 0x3) {
      // Flow Control.
      const fs = pci & 0x0f;
      const name = fs === 0 ? 'ClearToSend' : fs === 1 ? 'Wait' : 'Overflow';
      rows.push({
        key: `isotp:fc:${frame.seq}`,
        seq: frame.seq,
        canId: hexId(frame.id),
        kind: 'status',
        service: 'ISO-TP',
        detail: `FlowControl · ${name}`,
        payload: payloadHex(data),
      });
      continue;
    }

    if (!sdu || sdu.length === 0) continue;
    const sid = sdu[0];

    if (sid === 0x7f) {
      const reqSid = sdu[1] ?? 0;
      const nrc = sdu[2] ?? 0;
      rows.push({
        key: `uds:nrc:${frame.seq}`,
        seq: frame.seq,
        canId: hexId(frame.id),
        kind: 'negative-response',
        service: '0x7F',
        detail: `${serviceName(reqSid)} rejected · NRC 0x${hexByte(nrc)} ${NRC[nrc] ?? ''}`.trim(),
        payload: payloadHex(sdu),
      });
      continue;
    }

    const isResponse = sid >= 0x40;
    const baseSid = isResponse ? sid - 0x40 : sid;
    const parts: string[] = [serviceName(baseSid)];
    if (isResponse) parts.push('positive response');

    if (baseSid === 0x22 || baseSid === 0x2e) {
      if (sdu.length >= 3) parts.push(`DID 0x${hexByte(sdu[1])}${hexByte(sdu[2])}`);
    } else if (sdu.length >= 2) {
      parts.push(`sub 0x${hexByte(sdu[1])}`);
    }

    const dataStart = baseSid === 0x22 ? 3 : 2;
    const ascii = asciiTail(sdu.slice(dataStart));
    if (ascii) parts.push(`"${ascii}"`);

    rows.push({
      key: `uds:${isResponse ? 'resp' : 'req'}:${frame.seq}`,
      seq: frame.seq,
      canId: hexId(frame.id),
      kind: isResponse ? 'positive-response' : 'request',
      service: `0x${hexByte(sid)}`,
      detail: parts.join(' · '),
      payload: payloadHex(sdu),
    });
  }

  return rows;
}
