// GitHub OAuth + session management
import type { Env, SessionRecord } from './types.js';

const STATE_TTL_SECONDS = 600;
const SESSION_TTL_SECONDS = 60 * 60 * 24 * 30;

function randomHex(bytes: number): string {
  const buf = new Uint8Array(bytes);
  crypto.getRandomValues(buf);
  return Array.from(buf)
    .map((b) => b.toString(16).padStart(2, '0'))
    .join('');
}

function stateKey(state: string): string {
  return `oauth_state:${state}`;
}

function readBearerSession(request: Request): string | null {
  const header = request.headers.get('Authorization') ?? '';
  if (!header.startsWith('Bearer ')) return null;
  return header.slice(7).trim() || null;
}

export async function getSession(env: Env, token: string): Promise<SessionRecord | null> {
  try {
    const raw = await env.KV_SESSIONS.get(token);
    if (!raw) return null;
    return JSON.parse(raw) as SessionRecord;
  } catch {
    return null;
  }
}

export async function handleGithubStart(request: Request, env: Env): Promise<Response> {
  const clientId = env.GITHUB_CLIENT_ID;
  if (!clientId) {
    return new Response(JSON.stringify({ error: 'GitHub OAuth not configured' }), {
      status: 500,
      headers: { 'Content-Type': 'application/json' },
    });
  }

  const state = randomHex(16);
  await env.KV_SESSIONS.put(stateKey(state), '1', { expirationTtl: STATE_TTL_SECONDS });

  const url = new URL(request.url);
  const redirectUri = `${url.origin}/v1/auth/github/callback`;

  const authorize = new URL('https://github.com/login/oauth/authorize');
  authorize.searchParams.set('client_id', clientId);
  authorize.searchParams.set('scope', 'read:user');
  authorize.searchParams.set('state', state);
  authorize.searchParams.set('redirect_uri', redirectUri);

  return Response.redirect(authorize.toString(), 302);
}

interface GithubTokenResponse {
  access_token?: string;
  error?: string;
  error_description?: string;
}

interface GithubUserResponse {
  id: number;
  login: string;
  avatar_url: string;
  email: string | null;
}

export async function handleGithubCallback(request: Request, env: Env): Promise<Response> {
  const url = new URL(request.url);
  const code = url.searchParams.get('code');
  const state = url.searchParams.get('state');
  const playgroundOrigin = env.PLAYGROUND_ORIGIN || 'https://foundry.labwired.com';

  if (!code || !state) {
    return Response.redirect(`${playgroundOrigin}/?auth_error=missing_params`, 302);
  }

  const stored = await env.KV_SESSIONS.get(stateKey(state));
  if (!stored) {
    return Response.redirect(`${playgroundOrigin}/?auth_error=invalid_state`, 302);
  }
  await env.KV_SESSIONS.delete(stateKey(state));

  const redirectUri = `${url.origin}/v1/auth/github/callback`;

  const tokenResp = await fetch('https://github.com/login/oauth/access_token', {
    method: 'POST',
    headers: {
      Accept: 'application/json',
      'Content-Type': 'application/json',
      'User-Agent': 'labwired-api',
    },
    body: JSON.stringify({
      client_id: env.GITHUB_CLIENT_ID,
      client_secret: env.GITHUB_CLIENT_SECRET,
      code,
      redirect_uri: redirectUri,
    }),
  });

  if (!tokenResp.ok) {
    return Response.redirect(`${playgroundOrigin}/?auth_error=token_exchange_failed`, 302);
  }

  const tokenJson = (await tokenResp.json()) as GithubTokenResponse;
  const accessToken = tokenJson.access_token;
  if (!accessToken) {
    return Response.redirect(`${playgroundOrigin}/?auth_error=no_access_token`, 302);
  }

  const userResp = await fetch('https://api.github.com/user', {
    headers: {
      Authorization: `Bearer ${accessToken}`,
      Accept: 'application/vnd.github+json',
      'User-Agent': 'labwired-api',
    },
  });

  if (!userResp.ok) {
    return Response.redirect(`${playgroundOrigin}/?auth_error=user_fetch_failed`, 302);
  }

  const user = (await userResp.json()) as GithubUserResponse;

  const sessionToken = randomHex(32);
  const record: SessionRecord = {
    github_id: user.id,
    login: user.login,
    avatar_url: user.avatar_url,
    email: user.email ?? null,
    created_at: new Date().toISOString(),
  };

  await env.KV_SESSIONS.put(sessionToken, JSON.stringify(record), {
    expirationTtl: SESSION_TTL_SECONDS,
  });

  return Response.redirect(`${playgroundOrigin}/#session=${sessionToken}`, 302);
}

const CORS_HEADERS: Record<string, string> = {
  'Access-Control-Allow-Origin': '*',
  'Access-Control-Allow-Methods': 'GET, POST, OPTIONS',
  'Access-Control-Allow-Headers': 'Content-Type, Authorization',
};

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json', ...CORS_HEADERS },
  });
}

export async function handleAuthMe(request: Request, env: Env): Promise<Response> {
  const token = readBearerSession(request);
  if (!token) return jsonResponse({ error: 'Missing session token' }, 401);

  const session = await getSession(env, token);
  if (!session) return jsonResponse({ error: 'Invalid or expired session' }, 401);

  // Plan lookup: GitHub-only sign-ins are 'free' for now. Stripe-paid users
  // continue to use the API-key flow; no github_id↔workspace mapping yet.
  return jsonResponse({
    github_id: session.github_id,
    login: session.login,
    avatar_url: session.avatar_url,
    email: session.email,
    plan: 'free',
  });
}

export async function handleAuthLogout(request: Request, env: Env): Promise<Response> {
  const token = readBearerSession(request);
  if (!token) return jsonResponse({ ok: true });
  try {
    await env.KV_SESSIONS.delete(token);
  } catch {
    // best-effort
  }
  return jsonResponse({ ok: true });
}
