// Clerk JWT verification for Cloudflare Workers — networkless (no Clerk API fetch).
import { createClerkClient } from '@clerk/backend';
import type { Env } from './types.js';

export interface VerifiedClerk {
  userId: string;
  sessionId: string;
  claims: Record<string, unknown>;
}

function clerkAuthorizationServer(env: Env): string {
  return env.MCP_AUTHORIZATION_SERVER ?? 'https://clerk.labwired.com';
}

export async function verifyClerkRequest(
  request: Request,
  env: Env,
): Promise<VerifiedClerk | null> {
  if (!env.CLERK_SECRET_KEY) return null;

  const client = createClerkClient({
    secretKey: env.CLERK_SECRET_KEY,
    publishableKey: env.CLERK_PUBLISHABLE_KEY || undefined,
    jwtKey: env.CLERK_JWT_KEY || undefined,
  });

  let state;
  try {
    state = await client.authenticateRequest(request, {
      jwtKey: env.CLERK_JWT_KEY || undefined,
      authorizedParties: ['https://app.labwired.com', 'https://labwired.com'],
    });
  } catch {
    return null;
  }

  if (!state.isAuthenticated) return null;

  const auth = state.toAuth();
  if (!auth || !auth.userId || !auth.sessionId) return null;

  return {
    userId: auth.userId,
    sessionId: auth.sessionId,
    claims: (auth.sessionClaims ?? {}) as Record<string, unknown>,
  };
}

export async function verifyClerkOAuthAccessToken(
  token: string,
  env: Env,
): Promise<VerifiedClerk | null> {
  if (!token) return null;

  let response: Response;
  try {
    response = await fetch(`${clerkAuthorizationServer(env)}/oauth/userinfo`, {
      headers: { Authorization: `Bearer ${token}` },
    });
  } catch {
    return null;
  }

  if (!response.ok) return null;

  let claims: Record<string, unknown>;
  try {
    claims = (await response.json()) as Record<string, unknown>;
  } catch {
    return null;
  }

  const subject = claims.sub;
  if (typeof subject !== 'string' || !subject) return null;

  return {
    userId: subject,
    sessionId: 'oauth',
    claims,
  };
}
