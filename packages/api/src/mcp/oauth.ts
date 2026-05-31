import type { Env } from '../types.js';

const MCP_SCOPE = 'labwired:mcp';

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
    `scope="${MCP_SCOPE}"`,
  ].join(', ');
}

export function handleMcpProtectedResourceMetadata(request: Request, env: Env): Response {
  const body: {
    resource: string;
    resource_name: string;
    scopes_supported: string[];
    bearer_methods_supported: string[];
    resource_documentation: string;
    authorization_servers?: string[];
  } = {
    resource: hostedMcpResourceUrl(request),
    resource_name: 'LabWired Engine MCP',
    scopes_supported: [MCP_SCOPE],
    bearer_methods_supported: ['header'],
    resource_documentation: 'https://labwired.com/#agent-harness',
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
