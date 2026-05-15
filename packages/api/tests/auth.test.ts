// GitHub OAuth session route tests
import { describe, it, expect, vi } from 'vitest';
import type { SessionRecord } from '../src/types.js';

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

function makeEnv(kvSessions: KvStub) {
  return {
    KV_KEYS: makeKvStub() as unknown as KVNamespace,
    KV_WORKSPACES: makeKvStub() as unknown as KVNamespace,
    KV_STRIPE_SUBS: makeKvStub() as unknown as KVNamespace,
    KV_SESSIONS: kvSessions as unknown as KVNamespace,
    STRIPE_SECRET_KEY: 'sk_test_placeholder',
    STRIPE_WEBHOOK_SECRET: 'whsec_placeholder',
    RESEND_API_KEY: '',
    GITHUB_CLIENT_SECRET: 'gh_secret_placeholder',
    FROM_EMAIL: 'onboarding@labwired.com',
    PRO_CYCLES_QUOTA: '100000000',
    ENVIRONMENT: 'test',
    GITHUB_CLIENT_ID: 'gh_client_placeholder',
    PLAYGROUND_ORIGIN: 'https://foundry.labwired.com',
  };
}

const worker = await import('../src/index.js');

describe('GET /v1/auth/me', () => {
  it('returns the session payload for a valid Bearer token', async () => {
    const kvSessions = makeKvStub();
    const token = 'sess_token_abc';
    const record: SessionRecord = {
      github_id: 4242,
      login: 'octocat',
      avatar_url: 'https://avatars.example/octocat.png',
      email: null,
      created_at: '2026-05-15T00:00:00.000Z',
    };
    kvSessions._store.set(token, JSON.stringify(record));

    const env = makeEnv(kvSessions);
    const req = new Request('https://api.labwired.com/v1/auth/me', {
      method: 'GET',
      headers: { Authorization: `Bearer ${token}` },
    });

    const resp = await worker.default.fetch(req, env as any);
    expect(resp.status).toBe(200);
    const body = (await resp.json()) as any;
    expect(body.github_id).toBe(4242);
    expect(body.login).toBe('octocat');
    expect(body.avatar_url).toBe('https://avatars.example/octocat.png');
    expect(body.email).toBeNull();
    expect(body.plan).toBe('free');
  });

  it('returns 401 without Authorization header', async () => {
    const env = makeEnv(makeKvStub());
    const req = new Request('https://api.labwired.com/v1/auth/me', { method: 'GET' });
    const resp = await worker.default.fetch(req, env as any);
    expect(resp.status).toBe(401);
  });

  it('returns 401 for unknown session token', async () => {
    const env = makeEnv(makeKvStub());
    const req = new Request('https://api.labwired.com/v1/auth/me', {
      method: 'GET',
      headers: { Authorization: 'Bearer not_a_real_session' },
    });
    const resp = await worker.default.fetch(req, env as any);
    expect(resp.status).toBe(401);
  });
});
