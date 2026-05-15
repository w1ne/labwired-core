// Stripe checkout.session.completed → workspace creation tests.
// Verifies the price-ID-based plan routing introduced for the Designer tier:
//   • Pro price_id  → plan='pro',      cycles_quota = PRO_CYCLES_QUOTA
//   • Designer price_id → plan='designer', cycles_quota = DESIGNER_CYCLES_QUOTA
//   • Unknown price_id → falls back to 'pro' (no downgrade of paying customers)
//
// We bypass real Stripe signature verification with a vi.mock that returns a
// synthetic Stripe.Event. The session's line_items are inlined on the payload
// so the worker doesn't need to call the live Stripe API.

import { describe, it, expect, vi } from 'vitest';
import type Stripe from 'stripe';
import type { WorkspaceRecord } from '../src/types.js';
import { STRIPE_PRICE_PRO, STRIPE_PRICE_DESIGNER } from '../src/index.js';

let mockEvent: Stripe.Event | null = null;

vi.mock('../src/stripe.js', () => ({
  verifyStripeWebhook: vi.fn(async () => {
    if (!mockEvent) throw new Error('mockEvent not set');
    return mockEvent;
  }),
  getStripeClient: vi.fn(() => {
    throw new Error('getStripeClient should not be reached in tests (line_items are inlined)');
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
    DESIGNER_CYCLES_QUOTA: '10000000',
    ENVIRONMENT: 'test',
    CLERK_JWT_KEY: '',
  };
}

function makeCheckoutSessionEvent(priceId: string | null): Stripe.Event {
  const session = {
    id: 'cs_test_synthetic',
    object: 'checkout.session',
    customer: 'cus_test_synthetic',
    customer_email: null,
    customer_details: { email: 'buyer@example.com' },
    subscription: 'sub_test_synthetic',
    client_reference_id: 'user_test_clerk',
    line_items: priceId
      ? {
          object: 'list',
          data: [
            {
              id: 'li_test',
              price: { id: priceId, object: 'price' },
            },
          ],
        }
      : undefined,
  } as unknown as Stripe.Checkout.Session;

  return {
    id: 'evt_test_synthetic',
    object: 'event',
    type: 'checkout.session.completed',
    data: { object: session },
  } as unknown as Stripe.Event;
}

async function postSyntheticWebhook(env: ReturnType<typeof makeEnv>) {
  const worker = await import('../src/index.js');
  const req = new Request('https://api.labwired.com/v1/webhooks/stripe', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', 'stripe-signature': 'mocked' },
    body: '{}',
  });
  return worker.default.fetch(req, env as any);
}

function readOnlyWorkspace(kvWorkspaces: KvStub): WorkspaceRecord {
  const entries = Array.from(kvWorkspaces._store.entries());
  expect(entries.length).toBe(1);
  return JSON.parse(entries[0][1]) as WorkspaceRecord;
}

describe('Stripe checkout.session.completed → workspace creation', () => {
  it('creates a Pro workspace (plan=pro, 100M cycles) when the Pro price is bought', async () => {
    mockEvent = makeCheckoutSessionEvent(STRIPE_PRICE_PRO);
    const kvKeys = makeKvStub();
    const kvWorkspaces = makeKvStub();
    const kvSubs = makeKvStub();
    const kvClerk = makeKvStub();
    const env = makeEnv({ kvKeys, kvWorkspaces, kvSubs, kvClerk });

    const resp = await postSyntheticWebhook(env);
    expect(resp.status).toBe(200);

    const ws = readOnlyWorkspace(kvWorkspaces);
    expect(ws.plan).toBe('pro');
    expect(ws.cycles_quota_per_month).toBe(100_000_000);
    expect(ws.customer_email).toBe('buyer@example.com');
    expect(ws.stripe_subscription_id).toBe('sub_test_synthetic');
    expect(ws.clerk_user_id).toBe('user_test_clerk');
    expect(ws.status).toBe('active');
  });

  it('creates a Designer workspace (plan=designer, 10M cycles) when the Designer price is bought', async () => {
    mockEvent = makeCheckoutSessionEvent(STRIPE_PRICE_DESIGNER);
    const kvKeys = makeKvStub();
    const kvWorkspaces = makeKvStub();
    const kvSubs = makeKvStub();
    const kvClerk = makeKvStub();
    const env = makeEnv({ kvKeys, kvWorkspaces, kvSubs, kvClerk });

    const resp = await postSyntheticWebhook(env);
    expect(resp.status).toBe(200);

    const ws = readOnlyWorkspace(kvWorkspaces);
    expect(ws.plan).toBe('designer');
    expect(ws.cycles_quota_per_month).toBe(10_000_000);
    expect(ws.customer_email).toBe('buyer@example.com');
    expect(ws.status).toBe('active');
  });

  it('falls back to Pro for an unrecognized price (so we never downgrade paying customers)', async () => {
    mockEvent = makeCheckoutSessionEvent('price_unknown_sku');
    const kvKeys = makeKvStub();
    const kvWorkspaces = makeKvStub();
    const env = makeEnv({ kvKeys, kvWorkspaces });

    const resp = await postSyntheticWebhook(env);
    expect(resp.status).toBe(200);

    const ws = readOnlyWorkspace(kvWorkspaces);
    expect(ws.plan).toBe('pro');
    expect(ws.cycles_quota_per_month).toBe(100_000_000);
  });
});
