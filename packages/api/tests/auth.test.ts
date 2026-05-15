// Clerk-backed /v1/auth/me route tests
import { describe, it, expect, vi, beforeEach } from 'vitest';

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

function makeEnv() {
  return {
    KV_KEYS: makeKvStub() as unknown as KVNamespace,
    KV_WORKSPACES: makeKvStub() as unknown as KVNamespace,
    KV_STRIPE_SUBS: makeKvStub() as unknown as KVNamespace,
    STRIPE_SECRET_KEY: 'sk_test_placeholder',
    STRIPE_WEBHOOK_SECRET: 'whsec_placeholder',
    RESEND_API_KEY: '',
    CLERK_SECRET_KEY: 'sk_test_clerk_placeholder',
    FROM_EMAIL: 'onboarding@labwired.com',
    PRO_CYCLES_QUOTA: '100000000',
    ENVIRONMENT: 'test',
    CLERK_JWT_KEY: '-----BEGIN PUBLIC KEY-----\nplaceholder\n-----END PUBLIC KEY-----',
  };
}

const worker = await import('../src/index.js');

describe('GET /v1/auth/me', () => {
  beforeEach(() => {
    mockAuthState = { isAuthenticated: false };
  });

  it('returns user_id, email, and plan=free for an authenticated Clerk session', async () => {
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
  });

  it('returns 401 when Clerk reports unauthenticated', async () => {
    mockAuthState = { isAuthenticated: false };

    const req = new Request('https://api.labwired.com/v1/auth/me', { method: 'GET' });
    const resp = await worker.default.fetch(req, makeEnv() as any);
    expect(resp.status).toBe(401);
  });
});
