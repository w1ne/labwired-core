import { verifyClerkRequest } from '../clerk.js';
import { getWorkspaceIdByClerkUserId, getKeyRecord, touchKeyLastUsed } from '../keys.js';
import type { Env } from '../types.js';
import type { HostedMcpIdentity } from './types.js';
import { hostedMcpAuthenticateHeader } from './oauth.js';

// The 401 must carry the OAuth challenge (RFC 9728): realm +
// resource_metadata pointer. Clients read resource_metadata to start the browser
// login; realm alone leaves them with nowhere to go.
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

  // Workspace API key path. Lets agents/CI authenticate by pasting a key
  // (Authorization: Bearer lwk_live_…) instead of the interactive OAuth flow —
  // the same key the REST API accepts. The Clerk/OAuth path below stays the
  // default for humans in local MCP clients; this is purely additive.
  if (token.startsWith('lwk_live_')) {
    const record = await getKeyRecord(env, token);
    if (!record || record.status !== 'active') return unauthorized(request);
    await touchKeyLastUsed(env, token);
    return { userId: `key:${record.workspace_id}`, workspaceId: record.workspace_id };
  }

  const clerk = await verifyClerkRequest(request, env);
  if (!clerk) return unauthorized(request);

  const workspaceId = await getWorkspaceIdByClerkUserId(env, clerk.userId);
  return { userId: clerk.userId, workspaceId: workspaceId ?? undefined };
}
