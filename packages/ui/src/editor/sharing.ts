import { COMPONENT_REGISTRY } from './components/index';
import type { Diagram, Part, Wire } from './types';

const FALLBACK_WIRE_COLORS = ['#e83e8c', '#27c93f', '#569cd6', '#ffcc00', '#ff6633', '#00cccc'];

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
  const def = COMPONENT_REGISTRY.get(type);
  const attrs: Record<string, string> = { ...(def?.defaultAttrs ?? {}) };

  for (const key of Object.keys(def?.defaultAttrs ?? {})) {
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

function normalizePart(value: unknown, index: number): Part | null {
  if (!isRecord(value)) return null;
  const type = typeof value.type === 'string' && value.type ? value.type : '';
  if (!type) return null;

  const part: Part = {
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

function normalizeEndpoint(value: unknown): { part: string; pin: string } | null {
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

function normalizeWire(value: unknown, index: number): Wire | null {
  if (!isRecord(value)) return null;
  const from = normalizeEndpoint(value.from);
  const to = normalizeEndpoint(value.to);
  if (!from || !to) return null;

  const wire: Wire = {
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

export function normalizeSharedDiagram(value: unknown): Diagram | null {
  if (!isRecord(value)) return null;
  const parts = Array.isArray(value.parts)
    ? value.parts.map(normalizePart).filter((part): part is Part => !!part)
    : [];
  const wires = Array.isArray(value.wires)
    ? value.wires.map(normalizeWire).filter((wire): wire is Wire => !!wire)
    : [];

  return {
    version: 1,
    board: typeof value.board === 'string' && value.board ? value.board : 'stm32f103',
    parts,
    wires,
  };
}

/**
 * Encode a diagram + source to a URL-safe base64 string.
 * Uses built-in compression via CompressionStream when available.
 */
export async function encodeProject(diagram: Diagram, source: string): Promise<string> {
  const payload = JSON.stringify({ d: diagram, s: source });
  const bytes = new TextEncoder().encode(payload);

  // Try CompressionStream (modern browsers)
  if (typeof CompressionStream !== 'undefined') {
    const cs = new CompressionStream('deflate');
    const writer = cs.writable.getWriter();
    writer.write(bytes);
    writer.close();
    const chunks: Uint8Array[] = [];
    const reader = cs.readable.getReader();
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      chunks.push(value);
    }
    const compressed = new Uint8Array(chunks.reduce((a, c) => a + c.length, 0));
    let offset = 0;
    for (const chunk of chunks) {
      compressed.set(chunk, offset);
      offset += chunk.length;
    }
    return 'z' + btoa(String.fromCharCode(...compressed));
  }

  // Fallback: raw base64
  return 'r' + btoa(payload);
}

/**
 * Decode a project from a URL hash string.
 */
export async function decodeProject(hash: string): Promise<{ diagram: Diagram; source: string } | null> {
  if (!hash || hash.length < 2) return null;

  try {
    const prefix = hash[0];
    const data = hash.slice(1);

    if (prefix === 'z' && typeof DecompressionStream !== 'undefined') {
      const compressed = Uint8Array.from(atob(data), (c) => c.charCodeAt(0));
      const ds = new DecompressionStream('deflate');
      const writer = ds.writable.getWriter();
      writer.write(compressed);
      writer.close();
      const chunks: Uint8Array[] = [];
      const reader = ds.readable.getReader();
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        chunks.push(value);
      }
      const decompressed = new Uint8Array(chunks.reduce((a, c) => a + c.length, 0));
      let offset = 0;
      for (const chunk of chunks) {
        decompressed.set(chunk, offset);
        offset += chunk.length;
      }
      const json = new TextDecoder().decode(decompressed);
      const obj = JSON.parse(json);
      const diagram = normalizeSharedDiagram(obj.d);
      if (!diagram) return null;
      return { diagram, source: obj.s ?? '' };
    }

    if (prefix === 'r') {
      const json = atob(data);
      const obj = JSON.parse(json);
      const diagram = normalizeSharedDiagram(obj.d);
      if (!diagram) return null;
      return { diagram, source: obj.s ?? '' };
    }
  } catch {
    // Invalid hash
  }
  return null;
}

/**
 * Check if the page is in embed mode (?embed=true).
 */
export function isEmbedMode(): boolean {
  return new URLSearchParams(window.location.search).get('embed') === 'true';
}

/**
 * Generate a shareable URL with the project encoded in the hash.
 */
export async function generateShareUrl(diagram: Diagram, source: string): Promise<string> {
  const encoded = await encodeProject(diagram, source);
  const url = new URL(window.location.href);
  url.hash = encoded;
  url.searchParams.delete('embed');
  return url.toString();
}

/**
 * Generate an embed URL.
 */
export async function generateEmbedUrl(diagram: Diagram, source: string): Promise<string> {
  const encoded = await encodeProject(diagram, source);
  const url = new URL(window.location.href);
  url.hash = encoded;
  url.searchParams.set('embed', 'true');
  return url.toString();
}
