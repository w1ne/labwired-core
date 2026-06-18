import { normalizeLabWiredDiagramV1 } from '@labwired/board-config';
import type { Env } from './types.js';
import { verifyClerkRequest } from './clerk.js';

/** Brand logo served as the anonymous/no-image fallback for share cards. */
const FALLBACK_IMAGE_URL = 'https://app.labwired.com/icon-512.png';

const SHARE_ID_BYTES = 9;
const SHARE_TTL_SECONDS = 60 * 60 * 24 * 90;
const SOURCE_MAX = 1024 * 1024;
const FIRMWARE_MAX = 12 * 1024 * 1024; // base64 ELF; demo ELFs are well under this
const PREVIEW_MAX_BYTES = 512 * 1024; // decoded PNG size guard
// PNG signature: 89 50 4E 47 0D 0A 1A 0A
const PNG_MAGIC = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];

export interface ShareRecord {
  id: string;
  diagram: unknown;
  source: string;
  /** Base64 ELF carried with the share — one link format; if present, run it. */
  firmware?: string;
  /** MCU target id for the firmware (e.g. 'nrf52840', 'stm32l476'). */
  target?: string;
  created_at: number;
}

const CORS_HEADERS: Record<string, string> = {
  'Access-Control-Allow-Origin': '*',
  'Access-Control-Allow-Methods': 'GET, POST, OPTIONS',
  // `Authorization` so a signed-in user can attach a Clerk bearer token to the
  // share POST (gates per-lab preview-image storage).
  'Access-Control-Allow-Headers': 'Content-Type, Authorization',
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

function shareImageKey(id: string): string {
  return `shareimg:${id}`;
}

/**
 * Decode an optional `preview` field (data URL or raw base64 PNG) into PNG bytes.
 * Returns null — never throws — when absent, malformed, not a PNG, or oversized,
 * so a bad image never blocks share creation.
 */
function decodePreviewPng(value: unknown): Uint8Array | null {
  if (typeof value !== 'string' || !value) return null;

  // Strip an optional data-URL prefix (data:image/png;base64,…).
  const comma = value.indexOf(',');
  const base64 = value.startsWith('data:') && comma !== -1 ? value.slice(comma + 1) : value;
  if (!base64) return null;

  let binary: string;
  try {
    binary = atob(base64);
  } catch {
    return null;
  }

  if (binary.length < PNG_MAGIC.length || binary.length > PREVIEW_MAX_BYTES) return null;

  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);

  for (let i = 0; i < PNG_MAGIC.length; i++) {
    if (bytes[i] !== PNG_MAGIC[i]) return null;
  }
  return bytes;
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
  input: {
    diagram: unknown;
    source?: unknown;
    firmware?: unknown;
    target?: unknown;
    preview?: unknown;
  },
): Promise<ShareRecord> {
  const diagram = normalizeLabWiredDiagramV1(input.diagram);
  if (!diagram) throw new Error('diagram is required');

  const source = sourceFrom(input.source);
  if (source.length > SOURCE_MAX) throw new Error(`source exceeds ${SOURCE_MAX} bytes`);

  const firmware = typeof input.firmware === 'string' && input.firmware ? input.firmware : undefined;
  if (firmware && firmware.length > FIRMWARE_MAX) throw new Error(`firmware exceeds ${FIRMWARE_MAX} bytes`);
  const target = typeof input.target === 'string' && input.target ? input.target : undefined;

  const record: ShareRecord = {
    id: randomShareId(),
    diagram,
    source,
    ...(firmware ? { firmware } : {}),
    ...(target ? { target } : {}),
    created_at: Date.now(),
  };
  await env.KV_PROJECTS.put(shareKey(record.id), JSON.stringify(record), { expirationTtl: SHARE_TTL_SECONDS });

  // Optional social-preview PNG. Non-fatal: a missing/bad image never blocks the
  // share — we just skip storing it. One extra KV write when present.
  const preview = decodePreviewPng(input.preview);
  if (preview) {
    await env.KV_PROJECTS.put(shareImageKey(record.id), preview, {
      expirationTtl: SHARE_TTL_SECONDS,
      metadata: { contentType: 'image/png' },
    });
  }

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

  // Per-lab preview images are stored ONLY for authenticated (signed-in) shares.
  // Anonymous shares are still created — they just carry no image and fall back
  // to the brand logo. This auth-gates the abuse surface (hosting arbitrary
  // attacker-supplied bytes on our domain; unauthenticated storage/cost) without
  // removing anonymous sharing.
  const authed = await verifyClerkRequest(request, env);

  try {
    const record = await createShareRecord(env, {
      diagram: body.diagram,
      source: body.source ?? body.source_code,
      firmware: body.firmware,
      target: body.target,
      preview: authed ? body.preview : undefined,
    });
    const urls = shareUrls(record.id);
    return json({ id: record.id, url: urls.studioUrl, embed_url: urls.embedUrl }, 201);
  } catch (error) {
    return err(error instanceof Error ? error.message : String(error));
  }
}

export async function handleGetShareImage(
  _request: Request,
  env: Env,
  shareId: string,
): Promise<Response> {
  const bytes = await env.KV_PROJECTS.get(shareImageKey(shareId), 'arrayBuffer');

  // No image (anonymous share) or expired (images share the 90-day share TTL, so
  // old ones are auto-removed) → fall back to the brand logo. 302, NOT cached
  // immutably, so it isn't pinned forever.
  if (!bytes) {
    return new Response(null, {
      status: 302,
      headers: {
        Location: FALLBACK_IMAGE_URL,
        'Cache-Control': 'public, max-age=300',
        'Access-Control-Allow-Origin': '*',
      },
    });
  }

  return new Response(bytes, {
    status: 200,
    headers: {
      'Content-Type': 'image/png',
      // Defang PNG/HTML polyglots: force image interpretation, never sniff/execute.
      'X-Content-Type-Options': 'nosniff',
      'Content-Disposition': 'inline',
      'Cache-Control': 'public, max-age=31536000, immutable',
      'Access-Control-Allow-Origin': '*',
    },
  });
}

export async function handleGetShare(_request: Request, env: Env, shareId: string): Promise<Response> {
  const raw = await env.KV_PROJECTS.get(shareKey(shareId));
  if (!raw) return err('Share not found', 404);

  try {
    const record = JSON.parse(raw) as ShareRecord;
    return json({
      id: record.id,
      diagram: record.diagram,
      source: record.source,
      ...(record.firmware ? { firmware: record.firmware } : {}),
      ...(record.target ? { target: record.target } : {}),
      created_at: record.created_at,
    });
  } catch {
    return err('Stored share is corrupt', 500);
  }
}
