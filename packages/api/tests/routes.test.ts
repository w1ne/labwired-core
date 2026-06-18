// Route handler integration tests using stub KV and mocked Stripe
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { generateApiKey, generateWorkspaceId } from '../src/keys.js';
import type { WorkspaceRecord, KeyRecord } from '../src/types.js';

// ── KV stub factory ────────────────────────────────────────────────────────

function makeKvStub() {
  const store = new Map<string, string>();
  return {
    get: vi.fn((key: string) => Promise.resolve(store.get(key) ?? null)),
    put: vi.fn((key: string, value: string) => {
      store.set(key, value);
      return Promise.resolve();
    }),
    delete: vi.fn(),
    list: vi.fn(() => Promise.resolve({ keys: [], list_complete: true })),
    getWithMetadata: vi.fn(),
    _store: store,
  };
}

type KvStub = ReturnType<typeof makeKvStub>;

function makeEnv(
  kvKeys: KvStub,
  kvWorkspaces: KvStub,
  kvSubs: KvStub,
  kvClerk?: KvStub,
  kvProjects?: KvStub,
) {
  return {
    KV_KEYS: kvKeys as unknown as KVNamespace,
    KV_WORKSPACES: kvWorkspaces as unknown as KVNamespace,
    KV_STRIPE_SUBS: kvSubs as unknown as KVNamespace,
    KV_CLERK_TO_WORKSPACE: (kvClerk ?? makeKvStub()) as unknown as KVNamespace,
    KV_PROJECTS: (kvProjects ?? makeKvStub()) as unknown as KVNamespace,
    STRIPE_SECRET_KEY: 'sk_test_placeholder',
    STRIPE_WEBHOOK_SECRET: 'whsec_placeholder',
    PRO_CYCLES_QUOTA: '100000000',
    ENVIRONMENT: 'test',
    // base64("clerk.labwired.com$") — same value the prod Worker carries.
    CLERK_PUBLISHABLE_KEY: 'pk_live_Y2xlcmsubGFid2lyZWQuY29tJA',
    MCP_AUTHORIZATION_SERVER: 'https://clerk.labwired.com',
  };
}

// ── Seed helpers ───────────────────────────────────────────────────────────

function seedWorkspaceAndKey(
  kvKeys: KvStub,
  kvWorkspaces: KvStub,
  options: { status?: WorkspaceRecord['status']; cyclesUsed?: number } = {},
) {
  const apiKey = generateApiKey();
  const workspaceId = generateWorkspaceId();
  const status = options.status ?? 'active';

  const keyRecord: KeyRecord = {
    workspace_id: workspaceId,
    status,
    created_at: new Date().toISOString(),
    last_used_at: null,
  };

  const workspace: WorkspaceRecord = {
    stripe_customer_id: 'cus_test',
    stripe_subscription_id: 'sub_test',
    customer_email: 'test@example.com',
    plan: 'pro',
    cycles_quota_per_month: 100_000_000,
    cycles_used_mtd: options.cyclesUsed ?? 0,
    period_start_date: new Date(new Date().getFullYear(), new Date().getMonth(), 1).toISOString(),
    status,
    created_at: new Date().toISOString(),
    api_key: apiKey,
  };

  kvKeys._store.set(apiKey, JSON.stringify(keyRecord));
  kvWorkspaces._store.set(workspaceId, JSON.stringify(workspace));

  return { apiKey, workspaceId };
}

// ── Import the worker fetch handler ───────────────────────────────────────

const worker = await import('../src/index.js');

