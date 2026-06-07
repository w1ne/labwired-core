// LabWired API — anonymous product usage tracking.
//
// Events land in a Cloudflare Analytics Engine dataset (binding `USAGE`).
// Two sources share one schema:
//   * `api` — server-side MCP tool calls, recorded in callHostedTool.
//   * `web` — playground beacons via POST /v1/events (allowlisted names only).
//
// Data point shape (query with the Analytics Engine SQL API):
//   blobs:   [event, tool, board, status, source]
//   doubles: [duration_ms]
//   indexes: [event]
//
// No PII: no user ids, no IPs, no payload contents. The binding is optional —
// local dev and tests run without it and every path here is a silent no-op.

import type { Env } from './types.js';

const MAX_FIELD_LEN = 64;

export interface UsageEvent {
  event: string;
  tool?: string;
  board?: string;
  status?: 'ok' | 'error';
  durationMs?: number;
  source?: 'api' | 'web';
}

/** Web beacon events the playground may report. Anything else is rejected. */
export const WEB_EVENT_ALLOWLIST: ReadonlySet<string> = new Set([
  'app_loaded',
  'board_selected',
  'run_clicked',
  'lab_opened',
  'diagram_shared',
]);

function clip(value: string | undefined): string {
  return (value ?? '').slice(0, MAX_FIELD_LEN);
}

/** Record one usage event. Never throws; no-op without the USAGE binding. */
export function trackUsage(env: Env, e: UsageEvent): void {
  try {
    env.USAGE?.writeDataPoint({
      blobs: [clip(e.event), clip(e.tool), clip(e.board), clip(e.status), e.source ?? 'api'],
      doubles: [e.durationMs ?? 0],
      indexes: [clip(e.event)],
    });
  } catch {
    // Usage tracking must never affect the request path.
  }
}

/**
 * POST /v1/events — anonymous beacon endpoint for the playground.
 * Always answers 204 on accepted events (even with no binding) so the client
 * fire-and-forget path never sees an error; 400 only for malformed/unknown
 * input so bugs in our own instrumentation surface in dev.
 */
export async function handleTrackEvent(request: Request, env: Env): Promise<Response> {
  // Browser beacons are cross-origin (labwired.com → api.labwired.com).
  const headers = { 'Access-Control-Allow-Origin': '*' };

  type EventBody = { event?: unknown; board?: unknown; tool?: unknown };
  let body: EventBody | null = null;
  try {
    body = (await request.json()) as EventBody;
  } catch {
    return new Response(null, { status: 400, headers });
  }

  const event = typeof body?.event === 'string' ? body.event : '';
  if (!WEB_EVENT_ALLOWLIST.has(event)) {
    return new Response(null, { status: 400, headers });
  }

  trackUsage(env, {
    event,
    board: typeof body?.board === 'string' ? body.board : undefined,
    tool: typeof body?.tool === 'string' ? body.tool : undefined,
    source: 'web',
  });
  return new Response(null, { status: 204, headers });
}
