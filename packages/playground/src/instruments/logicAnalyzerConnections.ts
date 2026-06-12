import { findPinFunction, getPinMapping, type Diagram, type WireEndpoint } from '@labwired/ui';

export interface LogicAnalyzerChannelBinding {
  channel: string;
  endpoints: WireEndpoint[];
}

export interface IolinkDecoderBinding {
  connected: boolean;
  channels: { channel: string; pin: 'TX' | 'RX' }[];
}

export interface UartDecoderBinding {
  connected: boolean;
  channels: { channel: string; peripheral: string; role: 'tx' | 'rx'; pin: string }[];
}

export interface UdsDecoderBinding {
  connected: boolean;
  channels: { channel: string; part: string; pin: 'CAN_H' | 'CAN_L'; peripheral: string }[];
}

const ANALYZER_TYPE = 'logic-analyzer';
const CHANNELS = ['CH0', 'CH1', 'CH2', 'CH3'];

function endpointKey(endpoint: WireEndpoint): string {
  return `${endpoint.part}:${endpoint.pin}`;
}

function endpointEquals(a: WireEndpoint, b: WireEndpoint): boolean {
  return a.part === b.part && a.pin === b.pin;
}

function connectedEndpoints(diagram: Diagram, start: WireEndpoint): WireEndpoint[] {
  const byKey = new Map<string, WireEndpoint[]>();
  const addEdge = (a: WireEndpoint, b: WireEndpoint) => {
    const key = endpointKey(a);
    const next = byKey.get(key) ?? [];
    next.push(b);
    byKey.set(key, next);
  };

  for (const wire of diagram.wires) {
    addEdge(wire.from, wire.to);
    addEdge(wire.to, wire.from);
  }

  const out: WireEndpoint[] = [];
  const seen = new Set<string>();
  const queue = [start];
  while (queue.length > 0) {
    const endpoint = queue.shift()!;
    const key = endpointKey(endpoint);
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(endpoint);
    for (const next of byKey.get(key) ?? []) {
      if (!seen.has(endpointKey(next))) queue.push(next);
    }
  }
  return out;
}

export function getLogicAnalyzerChannelBindings(
  diagram: Diagram,
  analyzerId: string,
): LogicAnalyzerChannelBinding[] {
  const analyzer = diagram.parts.find((part) => part.id === analyzerId && part.type === ANALYZER_TYPE);
  if (!analyzer) return [];

  return CHANNELS.map((channel) => {
    const start = { part: analyzer.id, pin: channel };
    return {
      channel,
      endpoints: connectedEndpoints(diagram, start).filter((endpoint) => !endpointEquals(endpoint, start)),
    };
  });
}

export function getIolinkDecoderBinding(diagram: Diagram, analyzerId: string): IolinkDecoderBinding {
  const channels: IolinkDecoderBinding['channels'] = [];
  for (const binding of getLogicAnalyzerChannelBindings(diagram, analyzerId)) {
    for (const endpoint of binding.endpoints) {
      const part = diagram.parts.find((candidate) => candidate.id === endpoint.part);
      if (part?.type !== 'iolink-master') continue;
      if (endpoint.pin === 'TX' || endpoint.pin === 'RX') {
        channels.push({ channel: binding.channel, pin: endpoint.pin });
      }
    }
  }

  const unique = channels.filter(
    (channel, index) =>
      channels.findIndex((candidate) => candidate.channel === channel.channel && candidate.pin === channel.pin) === index,
  );

  return { connected: unique.length > 0, channels: unique };
}