describe('public Playground shares', () => {
  it('creates and reads a short public share containing diagram and source code', async () => {
    const kvProjects = makeKvStub();
    const env = makeEnv(makeKvStub(), makeKvStub(), makeKvStub(), makeKvStub(), kvProjects);

    const create = await worker.default.fetch(
      new Request('https://api.labwired.com/v1/shares', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          diagram: {
            board: 'stm32l476',
            parts: [
              { id: 'mcu', type: 'mcu' },
              { id: 'led1', type: 'led', color: 'green' },
            ],
            wires: [
              { from: { part: 'mcu', pin: 'PA5' }, to: { part: 'led1', pin: 'A' } },
            ],
          },
          source: 'int main(void) { return 0; }',
        }),
      }),
      env as any,
    );

    expect(create.status).toBe(201);
    const created = (await create.json()) as any;
    expect(created).toMatchObject({
      id: expect.stringMatching(/^[A-Za-z0-9_-]{12,}$/),
      url: expect.stringContaining('https://app.labwired.com/?share='),
      embed_url: expect.stringContaining('https://app.labwired.com/?embed=true&share='),
    });
    expect(created.url.length).toBeLessThan(90);

    const read = await worker.default.fetch(
      new Request(`https://api.labwired.com/v1/shares/${created.id}`),
      env as any,
    );
    expect(read.status).toBe(200);
    const body = (await read.json()) as any;
    expect(body.source).toBe('int main(void) { return 0; }');
    expect(body.diagram).toMatchObject({
      version: 1,
      board: 'stm32l476',
      parts: [
        { id: 'mcu', attrs: {} },
        { id: 'led1', attrs: { color: 'green' } },
      ],
      wires: [{ color: '#e83e8c' }],
    });
  });

  it('rejects an invalid diagram at the storage boundary (hallucinated pin)', async () => {
    const kvProjects = makeKvStub();
    const env = makeEnv(makeKvStub(), makeKvStub(), makeKvStub(), makeKvStub(), kvProjects);

    const create = await worker.default.fetch(
      new Request('https://api.labwired.com/v1/shares', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          diagram: {
            board: 'stm32l476',
            parts: [
              { id: 'mcu', type: 'mcu' },
              { id: 'led1', type: 'rgb-led' },
            ],
            // rgb-led has no 'DIN' pin (R/G/B/GND) — must not be persisted.
            wires: [{ from: { part: 'mcu', pin: 'PA5' }, to: { part: 'led1', pin: 'DIN' } }],
          },
          source: 'int main(void) { return 0; }',
        }),
      }),
      env as any,
    );

    expect(create.status).toBe(422);
    const body = (await create.json()) as any;
    expect(body.error).toBe('DIAGRAM_INVALID');
    expect(body.validation.ok).toBe(false);
    // Nothing was written to KV.
    expect([...kvProjects._store.keys()].filter((k) => k.startsWith('share:'))).toEqual([]);
  });
});

// ── /v1/keys/validate tests ───────────────────────────────────────────────

