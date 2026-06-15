// Hosted MCP auth: workspace API-key path (additive) + OAuth/Clerk path (must
// keep working). The API-key path lets agents/CI authenticate with a pasted
// `lwk_live_` key instead of the interactive browser OAuth flow.
import { describe, it, expect, vi } from 'vitest';
import { generateApiKey } from '../src/keys.js';
import type { Env, KeyRecord } from '../src/types.js';

// Control the Clerk mock's auth state so we can exercise the OAuth fall-through.
let mockAuthState: { isAuthenticated: boolean; userId?: string } = { isAuthenticated: false };
let mockUserinfo:
  | { status: number; body: Record<string, unknown>; authorization?: string }
  | undefined;
vi.mock('@clerk/backend', () => ({
  createClerkClient: () => ({
    authenticateRequest: vi.fn(async () =>
      mockAuthState.isAuthenticated
        ? {
            isAuthenticated: true,
            toAuth: () => ({ userId: mockAuthState.userId, sessionId: 's', sessionClaims: {} }),
          }
        : { isAuthenticated: false, toAuth: () => null },
    ),
  }),
}));

const originalFetch = globalThis.fetch;
beforeEach(() => {
  mockUserinfo = undefined;
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = input instanceof Request ? input.url : input.toString();
      if (url === 'https://clerk.labwired.com/oauth/userinfo' && mockUserinfo) {
        mockUserinfo.authorization =
          input instanceof Request
            ? input.headers.get('authorization') ?? undefined
            : (init?.headers as Record<string, string> | undefined)?.Authorization;
        return Response.json(mockUserinfo.body, { status: mockUserinfo.status });
      }
      return originalFetch(input as RequestInfo, init);
    }),
  );
});

afterEach(() => {
  mockAuthState = { isAuthenticated: false };
  mockUserinfo = undefined;
  vi.unstubAllGlobals();
});

const { authenticateHostedMcpRequest } = await import('../src/mcp/auth.js');

function makeKvStub() {
  const store = new Map<string, string>();
  return {
    get: vi.fn((k: string) => Promise.resolve(store.get(k) ?? null)),
    put: vi.fn((k: string, v: string) => {
      store.set(k, v);
      return Promise.resolve();
    }),
    delete: vi.fn((k: string) => {
      store.delete(k);
      return Promise.resolve();
    }),
    list: vi.fn(() => Promise.resolve({ keys: [], list_complete: true })),
    getWithMetadata: vi.fn(),
    _store: store,
  };
}
type KvStub = ReturnType<typeof makeKvStub>;

function makeEnv(kvKeys: KvStub = makeKvStub()): Env {
  return {
    KV_KEYS: kvKeys as unknown as KVNamespace,
    KV_WORKSPACES: makeKvStub() as unknown as KVNamespace,
    KV_STRIPE_SUBS: makeKvStub() as unknown as KVNamespace,
    KV_CLERK_TO_WORKSPACE: makeKvStub() as unknown as KVNamespace,
    STRIPE_SECRET_KEY: 'sk_test',
    STRIPE_WEBHOOK_SECRET: 'whsec',
    CLERK_SECRET_KEY: 'sk_test_clerk',
    PRO_CYCLES_QUOTA: '100000000',
    // 'production' so the test-only `test_user:` backdoor is NOT what carries
    // these cases — the API-key path is real, environment-independent behaviour.
    ENVIRONMENT: 'production',
    CLERK_JWT_KEY: '-----BEGIN PUBLIC KEY-----\nx\n-----END PUBLIC KEY-----',
    MCP_AUTHORIZATION_SERVER: 'https://clerk.labwired.com',
  } as unknown as Env;
}

function req(authHeader?: string): Request {
  return new Request('https://api.labwired.com/mcp', {
    method: 'POST',
    headers: authHeader ? { authorization: authHeader } : {},
  });
}

function seedKey(kvKeys: KvStub, status: KeyRecord['status'] = 'active'): string {
  const key = generateApiKey(); // lwk_live_…
  const record: KeyRecord = {
    workspace_id: 'ws_abc123',
    status,
    created_at: new Date().toISOString(),
    last_used_at: null,
  };
  kvKeys._store.set(key, JSON.stringify(record));
  return key;
}

describe('authenticateHostedMcpRequest — workspace API key', () => {
  it('authenticates a valid lwk_live_ key and resolves its workspace', async () => {
    const kvKeys = makeKvStub();
    const key = seedKey(kvKeys);
    const res = await authenticateHostedMcpRequest(req(`Bearer ${key}`), makeEnv(kvKeys));
    expect(res).not.toBeInstanceOf(Response);
    expect(res).toMatchObject({ workspaceId: 'ws_abc123', userId: 'key:ws_abc123' });
  });

  it('stamps last_used_at on the key', async () => {
    const kvKeys = makeKvStub();
    const key = seedKey(kvKeys);
    await authenticateHostedMcpRequest(req(`Bearer ${key}`), makeEnv(kvKeys));
    const rec = JSON.parse(kvKeys._store.get(key) as string) as KeyRecord;
    expect(rec.last_used_at).not.toBeNull();
  });

  it('rejects a non-active (canceled) key with 401', async () => {
    const kvKeys = makeKvStub();
    const key = seedKey(kvKeys, 'canceled');
    const res = await authenticateHostedMcpRequest(req(`Bearer ${key}`), makeEnv(kvKeys));
    expect(res).toBeInstanceOf(Response);
    expect((res as Response).status).toBe(401);
  });

  it('rejects an unknown lwk_live_ key with 401', async () => {
    const res = await authenticateHostedMcpRequest(
      req('Bearer lwk_live_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA'),
      makeEnv(),
    );
    expect((res as Response).status).toBe(401);
  });

  it('rejects a missing Authorization header with 401', async () => {
    const res = await authenticateHostedMcpRequest(req(), makeEnv());
    expect((res as Response).status).toBe(401);
  });
});

describe('authenticateHostedMcpRequest — OAuth/Clerk path stays working', () => {
  it('authenticates via Clerk when the bearer is not an API key', async () => {
    mockAuthState = { isAuthenticated: true, userId: 'user_clerk_1' };
    const res = await authenticateHostedMcpRequest(req('Bearer eyJ.fake.jwt'), makeEnv());
    expect(res).not.toBeInstanceOf(Response);
    expect(res).toMatchObject({ userId: 'user_clerk_1' });
  });

  it('authenticates Clerk OAuth access tokens via userinfo when session JWT auth rejects them', async () => {
    mockAuthState = { isAuthenticated: false };
    mockUserinfo = {
      status: 200,
      body: {
        sub: 'user_oauth_1',
        email: 'agent@example.com',
      },
    };

    const res = await authenticateHostedMcpRequest(req('Bearer oauth_access_token'), makeEnv());

    expect(res).not.toBeInstanceOf(Response);
    expect(res).toMatchObject({ userId: 'user_oauth_1' });
    expect(mockUserinfo.authorization).toBe('Bearer oauth_access_token');
  });

  it('returns 401 when Clerk rejects the token', async () => {
    mockAuthState = { isAuthenticated: false };
    const res = await authenticateHostedMcpRequest(req('Bearer eyJ.fake.jwt'), makeEnv());
    expect((res as Response).status).toBe(401);
  });
});
