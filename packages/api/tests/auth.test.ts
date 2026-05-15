// Clerk-backed /v1/auth/me + /v1/keys/rotate tests
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { generateApiKey, generateWorkspaceId } from '../src/keys.js';
import type { WorkspaceRecord, KeyRecord } from '../src/types.js';

// Mock @clerk/backend so we don't hit the network. The mock returns a client
// whose authenticateRequest behaviour we control via `mockAuthState`.
let mockAuthState: {
  isAuthenticated: boolean;
  userId?: string;
  sessionId?: string;
  claims?: Record<string, unknown>;
} = { isAuthenticated: false };

vi.mock('@clerk/backend', () => ({
  createClerkClient: () => ({
    authenticateRequest: vi.fn(async () => {
      if (mockAuthState.isAuthenticated) {
        return {
          isAuthenticated: true,
          toAuth: () => ({
            userId: mockAuthState.userId,
            sessionId: mockAuthState.sessionId,
            sessionClaims: mockAuthState.claims ?? {},
          }),
        };
      }
      return { isAuthenticated: false, toAuth: () => null };
    }),
  }),
}));

function makeKvStub() {
  const store = new Map<string, string>();
  return {
    get: vi.fn((key: string) => Promise.resolve(store.get(key) ?? null)),
    put: vi.fn((key: string, value: string) => {
      store.set(key, value);
      return Promise.resolve();
    }),
    delete: vi.fn((key: string) => {
      store.delete(key);
      return Promise.resolve();
    }),
    list: vi.fn(() => Promise.resolve({ keys: [], list_complete: true })),
    getWithMetadata: vi.fn(),
    _store: store,
  };
}

type KvStub = ReturnType<typeof makeKvStub>;

function makeEnv(opts: {
  kvKeys?: KvStub;
  kvWorkspaces?: KvStub;
  kvSubs?: KvStub;
  kvClerk?: KvStub;
} = {}) {
  return {
    KV_KEYS: (opts.kvKeys ?? makeKvStub()) as unknown as KVNamespace,
    KV_WORKSPACES: (opts.kvWorkspaces ?? makeKvStub()) as unknown as KVNamespace,
    KV_STRIPE_SUBS: (opts.kvSubs ?? makeKvStub()) as unknown as KVNamespace,
    KV_CLERK_TO_WORKSPACE: (opts.kvClerk ?? makeKvStub()) as unknown as KVNamespace,
    STRIPE_SECRET_KEY: 'sk_test_placeholder',
    STRIPE_WEBHOOK_SECRET: 'whsec_placeholder',
    CLERK_SECRET_KEY: 'sk_test_clerk_placeholder',
    PRO_CYCLES_QUOTA: '100000000',
    ENVIRONMENT: 'test',
    CLERK_JWT_KEY: '-----BEGIN PUBLIC KEY-----\nplaceholder\n-----END PUBLIC KEY-----',
  };
}

function seedPaidWorkspaceForClerkUser(
  clerkUserId: string,
  kvKeys: KvStub,
  kvWorkspaces: KvStub,
  kvClerk: KvStub,
) {
  const apiKey = generateApiKey();
  const workspaceId = generateWorkspaceId();

  const keyRecord: KeyRecord = {
    workspace_id: workspaceId,
    status: 'active',
    created_at: new Date().toISOString(),
    last_used_at: null,
  };
  const workspace: WorkspaceRecord = {
    stripe_customer_id: 'cus_test',
    stripe_subscription_id: 'sub_test',
    customer_email: 'andrii@example.com',
    plan: 'pro',
    cycles_quota_per_month: 100_000_000,
    cycles_used_mtd: 1234,
    period_start_date: new Date(new Date().getFullYear(), new Date().getMonth(), 1).toISOString(),
    status: 'active',
    created_at: new Date().toISOString(),
    api_key: apiKey,
    clerk_user_id: clerkUserId,
  };

  kvKeys._store.set(apiKey, JSON.stringify(keyRecord));
  kvWorkspaces._store.set(workspaceId, JSON.stringify(workspace));
  kvClerk._store.set(clerkUserId, workspaceId);

  return { apiKey, workspaceId };
}

const worker = await import('../src/index.js');