describe('POST /v1/keys/validate', () => {
  it('returns 401 for missing key', async () => {
    const kvKeys = makeKvStub();
    const kvWorkspaces = makeKvStub();
    const kvSubs = makeKvStub();
    const env = makeEnv(kvKeys, kvWorkspaces, kvSubs);

    const req = new Request('https://api.labwired.com/v1/keys/validate', {
      method: 'POST',
      body: JSON.stringify({}),
      headers: { 'Content-Type': 'application/json' },
    });

    const resp = await worker.default.fetch(req, env as any);
    expect(resp.status).toBe(401);
  });

  it('returns 401 for unknown key', async () => {
    const kvKeys = makeKvStub();
    const kvWorkspaces = makeKvStub();
    const kvSubs = makeKvStub();
    const env = makeEnv(kvKeys, kvWorkspaces, kvSubs);

    const req = new Request('https://api.labwired.com/v1/keys/validate', {
      method: 'POST',
      body: JSON.stringify({ api_key: generateApiKey() }),
      headers: { 'Content-Type': 'application/json' },
    });

    const resp = await worker.default.fetch(req, env as any);
    expect(resp.status).toBe(401);
  });

  it('returns 200 and quota info for valid active key', async () => {
    const kvKeys = makeKvStub();
    const kvWorkspaces = makeKvStub();
    const kvSubs = makeKvStub();
    const env = makeEnv(kvKeys, kvWorkspaces, kvSubs);

    const { apiKey, workspaceId } = seedWorkspaceAndKey(kvKeys, kvWorkspaces);

    const req = new Request('https://api.labwired.com/v1/keys/validate', {
      method: 'POST',
      body: JSON.stringify({ api_key: apiKey }),
      headers: { 'Content-Type': 'application/json' },
    });

    const resp = await worker.default.fetch(req, env as any);
    expect(resp.status).toBe(200);
    const body = await resp.json() as any;
    expect(body.valid).toBe(true);
    expect(body.workspace_id).toBe(workspaceId);
    expect(body.plan).toBe('pro');
    expect(body.cycles_quota).toBe(100_000_000);
    expect(body.cycles_used_mtd).toBe(0);
  });

  it('returns 403 for canceled key', async () => {
    const kvKeys = makeKvStub();
    const kvWorkspaces = makeKvStub();
    const kvSubs = makeKvStub();
    const env = makeEnv(kvKeys, kvWorkspaces, kvSubs);

    const { apiKey } = seedWorkspaceAndKey(kvKeys, kvWorkspaces, { status: 'canceled' });

    const req = new Request('https://api.labwired.com/v1/keys/validate', {
      method: 'POST',
      body: JSON.stringify({ api_key: apiKey }),
      headers: { 'Content-Type': 'application/json' },
    });

    const resp = await worker.default.fetch(req, env as any);
    expect(resp.status).toBe(403);
  });

  it('returns 403 when quota exhausted', async () => {
    const kvKeys = makeKvStub();
    const kvWorkspaces = makeKvStub();
    const kvSubs = makeKvStub();
    const env = makeEnv(kvKeys, kvWorkspaces, kvSubs);

    const { apiKey } = seedWorkspaceAndKey(kvKeys, kvWorkspaces, {
      cyclesUsed: 100_000_000, // at quota
    });

    const req = new Request('https://api.labwired.com/v1/keys/validate', {
      method: 'POST',
      body: JSON.stringify({ api_key: apiKey }),
      headers: { 'Content-Type': 'application/json' },
    });

    const resp = await worker.default.fetch(req, env as any);
    expect(resp.status).toBe(403);
    const body = await resp.json() as any;
    expect(body.valid).toBe(false);
  });
});

// ── /v1/runs tests ────────────────────────────────────────────────────────

describe('POST /v1/runs', () => {
  it('meters cycles and returns updated totals', async () => {
    const kvKeys = makeKvStub();
    const kvWorkspaces = makeKvStub();
    const kvSubs = makeKvStub();
    const env = makeEnv(kvKeys, kvWorkspaces, kvSubs);

    const { apiKey } = seedWorkspaceAndKey(kvKeys, kvWorkspaces, { cyclesUsed: 1_000_000 });

    const req = new Request('https://api.labwired.com/v1/runs', {
      method: 'POST',
      body: JSON.stringify({
        api_key: apiKey,
        firmware_hash: 'abc123',
        cycles: 500_000,
        duration_ms: 120,
        exit_status: 0,
      }),
      headers: { 'Content-Type': 'application/json' },
    });

    const resp = await worker.default.fetch(req, env as any);
    expect(resp.status).toBe(200);
    const body = await resp.json() as any;
    expect(body.ok).toBe(true);
    expect(body.cycles_used_mtd).toBe(1_500_000);
  });

  it('returns 429 when run would exceed quota', async () => {
    const kvKeys = makeKvStub();
    const kvWorkspaces = makeKvStub();
    const kvSubs = makeKvStub();
    const env = makeEnv(kvKeys, kvWorkspaces, kvSubs);

    const { apiKey } = seedWorkspaceAndKey(kvKeys, kvWorkspaces, {
      cyclesUsed: 99_999_000, // 999k below quota
    });

    const req = new Request('https://api.labwired.com/v1/runs', {
      method: 'POST',
      body: JSON.stringify({
        api_key: apiKey,
        firmware_hash: 'def456',
        cycles: 5_000_000, // would push over
        duration_ms: 600,
        exit_status: 0,
      }),
      headers: { 'Content-Type': 'application/json' },
    });

    const resp = await worker.default.fetch(req, env as any);
    expect(resp.status).toBe(429);
  });

  it('returns 401 for invalid key', async () => {
    const kvKeys = makeKvStub();
    const kvWorkspaces = makeKvStub();
    const kvSubs = makeKvStub();
    const env = makeEnv(kvKeys, kvWorkspaces, kvSubs);

    const req = new Request('https://api.labwired.com/v1/runs', {
      method: 'POST',
      body: JSON.stringify({ api_key: generateApiKey(), firmware_hash: 'x', cycles: 1, duration_ms: 1, exit_status: 0 }),
      headers: { 'Content-Type': 'application/json' },
    });

    const resp = await worker.default.fetch(req, env as any);
    expect(resp.status).toBe(401);
  });
});

