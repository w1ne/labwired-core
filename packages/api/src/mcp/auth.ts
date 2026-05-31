import { verifyClerkRequest } from '../clerk.js';
import { getWorkspaceIdByClerkUserId } from '../keys.js';
import type { Env } from '../types.js';
import type { HostedMcpIdentity } from './types.js';
import { hostedMcpAuthenticateHeader } from './oauth.js';

// The 401 must carry the full OAuth challenge (RFC 9728): realm +
// resource_metadata pointer + scope. Clients read resource_metadata to start
// the browser login; realm alone leaves them with nowhere to go.
export function unauthorized(request: Request): Response {
  return new Response(JSON.stringify({ error: 'Unauthorized' }), {
    status: 401,
    headers: {
      'Content-Type': 'application/json',
      'WWW-Authenticate': hostedMcpAuthenticateHeader(request),
      'Access-Control-Allow-Origin': '*',
    },
  });
}

export async function authenticateHostedMcpRequest(
  request: Request,
  env: Env,
): Promise<HostedMcpIdentity | Response> {
  const auth = request.headers.get('authorization') ?? '';
  const match = auth.match(/^Bearer\s+(.+)$/i);
  if (!match) return unauthorized(request);

  const token = match[1];
  if (env.ENVIRONMENT === 'test' && token.startsWith('test_user:')) {
    const [, userId, workspaceId] = token.split(':');
    if (!userId) return unauthorized(request);
    return { userId, workspaceId: workspaceId || undefined };
  }

  const clerk = await verifyClerkRequest(request, env);
  if (!clerk) return unauthorized(request);

  const workspaceId = await getWorkspaceIdByClerkUserId(env, clerk.userId);
  return { userId: clerk.userId, workspaceId: workspaceId ?? undefined };
}