describe('GET /v1/auth/me', () => {
  beforeEach(() => {
    mockAuthState = { isAuthenticated: false };
  });

  it('returns plan=free and no api_key for an authenticated Clerk user with no workspace', async () => {
    mockAuthState = {
      isAuthenticated: true,
      userId: 'user_2abc123',
      sessionId: 'sess_2xyz789',
      claims: { email: 'andrii@example.com' },
    };

    const req = new Request('https://api.labwired.com/v1/auth/me', {
      method: 'GET',
      headers: { Authorization: 'Bearer ey.fake.jwt' },
    });

    const resp = await worker.default.fetch(req, makeEnv() as any);
    expect(resp.status).toBe(200);

    const body = (await resp.json()) as Record<string, unknown>;
    expect(body.user_id).toBe('user_2abc123');
    expect(body.session_id).toBe('sess_2xyz789');
    expect(body.email).toBe('andrii@example.com');
    expect(body.plan).toBe('free');
    expect(body.api_key).toBeUndefined();
    expect(body.workspace_id).toBeUndefined();
  });

  it('returns plan=pro, workspace_id, api_key, and quota for a paid Clerk user', async () => {
    const clerkUserId = 'user_paid';
    mockAuthState = {
      isAuthenticated: true,
      userId: clerkUserId,
      sessionId: 'sess_paid',
      claims: { email: 'andrii@example.com' },
    };

    const kvKeys = makeKvStub();
    const kvWorkspaces = makeKvStub();
    const kvClerk = makeKvStub();
    const { apiKey, workspaceId } = seedPaidWorkspaceForClerkUser(
      clerkUserId,
      kvKeys,
      kvWorkspaces,
      kvClerk,
    );

    const env = makeEnv({ kvKeys, kvWorkspaces, kvClerk });
    const req = new Request('https://api.labwired.com/v1/auth/me', {
      method: 'GET',
      headers: { Authorization: 'Bearer ey.fake.jwt' },
    });

    const resp = await worker.default.fetch(req, env as any);
    expect(resp.status).toBe(200);

    const body = (await resp.json()) as Record<string, unknown>;
    expect(body.plan).toBe('pro');
    expect(body.workspace_id).toBe(workspaceId);
    expect(body.api_key).toBe(apiKey);
    expect(body.cycles_used_mtd).toBe(1234);
    expect(body.cycles_quota).toBe(100_000_000);
    expect(body.status).toBe('active');
  });

  it('returns 401 when Clerk reports unauthenticated', async () => {
    mockAuthState = { isAuthenticated: false };
    const req = new Request('https://api.labwired.com/v1/auth/me', { method: 'GET' });
    const resp = await worker.default.fetch(req, makeEnv() as any);
    expect(resp.status).toBe(401);
  });
});

describe('POST /v1/keys/rotate', () => {
  beforeEach(() => {
    mockAuthState = { isAuthenticated: false };
  });

  it('returns 401 when Clerk reports unauthenticated', async () => {
    const req = new Request('https://api.labwired.com/v1/keys/rotate', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: '{}',
    });
    const resp = await worker.default.fetch(req, makeEnv() as any);
    expect(resp.status).toBe(401);
  });

  it('returns 404 when the Clerk user has no mapped workspace', async () => {
    mockAuthState = {
      isAuthenticated: true,
      userId: 'user_free',
      sessionId: 'sess_x',
      claims: {},
    };
    const req = new Request('https://api.labwired.com/v1/keys/rotate', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: '{}',
    });
    const resp = await worker.default.fetch(req, makeEnv() as any);
    expect(resp.status).toBe(404);
  });

  it('returns a new key, invalidates the old key, and updates the workspace', async () => {
    const clerkUserId = 'user_paid';
    mockAuthState = {
      isAuthenticated: true,
      userId: clerkUserId,
      sessionId: 'sess_paid',
      claims: { email: 'andrii@example.com' },
    };

    const kvKeys = makeKvStub();
    const kvWorkspaces = makeKvStub();
    const kvClerk = makeKvStub();
    const { apiKey: oldKey, workspaceId } = seedPaidWorkspaceForClerkUser(
      clerkUserId,
      kvKeys,
      kvWorkspaces,
      kvClerk,
    );
    const env = makeEnv({ kvKeys, kvWorkspaces, kvClerk });

    const req = new Request('https://api.labwired.com/v1/keys/rotate', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: '{}',
    });

    const resp = await worker.default.fetch(req, env as any);
    expect(resp.status).toBe(200);

    const body = (await resp.json()) as { api_key: string; workspace_id: string };
    expect(body.workspace_id).toBe(workspaceId);
    expect(body.api_key.startsWith('lwk_live_')).toBe(true);
    expect(body.api_key).not.toBe(oldKey);

    // Old key gone from KV_KEYS
    expect(kvKeys._store.has(oldKey)).toBe(false);
    // New key present in KV_KEYS, pointing at the same workspace
    expect(kvKeys._store.has(body.api_key)).toBe(true);
    // Workspace api_key updated
    const stored = JSON.parse(kvWorkspaces._store.get(workspaceId) ?? '{}') as WorkspaceRecord;
    expect(stored.api_key).toBe(body.api_key);
  });
});