// ── /v1/workspaces/me tests ───────────────────────────────────────────────

describe('GET /v1/workspaces/me', () => {
  it('returns workspace info for valid Bearer token', async () => {
    const kvKeys = makeKvStub();
    const kvWorkspaces = makeKvStub();
    const kvSubs = makeKvStub();
    const env = makeEnv(kvKeys, kvWorkspaces, kvSubs);

    const { apiKey, workspaceId } = seedWorkspaceAndKey(kvKeys, kvWorkspaces);

    const req = new Request('https://api.labwired.com/v1/workspaces/me', {
      method: 'GET',
      headers: { Authorization: `Bearer ${apiKey}` },
    });

    const resp = await worker.default.fetch(req, env as any);
    expect(resp.status).toBe(200);
    const body = await resp.json() as any;
    expect(body.workspace_id).toBe(workspaceId);
    expect(body.plan).toBe('pro');
  });

  it('returns 401 without Authorization header', async () => {
    const kvKeys = makeKvStub();
    const kvWorkspaces = makeKvStub();
    const kvSubs = makeKvStub();
    const env = makeEnv(kvKeys, kvWorkspaces, kvSubs);

    const req = new Request('https://api.labwired.com/v1/workspaces/me', {
      method: 'GET',
    });

    const resp = await worker.default.fetch(req, env as any);
    expect(resp.status).toBe(401);
  });
});

// ── 404 for unknown routes ────────────────────────────────────────────────

describe('Unknown routes', () => {
  it('returns 404', async () => {
    const kvKeys = makeKvStub();
    const kvWorkspaces = makeKvStub();
    const kvSubs = makeKvStub();
    const env = makeEnv(kvKeys, kvWorkspaces, kvSubs);

    const req = new Request('https://api.labwired.com/v1/nonexistent', {
      method: 'GET',
    });

    const resp = await worker.default.fetch(req, env as any);
    expect(resp.status).toBe(404);
  });
});

// ── MCP OAuth discovery ───────────────────────────────────────────────────

