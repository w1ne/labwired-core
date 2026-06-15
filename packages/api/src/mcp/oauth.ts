import type { Env } from '../types.js';

export const HOSTED_MCP_OAUTH_SCOPES = ['email', 'offline_access', 'profile'] as const;

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

export function hostedMcpAuthorizationServerUrl(request: Request): string {
  return originFromRequest(request);
}

function clerkAuthorizationServer(env: Env): string {
  return env.MCP_AUTHORIZATION_SERVER ?? 'https://clerk.labwired.com';
}

export function hostedMcpAuthenticateHeader(request: Request): string {
  return [
    'Bearer realm="LabWired MCP"',
    `resource_metadata="${protectedResourceMetadataUrl(request)}"`,
    `scope="${HOSTED_MCP_OAUTH_SCOPES.join(' ')}"`,
  ].join(', ');
}

export function handleMcpProtectedResourceMetadata(request: Request, env: Env): Response {
  const clerkIssuer = env.MCP_AUTHORIZATION_SERVER;
  if (!clerkIssuer && env.ENVIRONMENT !== 'test') {
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

  body.authorization_servers = [hostedMcpAuthorizationServerUrl(request)];

  return Response.json(body, {
    headers: {
      'Access-Control-Allow-Origin': '*',
      'Access-Control-Allow-Methods': 'GET, OPTIONS',
      'Access-Control-Allow-Headers': 'Content-Type, Authorization, MCP-Protocol-Version',
    },
  });
}

export function handleHostedMcpAuthorizationServerMetadata(request: Request, env: Env): Response {
  const issuer = hostedMcpAuthorizationServerUrl(request);
  const clerkIssuer = clerkAuthorizationServer(env);
  return Response.json(
    {
      issuer,
      authorization_endpoint: `${clerkIssuer}/oauth/authorize`,
      token_endpoint: `${clerkIssuer}/oauth/token`,
      revocation_endpoint: `${clerkIssuer}/oauth/token/revoke`,
      jwks_uri: `${clerkIssuer}/.well-known/jwks.json`,
      registration_endpoint: `${issuer}/oauth/register`,
      response_types_supported: ['code'],
      grant_types_supported: ['authorization_code', 'refresh_token'],
      token_endpoint_auth_methods_supported: ['client_secret_basic', 'none', 'client_secret_post'],
      scopes_supported: [...HOSTED_MCP_OAUTH_SCOPES],
      code_challenge_methods_supported: ['S256'],
    },
    {
      headers: {
        'Access-Control-Allow-Origin': '*',
        'Access-Control-Allow-Methods': 'GET, OPTIONS',
        'Access-Control-Allow-Headers': 'Content-Type, Authorization, MCP-Protocol-Version',
      },
    },
  );
}

function allowedScopeString(): string {
  return HOSTED_MCP_OAUTH_SCOPES.join(' ');
}

function sanitizeRegistrationBody(body: Record<string, unknown>): Record<string, unknown> {
  const next = { ...body };
  next.scope = allowedScopeString();
  return next;
}

export async function handleHostedMcpDynamicClientRegistration(
  request: Request,
  env: Env,
): Promise<Response> {
  let body: Record<string, unknown>;
  try {
    body = (await request.json()) as Record<string, unknown>;
  } catch {
    return Response.json(
      { error: 'invalid_client_metadata', error_description: 'Registration body must be JSON.' },
      { status: 400, headers: { 'Access-Control-Allow-Origin': '*' } },
    );
  }

  const clerkResp = await fetch(`${clerkAuthorizationServer(env)}/oauth/register`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(sanitizeRegistrationBody(body)),
  });

  const text = await clerkResp.text();
  let responseBody: unknown = text;
  try {
    responseBody = text ? JSON.parse(text) : {};
  } catch {
    responseBody = { error: 'invalid_registration_response', response: text };
  }

  if (responseBody && typeof responseBody === 'object' && !Array.isArray(responseBody)) {
    responseBody = { ...responseBody, scope: allowedScopeString() };
  }

  return Response.json(responseBody, {
    status: clerkResp.status,
    headers: { 'Access-Control-Allow-Origin': '*' },
  });
}
