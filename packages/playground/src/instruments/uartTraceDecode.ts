import type { UartTraceSnapshot } from '@labwired/ui';
import type { UartDecoderBinding } from './logicAnalyzerConnections';

export interface UartTraceRow {
  key: string;
  seq: number;
  channel: string;
  peripheral: string;
  direction: 'tx' | 'rx';
  byte: number;
}

export function rowsForUartTrace(snapshots: UartTraceSnapshot[], binding: UartDecoderBinding): UartTraceRow[] {
  const selected = new Map<string, string>();
  for (const channel of binding.channels) {
    selected.set(`${channel.peripheral}:${channel.role}`, channel.channel);
  }

  return snapshots.flatMap((snapshot) =>
    snapshot.events.flatMap((event) => {
      const channel = selected.get(`${snapshot.peripheral}:${event.direction}`);
      if (!channel) return [];
      return [
        {
          key: `${snapshot.peripheral}:${event.seq}:${event.direction}:${event.byte}`,
          seq: event.seq,
          channel,
          peripheral: snapshot.peripheral,
          direction: event.direction,
          byte: event.byte & 0xff,
        },
      ];
    }),
  );
}
