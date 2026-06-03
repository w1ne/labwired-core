import { getPinMapping, type Diagram, type WireEndpoint } from '@labwired/ui';
import { getIolinkDecoderBinding, getLogicAnalyzerChannelBindings } from './logicAnalyzerConnections';

export type LogicLevel = 0 | 1;

export interface LogicAnalyzerChannelSample {
  channel: string;
  value: LogicLevel | null;
  source: string | null;
}

export interface LogicAnalyzerSample {
  t: number;
  channels: LogicAnalyzerChannelSample[];
}

export interface DecoderAvailability {
  raw: true;
  iolink: boolean;
  uart: boolean;
  spi: boolean;
}

export interface CaptureLogicAnalyzerSampleOptions {
  diagram: Diagram;
  analyzerId: string;
  nowMs: number;
  getPeripheralSnapshot: (name: string) => unknown;
}

const CHANNELS = ['CH0', 'CH1', 'CH2', 'CH3'];

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

function numberField(snapshot: Record<string, unknown>, key: string): number {
  const value = snapshot[key];
  return typeof value === 'number' && Number.isFinite(value) ? value : 0;
}

function readBit(value: number, pin: number): LogicLevel {
  return (value & (1 << pin)) !== 0 ? 1 : 0;
}

function isStm32F1Output(snapshot: Record<string, unknown>, pin: number): boolean {
  const cr = numberField(snapshot, pin < 8 ? 'crl' : 'crh');
  const shift = (pin % 8) * 4;
  const mode = (cr >> shift) & 0b11;
  return mode !== 0;
}

function isStm32V2Output(snapshot: Record<string, unknown>, pin: number): boolean {
  const moder = numberField(snapshot, 'moder');
  const mode = (moder >> (pin * 2)) & 0b11;
  return mode === 0b01 || mode === 0b10;
}

export function readGpioSnapshotPin(snapshot: unknown, pin: number): LogicLevel | null {
  if (!isRecord(snapshot) || pin < 0 || pin > 31) return null;

  const odr = numberField(snapshot, 'odr');
  const idr = numberField(snapshot, 'idr');

  if ('dir' in snapshot) {
    const dir = numberField(snapshot, 'dir');
    return readBit((dir & (1 << pin)) !== 0 ? odr : idr, pin);
  }

  if ('moder' in snapshot) {
    return readBit(isStm32V2Output(snapshot, pin) ? odr : idr, pin);
  }

  if ('crl' in snapshot || 'crh' in snapshot) {
    return readBit(isStm32F1Output(snapshot, pin) ? odr : idr, pin);
  }

  return readBit(odr || idr, pin);
}

function resolveMcuEndpoint(endpoints: WireEndpoint[]): WireEndpoint | null {
  return endpoints.find((endpoint) => endpoint.part === 'mcu') ?? null;
}

function readEndpointLevel(
  diagram: Diagram,
  endpoints: WireEndpoint[],
  getPeripheralSnapshot: (name: string) => unknown,
): { value: LogicLevel | null; source: string | null } {
  const mcuEndpoint = resolveMcuEndpoint(endpoints);
  if (!mcuEndpoint) return { value: null, source: null };

  const pin = getPinMapping(diagram.board, mcuEndpoint.pin);
  if (!pin) return { value: null, source: mcuEndpoint.pin };

  const { peripheral, pin: pinNumber } = pin.gpio;
  const value = readGpioSnapshotPin(getPeripheralSnapshot(peripheral), pinNumber);
  return { value, source: `${peripheral}.${pinNumber}` };
}

export function captureLogicAnalyzerSample({
  diagram,
  analyzerId,
  nowMs,
  getPeripheralSnapshot,
}: CaptureLogicAnalyzerSampleOptions): LogicAnalyzerSample {
  const bindingsByChannel = new Map(
    getLogicAnalyzerChannelBindings(diagram, analyzerId).map((binding) => [binding.channel, binding.endpoints]),
  );

  return {
    t: nowMs,
    channels: CHANNELS.map((channel) => {
      const endpoints = bindingsByChannel.get(channel) ?? [];
      const { value, source } = readEndpointLevel(diagram, endpoints, getPeripheralSnapshot);
      return { channel, value, source };
    }),
  };
}

export function getDecoderAvailability(diagram: Diagram, analyzerId: string): DecoderAvailability {
  return {
    raw: true,
    iolink: getIolinkDecoderBinding(diagram, analyzerId).connected,
    uart: false,
    spi: false,
  };
}
