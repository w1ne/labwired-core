import { normalizeLabWiredDiagramV1, composeDiagnostics } from '@labwired/board-config';
import type { ValidateDiagram } from '@labwired/board-config';
import type { Env } from './types.js';

const SHARE_ID_BYTES = 9;
const SHARE_TTL_SECONDS = 60 * 60 * 24 * 90;
const SOURCE_MAX = 1024 * 1024;
const FIRMWARE_MAX = 12 * 1024 * 1024; // base64 ELF; demo ELFs are well under this

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

/** Thrown by createShareRecord when the diagram fails validation. Carries the
 *  full diagnostics so the HTTP layer can return them to the caller. */
export class ShareValidationError extends Error {
  constructor(public readonly validation: ReturnType<typeof composeDiagnostics>) {
    super('DIAGRAM_INVALID');
    this.name = 'ShareValidationError';
  }
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
  input: { diagram: unknown; source?: unknown; firmware?: unknown; target?: unknown },
): Promise<ShareRecord> {
  const diagram = normalizeLabWiredDiagramV1(input.diagram);
  if (!diagram) throw new Error('diagram is required');

  // Storage-boundary validation gate: every share — whether created via the MCP
  // open_hardware_lab tool or the raw POST /v1/shares route — must pass
  // validation. This is the last line of defense against shipping a board that
  // "looks wired but isn't" (hallucinated/unresolvable pins). No invalid diagram
  // is ever persisted.
  const validation = composeDiagnostics(diagram as unknown as ValidateDiagram);
  if (!validation.ok) {
    throw new ShareValidationError(validation);
  }

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
      firmware: body.firmware,
      target: body.target,
    });
    const urls = shareUrls(record.id);
    return json({ id: record.id, url: urls.studioUrl, embed_url: urls.embedUrl }, 201);
  } catch (error) {
    if (error instanceof ShareValidationError) {
      return json({ error: 'DIAGRAM_INVALID', detail: 'The diagram has wiring errors and was not shared. Fix the errors below and retry.', validation: error.validation }, 422);
    }
    return err(error instanceof Error ? error.message : String(error));
  }
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