export function getUartDecoderBinding(diagram: Diagram, analyzerId: string): UartDecoderBinding {
  const channels: UartDecoderBinding['channels'] = [];

  for (const binding of getLogicAnalyzerChannelBindings(diagram, analyzerId)) {
    for (const endpoint of binding.endpoints) {
      const part = diagram.parts.find((candidate) => candidate.id === endpoint.part);
      if (part?.type === 'iolink-master' && (endpoint.pin === 'TX' || endpoint.pin === 'RX')) {
        channels.push({
          channel: binding.channel,
          peripheral: 'iolink',
          role: endpoint.pin.toLowerCase() as 'tx' | 'rx',
          pin: endpoint.pin,
        });
        continue;
      }

      if (endpoint.part !== 'mcu') continue;

      const mapping = getPinMapping(diagram.board, endpoint.pin);
      const uart = mapping?.functions.find(
        (fn) => fn.type === 'uart' && (fn.role === 'tx' || fn.role === 'rx'),
      );
      if (!uart || (uart.role !== 'tx' && uart.role !== 'rx')) continue;

      channels.push({
        channel: binding.channel,
        peripheral: uart.peripheral,
        role: uart.role,
        pin: endpoint.pin,
      });
    }
  }

  const unique = channels.filter(
    (channel, index) =>
      channels.findIndex(
        (candidate) =>
          candidate.channel === channel.channel &&
          candidate.peripheral === channel.peripheral &&
          candidate.role === channel.role &&
          candidate.pin === channel.pin,
      ) === index,
  );

  return { connected: unique.length > 0, channels: unique };
}

export function getUdsDecoderBinding(diagram: Diagram, analyzerId: string): UdsDecoderBinding {
  const channels: UdsDecoderBinding['channels'] = [];

  const canPeripheralForTransceiver = (partId: string): string | null => {
    const peripheralFor = (pin: 'TXD' | 'RXD', role: 'tx' | 'rx'): string | null => {
      const matches = new Set<string>();
      for (const endpoint of connectedEndpoints(diagram, { part: partId, pin })) {
        if (endpoint.part !== 'mcu') continue;
        const fn = findPinFunction(diagram.board, endpoint.pin, 'can');
        if (fn?.role === role) matches.add(fn.peripheral);
      }
      return matches.size === 1 ? [...matches][0] : null;
    };

    const tx = peripheralFor('TXD', 'tx');
    const rx = peripheralFor('RXD', 'rx');
    if (tx && rx && tx === rx) {
      return tx;
    }
    return null;
  };

  const canPeripheralForEndpoint = (endpoint: WireEndpoint): string | null => {
    const part = diagram.parts.find((candidate) => candidate.id === endpoint.part);
    if (part?.type === 'can-transceiver') {
      return canPeripheralForTransceiver(endpoint.part);
    }

    if (part?.type !== 'can-diagnostic-tool') return null;
    for (const peer of connectedEndpoints(diagram, endpoint)) {
      const peerPart = diagram.parts.find((candidate) => candidate.id === peer.part);
      if (peerPart?.type !== 'can-transceiver') continue;
      if (peer.pin !== 'CAN_H' && peer.pin !== 'CAN_L') continue;
      const peripheral = canPeripheralForTransceiver(peer.part);
      if (peripheral) return peripheral;
    }
    return null;
  };

  for (const binding of getLogicAnalyzerChannelBindings(diagram, analyzerId)) {
    for (const endpoint of binding.endpoints) {
      const part = diagram.parts.find((candidate) => candidate.id === endpoint.part);
      const isDiagnosticToolCanPin =
        part?.type === 'can-diagnostic-tool' && (endpoint.pin === 'CAN_H' || endpoint.pin === 'CAN_L');
      const isTransceiverCanPin =
        part?.type === 'can-transceiver' && (endpoint.pin === 'CAN_H' || endpoint.pin === 'CAN_L');

      if (!isDiagnosticToolCanPin && !isTransceiverCanPin) continue;
      const peripheral = canPeripheralForEndpoint(endpoint);
      if (!peripheral) continue;

      channels.push({
        channel: binding.channel,
        part: endpoint.part,
        pin: endpoint.pin as 'CAN_H' | 'CAN_L',
        peripheral,
      });
    }
  }

  const unique = channels.filter(
    (channel, index) =>
      channels.findIndex(
        (candidate) =>
          candidate.channel === channel.channel &&
          candidate.part === channel.part &&
          candidate.pin === channel.pin &&
          candidate.peripheral === channel.peripheral,
      ) === index,
  );

  unique.sort((a, b) =>
    a.channel.localeCompare(b.channel)
    || a.pin.localeCompare(b.pin)
    || a.part.localeCompare(b.part)
    || a.peripheral.localeCompare(b.peripheral),
  );

  return { connected: unique.length > 0, channels: unique };
}
