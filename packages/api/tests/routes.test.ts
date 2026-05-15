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
) {
  return {
    KV_KEYS: kvKeys as unknown as KVNamespace,
    KV_WORKSPACES: kvWorkspaces as unknown as KVNamespace,
    KV_STRIPE_SUBS: kvSubs as unknown as KVNamespace,
    STRIPE_SECRET_KEY: 'sk_test_placeholder',
    STRIPE_WEBHOOK_SECRET: 'whsec_placeholder',
    RESEND_API_KEY: '',
    FROM_EMAIL: 'onboarding@labwired.com',
    PRO_CYCLES_QUOTA: '100000000',
    ENVIRONMENT: 'test',
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
