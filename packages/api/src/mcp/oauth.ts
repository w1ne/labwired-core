import type { Env } from '../types.js';

export const HOSTED_MCP_OAUTH_SCOPES = ['openid', 'profile', 'email', 'offline_access'] as const;

function originFromRequest(request: Request): string {
  const url = new URL(request.url);
  return url.origin;
}

export function protectedResourceMetadataUrl(request: Request): string {
  return `${originFromRequest(request)}/.well-known/oauth-protected-resource/mcp`;
}

export function hostedMcpResourceUrl(request: Request): string {
  return `${originFromRequest(request)}/mcp`;
}

export function hostedMcpAuthenticateHeader(request: Request): string {
  return [
    'Bearer realm="LabWired MCP"',
    `resource_metadata="${protectedResourceMetadataUrl(request)}"`,
    `scope="${HOSTED_MCP_OAUTH_SCOPES.join(' ')}"`,
  ].join(', ');
}

export function handleMcpProtectedResourceMetadata(request: Request, env: Env): Response {
  if (!env.MCP_AUTHORIZATION_SERVER && env.ENVIRONMENT !== 'test') {
    return Response.json(
      {
        error: 'MCP_AUTHORIZATION_SERVER_MISSING',
        message: 'Set MCP_AUTHORIZATION_SERVER to the Clerk authorization server origin for hosted MCP OAuth discovery.',
      },
      {
        status: 500,
        headers: {
          'Access-Control-Allow-Origin': '*',
          'Access-Control-Allow-Methods': 'GET, OPTIONS',
          'Access-Control-Allow-Headers': 'Content-Type, Authorization, MCP-Protocol-Version',
        },
      },
    );
  }

  const body: {
    resource: string;
    resource_name: string;
    bearer_methods_supported: string[];
    resource_documentation: string;
    scopes_supported: string[];
    authorization_servers?: string[];
  } = {
    resource: hostedMcpResourceUrl(request),
    resource_name: 'LabWired Engine MCP',
    bearer_methods_supported: ['header'],
    resource_documentation: 'https://labwired.com/#agent-harness',
    scopes_supported: [...HOSTED_MCP_OAUTH_SCOPES],
  };

  if (env.MCP_AUTHORIZATION_SERVER) {
    body.authorization_servers = [env.MCP_AUTHORIZATION_SERVER];
  }

  return Response.json(body, {
    headers: {
      'Access-Control-Allow-Origin': '*',
      'Access-Control-Allow-Methods': 'GET, OPTIONS',
      'Access-Control-Allow-Headers': 'Content-Type, Authorization, MCP-Protocol-Version',
    },
  });
}
