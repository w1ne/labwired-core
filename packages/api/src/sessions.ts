/**
 * Public HTTP handlers for /v1/sessions/*. Wraps SessionDO instances.
 *
 *   POST   /v1/sessions               create + return { session_id, owner_token, watch_url }
 *   GET    /v1/sessions/:id           current public state
 *   PUT    /v1/sessions/:id           owner write (Authorization: Bearer <owner_token>)
 *   DELETE /v1/sessions/:id           owner destroy
 *   GET    /v1/sessions/:id/ws        WebSocket subscribe (watcher, no auth)
 *
 * Anonymous by default; optional Clerk-attach by passing a Clerk Bearer JWT
 * at create time (we record clerk_user_id but auth-for-writes still requires
 * the owner_token for v0.3).
 */
import type { Env } from './types.js';
import { verifyClerkRequest } from './clerk.js';

const CORS_HEADERS: Record<string, string> = {
  'Access-Control-Allow-Origin': '*',
  'Access-Control-Allow-Methods': 'GET, POST, PUT, DELETE, OPTIONS',
  'Access-Control-Allow-Headers': 'Content-Type, Authorization',
};

function ok(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json', ...CORS_HEADERS },
  });
}
function err(message: string, status = 400): Response {
  return ok({ error: message }, status);
}

function randomSessionId(): string {
  const buf = new Uint8Array(12);
  crypto.getRandomValues(buf);
  // base36-ish, URL-safe enough for query params.
  return Array.from(buf, (b) => b.toString(36).padStart(2, '0')).join('');
}

function watchUrl(env: Env, sessionId: string): string {
  // Default to app.labwired.com — this is the playground origin a human
  // would open. Override per env if needed in the future.
  return `https://app.labwired.com/?watch=${sessionId}`;
  void env;
}

/** POST /v1/sessions — create a new session. */
export async function handleCreateSession(request: Request, env: Env): Promise<Response> {
  // Optional Clerk attach: if a Bearer JWT is present and valid, record the user.
  let clerk_user_id: string | undefined;
  const auth = request.headers.get('Authorization');
  if (auth) {
    const v = await verifyClerkRequest(request, env);
    if (v) clerk_user_id = v.userId;
    // If Clerk verification fails, fall through to anonymous — we don't gate creation.
  }

  const session_id = randomSessionId();
  const stub = env.SESSIONS.get(env.SESSIONS.idFromName(session_id));
  const initResp = await stub.fetch('https://session/__init', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ session_id, clerk_user_id }),
  });
  if (!initResp.ok) {
    const body = await initResp.text();
    return err(`failed to init session: ${body}`, 500);
  }
  const { owner_token } = (await initResp.json()) as { owner_token: string };
  return ok({
    session_id,
    owner_token,
    watch_url: watchUrl(env, session_id),
    clerk_attached: !!clerk_user_id,
  });
}

/** Forward a request to a SessionDO instance, preserving method + headers + body. */
async function forwardToSession(env: Env, sessionId: string, path: string, request: Request): Promise<Response> {
  const stub = env.SESSIONS.get(env.SESSIONS.idFromName(sessionId));
  // Build a synthetic URL so the DO can dispatch on path.
  const inner = new Request(`https://session/sessions/${sessionId}${path}`, {
    method: request.method,
    headers: request.headers,
    body: request.method === 'GET' || request.method === 'DELETE' ? null : request.body,
  });
  const resp = await stub.fetch(inner);
  // Pipe response through with CORS.
  const headers = new Headers(resp.headers);
  for (const [k, v] of Object.entries(CORS_HEADERS)) headers.set(k, v);
  return new Response(resp.body, { status: resp.status, headers });
}

/** GET /v1/sessions/:id — public state. */
export async function handleGetSession(request: Request, env: Env, sessionId: string): Promise<Response> {
  return forwardToSession(env, sessionId, '', request);
}

/** PUT /v1/sessions/:id — owner write. */
export async function handleUpdateSession(request: Request, env: Env, sessionId: string): Promise<Response> {
  return forwardToSession(env, sessionId, '/state', request);
}

/** DELETE /v1/sessions/:id — owner destroy. */
export async function handleDeleteSession(request: Request, env: Env, sessionId: string): Promise<Response> {
  return forwardToSession(env, sessionId, '', request);
}

/** GET /v1/sessions/:id/ws — watcher WebSocket. */
export async function handleSessionWebSocket(request: Request, env: Env, sessionId: string): Promise<Response> {
  if (request.headers.get('Upgrade') !== 'websocket') {
    return err('Expected WebSocket upgrade', 426);
  }
  const stub = env.SESSIONS.get(env.SESSIONS.idFromName(sessionId));
  const inner = new Request(`https://session/sessions/${sessionId}/ws`, {
    method: 'GET',
    headers: request.headers,
  });
  return stub.fetch(inner);
}
