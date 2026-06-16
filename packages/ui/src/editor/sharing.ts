import { normalizeLabWiredDiagramV1 } from '@labwired/board-config';
import type { Diagram } from './types';

const API_BASE = 'https://api.labwired.com';

export function normalizeSharedDiagram(value: unknown): Diagram | null {
  return normalizeLabWiredDiagramV1(value) as Diagram | null;
}

function shareApiBase(): string {
  const envBase = (import.meta as unknown as { env?: { VITE_LABWIRED_API_BASE?: string } }).env?.VITE_LABWIRED_API_BASE;
  return envBase || API_BASE;
}

export async function fetchSharedProject(shareId: string): Promise<{ diagram: Diagram; source: string } | null> {
  if (!/^[A-Za-z0-9_-]+$/.test(shareId)) return null;
  try {
    const resp = await fetch(`${shareApiBase()}/v1/shares/${encodeURIComponent(shareId)}`);
    if (!resp.ok) return null;
    const obj = await resp.json() as { diagram?: unknown; source?: unknown };
    const diagram = normalizeSharedDiagram(obj.diagram);
    if (!diagram) return null;
    return { diagram, source: typeof obj.source === 'string' ? obj.source : '' };
  } catch {
    return null;
  }
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
  try {
    const resp = await fetch(`${shareApiBase()}/v1/shares`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ diagram, source }),
    });
    if (resp.ok) {
      const body = await resp.json() as { url?: unknown };
      if (typeof body.url === 'string') return body.url;
    }
  } catch {
    // Fall back to self-contained hash links when the API is unavailable.
  }
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
  try {
    const resp = await fetch(`${shareApiBase()}/v1/shares`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ diagram, source }),
    });
    if (resp.ok) {
      const body = await resp.json() as { embed_url?: unknown };
      if (typeof body.embed_url === 'string') return body.embed_url;
    }
  } catch {
    // Fall back to self-contained hash links when the API is unavailable.
  }
  const encoded = await encodeProject(diagram, source);
  const url = new URL(window.location.href);
  url.hash = encoded;
  url.searchParams.set('embed', 'true');
  return url.toString();
}
