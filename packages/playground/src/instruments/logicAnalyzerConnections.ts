import type { Diagram, WireEndpoint } from '@labwired/ui';

export interface LogicAnalyzerChannelBinding {
  channel: string;
  endpoints: WireEndpoint[];
}

export interface IolinkDecoderBinding {
  connected: boolean;
  channels: { channel: string; pin: 'TX' | 'RX' }[];
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
