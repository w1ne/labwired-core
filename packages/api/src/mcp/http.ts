import type { Env } from '../types.js';
import { authenticateHostedMcpRequest } from './auth.js';
import { callHostedTool, listHostedTools } from './tools.js';
import type { JsonRpcFailure, JsonRpcRequest, JsonRpcResponse } from './types.js';
import { RESOURCES, getResource } from './resources.js';

const JSON_HEADERS = {
  'Content-Type': 'application/json',
  'Access-Control-Allow-Origin': '*',
};
const MCP_ALLOW_HEADER = 'POST, OPTIONS';

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: JSON_HEADERS });
}

function error(id: JsonRpcRequest['id'], code: number, message: string): JsonRpcFailure {
  return { jsonrpc: '2.0', id: id ?? null, error: { code, message } };
}

export async function handleHostedMcp(request: Request, env: Env): Promise<Response> {
  const identity = await authenticateHostedMcpRequest(request, env);
  if (identity instanceof Response) return identity;

  if (request.method.toUpperCase() !== 'POST') {
    return new Response(JSON.stringify(error(null, -32000, 'Method not allowed')), {
      status: 405,
      headers: { ...JSON_HEADERS, Allow: MCP_ALLOW_HEADER },
    });
  }

  let rpc: JsonRpcRequest;
  try {
    rpc = (await request.json()) as JsonRpcRequest;
  } catch {
    return json(error(null, -32700, 'Parse error'), 400);
  }

  if (rpc.jsonrpc !== '2.0' || typeof rpc.method !== 'string') {
    return json(error(rpc.id, -32600, 'Invalid Request'), 400);
  }

  if (rpc.method === 'initialize') {
    return json({
      jsonrpc: '2.0',
      id: rpc.id ?? null,
      result: {
        protocolVersion: '2025-06-18',
        capabilities: { tools: {}, resources: {} },
        serverInfo: { name: '@labwired/hosted-mcp', version: '0.1.0' },
      },
    } satisfies JsonRpcResponse);
  }

  if (rpc.method === 'tools/list') {
    return json({
      jsonrpc: '2.0',
      id: rpc.id ?? null,
      result: { tools: listHostedTools() },
    } satisfies JsonRpcResponse);
  }

  if (rpc.method === 'tools/call') {
    const result = await callHostedTool(rpc.params, env, identity);
    return json({ jsonrpc: '2.0', id: rpc.id ?? null, result } satisfies JsonRpcResponse);
  }

  if (rpc.method === 'resources/list') {
    return json({
      jsonrpc: '2.0',
      id: rpc.id ?? null,
      result: { resources: RESOURCES },
    } satisfies JsonRpcResponse);
  }

  if (rpc.method === 'resources/read') {
    const params = (rpc.params ?? {}) as { uri?: unknown };
    const uri = typeof params.uri === 'string' ? params.uri : '';
    const resource = getResource(uri);
    if (!resource) return json(error(rpc.id, -32004, `Unknown resource: ${uri}`), 404);
    return json({
      jsonrpc: '2.0',
      id: rpc.id ?? null,
      result: { contents: [resource] },
    } satisfies JsonRpcResponse);
  }

  if (rpc.method.startsWith('notifications/')) {
    return new Response(null, { status: 202, headers: { 'Access-Control-Allow-Origin': '*' } });
  }

  return json(error(rpc.id, -32601, 'Method not found'), 404);
}
