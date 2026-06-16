import { normalizeLabWiredDiagramV1 } from '@labwired/board-config';
import type { Env } from './types.js';

const SHARE_ID_BYTES = 9;
const SHARE_TTL_SECONDS = 60 * 60 * 24 * 90;
const SOURCE_MAX = 1024 * 1024;

export interface ShareRecord {
  id: string;
  diagram: unknown;
  source: string;
  created_at: number;
}

const CORS_HEADERS: Record<string, string> = {
  'Access-Control-Allow-Origin': '*',
  'Access-Control-Allow-Methods': 'GET, POST, OPTIONS',
  'Access-Control-Allow-Headers': 'Content-Type',
};

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json', ...CORS_HEADERS },
  });
}

function err(message: string, status = 400): Response {
  return json({ error: message }, status);
}

function shareKey(id: string): string {
  return `share:${id}`;
}

function randomShareId(): string {
  const bytes = new Uint8Array(SHARE_ID_BYTES);
  crypto.getRandomValues(bytes);
  const binary = Array.from(bytes, (b) => String.fromCharCode(b)).join('');
  return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/g, '');
}

function sourceFrom(value: unknown): string {
  return typeof value === 'string' ? value : '';
}

export async function createShareRecord(
  env: Env,
  input: { diagram: unknown; source?: unknown },
): Promise<ShareRecord> {
  const diagram = normalizeLabWiredDiagramV1(input.diagram);
  if (!diagram) throw new Error('diagram is required');

  const source = sourceFrom(input.source);
  if (source.length > SOURCE_MAX) throw new Error(`source exceeds ${SOURCE_MAX} bytes`);

  const record: ShareRecord = {
    id: randomShareId(),
    diagram,
    source,
    created_at: Date.now(),
  };
  await env.KV_PROJECTS.put(shareKey(record.id), JSON.stringify(record), { expirationTtl: SHARE_TTL_SECONDS });
  return record;
}

export function shareUrls(id: string): { studioUrl: string; embedUrl: string } {
  return {
    studioUrl: `https://app.labwired.com/?share=${encodeURIComponent(id)}`,
    embedUrl: `https://app.labwired.com/?embed=true&share=${encodeURIComponent(id)}`,
  };
}

export async function handleCreateShare(request: Request, env: Env): Promise<Response> {
  let body: Record<string, unknown>;
  try {
    body = (await request.json()) as Record<string, unknown>;
  } catch {
    return err('Invalid JSON body');
  }

  try {
    const record = await createShareRecord(env, {
      diagram: body.diagram,
      source: body.source ?? body.source_code,
    });
    const urls = shareUrls(record.id);
    return json({ id: record.id, url: urls.studioUrl, embed_url: urls.embedUrl }, 201);
  } catch (error) {
    return err(error instanceof Error ? error.message : String(error));
  }
}

export async function handleGetShare(_request: Request, env: Env, shareId: string): Promise<Response> {
  const raw = await env.KV_PROJECTS.get(shareKey(shareId));
  if (!raw) return err('Share not found', 404);

  try {
    const record = JSON.parse(raw) as ShareRecord;
    return json({ id: record.id, diagram: record.diagram, source: record.source, created_at: record.created_at });
  } catch {
    return err('Stored share is corrupt', 500);
  }
}