describe('OAuth discovery for /mcp', () => {
  it('protected-resource metadata advertises an authorization server', async () => {
    const env = makeEnv(makeKvStub(), makeKvStub(), makeKvStub());

    const req = new Request(
      'https://api.labwired.com/.well-known/oauth-protected-resource/mcp',
      { method: 'GET' },
    );
    const resp = await worker.default.fetch(req, env as any);

    expect(resp.status).toBe(200);
    const body = (await resp.json()) as any;
    // The bug being guarded against: this array used to be missing entirely,
    // which dead-ended OAuth discovery and forced a manual API key.
    expect(Array.isArray(body.authorization_servers)).toBe(true);
    expect(body.authorization_servers).toContain('https://api.labwired.com');
    expect(body.resource).toBe('https://api.labwired.com/mcp');
    expect(body.scopes_supported).toEqual(['email', 'offline_access', 'profile']);
    expect(body.bearer_methods_supported).toContain('header');
    // Browser-facing discovery → must be CORS-reachable.
    expect(resp.headers.get('Access-Control-Allow-Origin')).toBe('*');
  });

  it('the bare protected-resource path resolves to the same document', async () => {
    const env = makeEnv(makeKvStub(), makeKvStub(), makeKvStub());

    const req = new Request(
      'https://api.labwired.com/.well-known/oauth-protected-resource',
      { method: 'GET' },
    );
    const resp = await worker.default.fetch(req, env as any);

    expect(resp.status).toBe(200);
    const body = (await resp.json()) as any;
    expect(body.authorization_servers).toContain('https://api.labwired.com');
  });

  it('hosted authorization metadata advertises only Clerk-granted scopes', async () => {
    const env = makeEnv(makeKvStub(), makeKvStub(), makeKvStub());

    const resp = await worker.default.fetch(
      new Request('https://api.labwired.com/.well-known/oauth-authorization-server', {
        method: 'GET',
      }),
      env as any,
    );

    expect(resp.status).toBe(200);
    const body = (await resp.json()) as any;
    expect(body.issuer).toBe('https://api.labwired.com');
    expect(body.authorization_endpoint).toBe('https://clerk.labwired.com/oauth/authorize');
    expect(body.token_endpoint).toBe('https://clerk.labwired.com/oauth/token');
    expect(body.registration_endpoint).toBe('https://api.labwired.com/oauth/register');
    expect(body.scopes_supported).toEqual(['email', 'offline_access', 'profile']);
    expect(body.scopes_supported).not.toContain('openid');
  });

  it('serves OpenID configuration as an alias for hosted authorization metadata', async () => {
    const env = makeEnv(makeKvStub(), makeKvStub(), makeKvStub());

    const resp = await worker.default.fetch(
      new Request('https://api.labwired.com/.well-known/openid-configuration', {
        method: 'GET',
      }),
      env as any,
    );

    expect(resp.status).toBe(200);
    const body = (await resp.json()) as any;
    expect(body.issuer).toBe('https://api.labwired.com');
    expect(body.token_endpoint).toBe('https://clerk.labwired.com/oauth/token');
    expect(body.scopes_supported).toEqual(['email', 'offline_access', 'profile']);
  });

  it('hosted dynamic client registration strips openid before proxying to Clerk', async () => {
    const env = makeEnv(makeKvStub(), makeKvStub(), makeKvStub());
    const originalFetch = globalThis.fetch;
    const clerkRequestBodies: any[] = [];
    vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = input instanceof Request ? input.url : input.toString();
      expect(url).toBe('https://clerk.labwired.com/oauth/register');
      const bodyText =
        input instanceof Request ? await input.clone().text() : (init?.body as string | undefined);
      clerkRequestBodies.push(JSON.parse(bodyText ?? '{}'));
      return Response.json(
        {
          client_id: 'client_123',
          scope: 'email offline_access profile',
          redirect_uris: ['https://chatgpt.com/connector/oauth/test'],
          grant_types: ['authorization_code'],
          response_types: ['code'],
          token_endpoint_auth_method: 'none',
        },
        { status: 201 },
      );
    }));

    try {
      const resp = await worker.default.fetch(
        new Request('https://api.labwired.com/oauth/register', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            client_name: 'ChatGPT',
            redirect_uris: ['https://chatgpt.com/connector/oauth/test'],
            scope: 'openid email offline_access profile',
          }),
        }),
        env as any,
      );

      expect(resp.status).toBe(201);
      expect(clerkRequestBodies[0].scope).toBe('email offline_access profile');
      const body = (await resp.json()) as any;
      expect(body.scope).toBe('email offline_access profile');
    } finally {
      vi.stubGlobal('fetch', originalFetch);
    }
  });

  it('fails closed in production when the authorization server env var is unset', async () => {
    const env = makeEnv(makeKvStub(), makeKvStub(), makeKvStub());
    delete (env as any).MCP_AUTHORIZATION_SERVER;
    (env as any).ENVIRONMENT = 'production';

    const req = new Request(
      'https://api.labwired.com/.well-known/oauth-protected-resource/mcp',
      { method: 'GET' },
    );
    const resp = await worker.default.fetch(req, env as any);
    const body = (await resp.json()) as any;
    expect(resp.status).toBe(500);
    expect(body.error).toBe('MCP_AUTHORIZATION_SERVER_MISSING');
  });

  it('POST /mcp without a token returns 401 with the full OAuth challenge', async () => {
    const env = makeEnv(makeKvStub(), makeKvStub(), makeKvStub());

    const req = new Request('https://api.labwired.com/mcp', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'initialize', params: {} }),
    });
    const resp = await worker.default.fetch(req, env as any);

    expect(resp.status).toBe(401);
    const challenge = resp.headers.get('WWW-Authenticate') ?? '';
    expect(challenge).toContain('realm="LabWired MCP"');
    expect(challenge).toContain('resource_metadata=');
    expect(challenge).toContain('scope="email offline_access profile"');
  });

  it('GET /mcp without a token returns the OAuth challenge for connector URL probes', async () => {
    const env = makeEnv(makeKvStub(), makeKvStub(), makeKvStub());

    const resp = await worker.default.fetch(
      new Request('https://api.labwired.com/mcp', { method: 'GET' }),
      env as any,
    );

    expect(resp.status).toBe(401);
    const challenge = resp.headers.get('WWW-Authenticate') ?? '';
    expect(challenge).toContain('realm="LabWired MCP"');
    expect(challenge).toContain('resource_metadata=');
  });

  it('HEAD /mcp without a token returns the OAuth challenge for URL validators', async () => {
    const env = makeEnv(makeKvStub(), makeKvStub(), makeKvStub());

    const resp = await worker.default.fetch(
      new Request('https://api.labwired.com/mcp', { method: 'HEAD' }),
      env as any,
    );

    expect(resp.status).toBe(401);
    const challenge = resp.headers.get('WWW-Authenticate') ?? '';
    expect(challenge).toContain('realm="LabWired MCP"');
    expect(challenge).toContain('resource_metadata=');
  });

  it('GET /mcp with a valid token returns method guidance instead of a parse error', async () => {
    const env = makeEnv(makeKvStub(), makeKvStub(), makeKvStub());

    const resp = await worker.default.fetch(
      new Request('https://api.labwired.com/mcp', {
        method: 'GET',
        headers: { Authorization: 'Bearer test_user:user_abc' },
      }),
      env as any,
    );

    expect(resp.status).toBe(405);
    expect(resp.headers.get('Allow')).toBe('POST, OPTIONS');
    const body = (await resp.json()) as any;
    expect(body.error.message).toBe('Method not allowed');
  });

  it('OPTIONS /mcp allows MCP protocol headers for browser clients', async () => {
    const env = makeEnv(makeKvStub(), makeKvStub(), makeKvStub());

    const resp = await worker.default.fetch(
      new Request('https://api.labwired.com/mcp', {
        method: 'OPTIONS',
        headers: {
          'Access-Control-Request-Headers': 'Content-Type, Authorization, MCP-Protocol-Version',
        },
      }),
      env as any,
    );

    expect(resp.status).toBe(204);
    expect(resp.headers.get('Access-Control-Allow-Headers')).toContain('MCP-Protocol-Version');
  });

  it('POST /mcp with a valid token handles initialize + tools/list', async () => {
    const env = makeEnv(makeKvStub(), makeKvStub(), makeKvStub());
    const headers = {
      'Content-Type': 'application/json',
      Authorization: 'Bearer test_user:user_abc',
    };

    const init = await worker.default.fetch(
      new Request('https://api.labwired.com/mcp', {
        method: 'POST',
        headers,
        body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'initialize', params: {} }),
      }),
      env as any,
    );
    expect(init.status).toBe(200);
    const initBody = (await init.json()) as any;
    expect(initBody.result.serverInfo).toBeTruthy();
    expect(initBody.result.capabilities.resources).toEqual({});

    const list = await worker.default.fetch(
      new Request('https://api.labwired.com/mcp', {
        method: 'POST',
        headers,
        body: JSON.stringify({ jsonrpc: '2.0', id: 2, method: 'tools/list' }),
      }),
      env as any,
    );
    const listBody = (await list.json()) as any;
    expect(Array.isArray(listBody.result.tools)).toBe(true);
    expect(listBody.result.tools.length).toBeGreaterThan(0);
  });

  it('POST /mcp with a valid token handles resources/list + resources/read', async () => {
    const env = makeEnv(makeKvStub(), makeKvStub(), makeKvStub());
    const headers = {
      'Content-Type': 'application/json',
      Authorization: 'Bearer test_user:user_abc',
    };

    const list = await worker.default.fetch(
      new Request('https://api.labwired.com/mcp', {
        method: 'POST',
        headers,
        body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'resources/list' }),
      }),
      env as any,
    );
    expect(list.status).toBe(200);
    const listBody = (await list.json()) as any;
    expect(listBody.result.resources).toContainEqual(
      expect.objectContaining({
        uri: 'labwired://guides/agent-hardware-loop',
        name: 'labwired-agent-hardware-loop',
        mimeType: 'text/markdown',
      }),
    );
    expect(listBody.result.resources).toContainEqual(
      expect.objectContaining({
        uri: 'ui://widget/labwired-hardware-lab-v8.html',
        name: 'labwired-hardware-lab',
        mimeType: 'text/html;profile=mcp-app',
      }),
    );

    const read = await worker.default.fetch(
      new Request('https://api.labwired.com/mcp', {
        method: 'POST',
        headers,
        body: JSON.stringify({
          jsonrpc: '2.0',
          id: 2,
          method: 'resources/read',
          params: { uri: 'labwired://guides/agent-hardware-loop' },
        }),
      }),
      env as any,
    );
    expect(read.status).toBe(200);
    const readBody = (await read.json()) as any;
    expect(readBody.result.contents[0]).toMatchObject({
      uri: 'labwired://guides/agent-hardware-loop',
      mimeType: 'text/markdown',
    });
    expect(readBody.result.contents[0].text).toContain('LabWired agent hardware loop');

    const html = await worker.default.fetch(
      new Request('https://api.labwired.com/mcp', {
        method: 'POST',
        headers,
        body: JSON.stringify({
          jsonrpc: '2.0',
          id: 3,
          method: 'resources/read',
          params: { uri: 'ui://widget/labwired-hardware-lab-v7.html' },
        }),
      }),
      env as any,
    );
    expect(html.status).toBe(200);
    const htmlBody = (await html.json()) as any;
    expect(htmlBody.result.contents[0]).toMatchObject({
      uri: 'ui://widget/labwired-hardware-lab-v7.html',
      mimeType: 'text/html;profile=mcp-app',
      _meta: {
        ui: {
          prefersBorder: true,
          csp: expect.objectContaining({
            connectDomains: expect.arrayContaining(['https://app.labwired.com']),
            frameDomains: expect.arrayContaining(['https://app.labwired.com']),
            resourceDomains: expect.arrayContaining(['https://app.labwired.com']),
          }),
        },
        'openai/widgetDescription': expect.any(String),
        'openai/widgetCSP': expect.objectContaining({
          connect_domains: expect.arrayContaining(['https://app.labwired.com']),
          frame_domains: expect.arrayContaining(['https://app.labwired.com']),
          resource_domains: expect.arrayContaining(['https://app.labwired.com']),
          redirect_domains: expect.arrayContaining(['https://app.labwired.com']),
        }),
      },
    });
    expect(htmlBody.result.contents[0].text).toContain('LabWired Hardware Lab');
    expect(htmlBody.result.contents[0].text).toContain('id="labwired-frame"');
    expect(htmlBody.result.contents[0].text).toContain('<iframe');
    expect(htmlBody.result.contents[0].text).toContain('frame.src = frameUrl');
    expect(htmlBody.result.contents[0].text).toContain('ui/notifications/tool-result');
    expect(htmlBody.result.contents[0].text).toContain('openai:set_globals');
    expect(htmlBody.result.contents[0].text).toContain('setOpenInAppUrl');
    expect(htmlBody.result.contents[0].text).toContain('redirectUrl: false');
    // Fullscreen/expand affordance for the cramped inline pane.
    expect(htmlBody.result.contents[0].text).toContain('id="fullscreen"');
    expect(htmlBody.result.contents[0].text).toContain('requestDisplayMode');
    expect(htmlBody.result.contents[0].text).not.toContain('data.watch_url');
    expect(htmlBody.result.contents[0].text).not.toContain("'https://app.labwired.com/'");
    expect(htmlBody.result.contents[0].text).not.toContain('renderScene');
    expect(htmlBody.result.contents[0].text).not.toContain("className = 'part'");
  });

  it('POST /mcp exposes static app resources without auth for component fetches', async () => {
    const env = makeEnv(makeKvStub(), makeKvStub(), makeKvStub());

    const list = await worker.default.fetch(
      new Request('https://api.labwired.com/mcp', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'resources/list' }),
      }),
      env as any,
    );
    expect(list.status).toBe(200);
    const listBody = (await list.json()) as any;
    expect(listBody.result.resources).toContainEqual(
      expect.objectContaining({
        uri: 'ui://widget/labwired-hardware-lab-v8.html',
        mimeType: 'text/html;profile=mcp-app',
      }),
    );

    const read = await worker.default.fetch(
      new Request('https://api.labwired.com/mcp', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          jsonrpc: '2.0',
          id: 2,
          method: 'resources/read',
          params: { uri: 'ui://widget/labwired-hardware-lab-v7.html' },
        }),
      }),
      env as any,
    );
    expect(read.status).toBe(200);
    const readBody = (await read.json()) as any;
    expect(readBody.result.contents[0]).toMatchObject({
      uri: 'ui://widget/labwired-hardware-lab-v7.html',
      mimeType: 'text/html;profile=mcp-app',
    });
    expect(readBody.result.contents[0].text).toContain('LabWired Hardware Lab');
  });

  it('POST /mcp serves legacy app template URIs for cached ChatGPT descriptors', async () => {
    const env = makeEnv(makeKvStub(), makeKvStub(), makeKvStub());

    const read = await worker.default.fetch(
      new Request('https://api.labwired.com/mcp', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          jsonrpc: '2.0',
          id: 1,
          method: 'resources/read',
          params: { uri: 'ui://labwired/hardware-lab.html' },
        }),
      }),
      env as any,
    );
    expect(read.status).toBe(200);
    const body = (await read.json()) as any;
    expect(body.result.contents[0]).toMatchObject({
      uri: 'ui://labwired/hardware-lab.html',
      mimeType: 'text/html;profile=mcp-app',
    });
    expect(body.result.contents[0].text).toContain('LabWired Hardware Lab');
  });
});

// ── CORS preflight ────────────────────────────────────────────────────────

describe('CORS preflight', () => {
  it('returns 204 for OPTIONS', async () => {
    const kvKeys = makeKvStub();
    const kvWorkspaces = makeKvStub();
    const kvSubs = makeKvStub();
    const env = makeEnv(kvKeys, kvWorkspaces, kvSubs);

    const req = new Request('https://api.labwired.com/v1/keys/validate', {
      method: 'OPTIONS',
    });

    const resp = await worker.default.fetch(req, env as any);
    expect(resp.status).toBe(204);
    expect(resp.headers.get('Access-Control-Allow-Origin')).toBe('*');
  });
});
