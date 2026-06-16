export interface LabWiredDiagramV1 {
  version: 1;
  board: string;
  parts: LabWiredDiagramPartV1[];
  wires: LabWiredDiagramWireV1[];
}

export interface LabWiredDiagramPartV1 {
  id: string;
  type: string;
  x: number;
  y: number;
  rotate: number;
  scale?: number;
  attrs: Record<string, string>;
}

export interface LabWiredDiagramWireEndpointV1 {
  part: string;
  pin: string;
}

export interface LabWiredDiagramWireV1 {
  from: LabWiredDiagramWireEndpointV1;
  to: LabWiredDiagramWireEndpointV1;
  color: string;
  waypoints?: { x: number; y: number }[];
}

export const LABWIRED_DIAGRAM_V1_SCHEMA = {
  $id: 'https://labwired.com/schemas/diagram-v1.json',
  $schema: 'https://json-schema.org/draft/2020-12/schema',
  type: 'object',
  additionalProperties: true,
  required: ['version', 'board', 'parts', 'wires'],
  properties: {
    version: { const: 1 },
    board: { type: 'string', minLength: 1 },
    parts: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: true,
        required: ['id', 'type', 'x', 'y', 'rotate', 'attrs'],
        properties: {
          id: { type: 'string', minLength: 1 },
          type: { type: 'string', minLength: 1 },
          x: { type: 'number' },
          y: { type: 'number' },
          rotate: { type: 'number' },
          scale: { type: 'number' },
          attrs: {
            type: 'object',
            additionalProperties: { type: 'string' },
          },
        },
      },
    },
    wires: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: true,
        required: ['from', 'to', 'color'],
        properties: {
          from: { $ref: '#/$defs/wireEndpoint' },
          to: { $ref: '#/$defs/wireEndpoint' },
          color: { type: 'string', minLength: 1 },
          waypoints: {
            type: 'array',
            items: {
              type: 'object',
              required: ['x', 'y'],
              properties: {
                x: { type: 'number' },
                y: { type: 'number' },
              },
            },
          },
        },
      },
    },
  },
  $defs: {
    wireEndpoint: {
      type: 'object',
      required: ['part', 'pin'],
      properties: {
        part: { type: 'string', minLength: 1 },
        pin: { type: 'string', minLength: 1 },
      },
    },
  },
} as const;

const FALLBACK_WIRE_COLORS = ['#e83e8c', '#27c93f', '#569cd6', '#ffcc00', '#ff6633', '#00cccc'];

const DEFAULT_ATTRS_BY_TYPE: Record<string, Record<string, string>> = {
  led: { color: 'red' },
  'seven-segment': { color: 'red' },
  'led-matrix': { color: 'red' },
  resistor: { value: '220' },
  capacitor: { value: '100nF' },
  diode: { type: '1N4148' },
  transistor: { type: 'NPN', part: '2N2222' },
  potentiometer: { value: '10K' },
  ldr: { value: '10K' },
  'ntc-thermistor': { beta: '3950', r0: '10K' },
  'slide-switch': { position: 'left' },
  dht22: { temperature: '25', humidity: '50' },
  servo: { angle: '90' },
  ultrasonic: { distance: '100' },
  lcd1602: { text: 'Hello World!' },
  neopixel: { count: '8' },
  'logic-analyzer': { decoder: 'raw' },
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === 'object' && !Array.isArray(value);
}

function finiteNumber(value: unknown, fallback: number): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : fallback;
}

function stringValue(value: unknown): string | undefined {
  if (typeof value === 'string') return value;
  if (typeof value === 'number' || typeof value === 'boolean') return String(value);
  return undefined;
}

function normalizeAttrs(part: Record<string, unknown>, type: string): Record<string, string> {
  const defaults = DEFAULT_ATTRS_BY_TYPE[type] ?? {};
  const attrs: Record<string, string> = { ...defaults };

  for (const key of Object.keys(defaults)) {
    const value = stringValue(part[key]);
    if (value !== undefined) attrs[key] = value;
  }

  if (isRecord(part.attrs)) {
    for (const [key, value] of Object.entries(part.attrs)) {
      const attrValue = stringValue(value);
      if (attrValue !== undefined) attrs[key] = attrValue;
    }
  }

  return attrs;
}

function normalizePart(value: unknown, index: number): LabWiredDiagramPartV1 | null {
  if (!isRecord(value)) return null;
  const type = typeof value.type === 'string' && value.type ? value.type : '';
  if (!type) return null;

  const part: LabWiredDiagramPartV1 = {
    id: typeof value.id === 'string' && value.id ? value.id : `part${index + 1}`,
    type,
    x: finiteNumber(value.x, 140 + (index % 4) * 150),
    y: finiteNumber(value.y, 140 + Math.floor(index / 4) * 130),
    rotate: finiteNumber(value.rotate, 0),
    attrs: normalizeAttrs(value, type),
  };

  if (typeof value.scale === 'number' && Number.isFinite(value.scale)) {
    part.scale = value.scale;
  }

  return part;
}

function normalizeEndpoint(value: unknown): LabWiredDiagramWireEndpointV1 | null {
  if (!isRecord(value)) return null;
  if (typeof value.part !== 'string' || !value.part) return null;
  if (typeof value.pin !== 'string' || !value.pin) return null;
  return { part: value.part, pin: value.pin };
}

function normalizeWaypoints(value: unknown): { x: number; y: number }[] | undefined {
  if (!Array.isArray(value)) return undefined;
  const waypoints = value
    .map((point) => {
      if (!isRecord(point)) return null;
      const x = finiteNumber(point.x, NaN);
      const y = finiteNumber(point.y, NaN);
      return Number.isFinite(x) && Number.isFinite(y) ? { x, y } : null;
    })
    .filter((point): point is { x: number; y: number } => !!point);
  return waypoints.length ? waypoints : undefined;
}

function normalizeWire(value: unknown, index: number): LabWiredDiagramWireV1 | null {
  if (!isRecord(value)) return null;
  const from = normalizeEndpoint(value.from);
  const to = normalizeEndpoint(value.to);
  if (!from || !to) return null;

  const wire: LabWiredDiagramWireV1 = {
    from,
    to,
    color: typeof value.color === 'string' && value.color
      ? value.color
      : FALLBACK_WIRE_COLORS[index % FALLBACK_WIRE_COLORS.length],
  };
  const waypoints = normalizeWaypoints(value.waypoints);
  if (waypoints) wire.waypoints = waypoints;
  return wire;
}

export function normalizeLabWiredDiagramV1(value: unknown): LabWiredDiagramV1 | null {
  if (!isRecord(value)) return null;
  const parts = Array.isArray(value.parts)
    ? value.parts.map(normalizePart).filter((part): part is LabWiredDiagramPartV1 => !!part)
    : [];
  const wires = Array.isArray(value.wires)
    ? value.wires.map(normalizeWire).filter((wire): wire is LabWiredDiagramWireV1 => !!wire)
    : [];

  return {
    version: 1,
    board: typeof value.board === 'string' && value.board ? value.board : 'stm32f103',
    parts,
    wires,
  };
}
