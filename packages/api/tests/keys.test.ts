// Tests for key generation and KV helpers
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { generateApiKey, generateWorkspaceId } from '../src/keys.js';

// ── Unit tests for pure functions ─────────────────────────────────────────

describe('generateApiKey', () => {
  it('starts with lwk_live_ prefix', () => {
    const key = generateApiKey();
    expect(key.startsWith('lwk_live_')).toBe(true);
  });

  it('body is exactly 32 characters', () => {
    const key = generateApiKey();
    const body = key.slice('lwk_live_'.length);
    expect(body.length).toBe(32);
  });

  it('body contains only base32 characters', () => {
    const key = generateApiKey();
    const body = key.slice('lwk_live_'.length);
    expect(/^[A-Z2-7]+$/.test(body)).toBe(true);
  });

  it('produces unique keys', () => {
    const keys = new Set(Array.from({ length: 100 }, () => generateApiKey()));
    expect(keys.size).toBe(100);
  });
});

describe('generateWorkspaceId', () => {
  it('starts with ws_ prefix', () => {
    const id = generateWorkspaceId();
    expect(id.startsWith('ws_')).toBe(true);
  });

  it('has 16 hex chars after prefix', () => {
    const id = generateWorkspaceId();
    const hex = id.slice('ws_'.length);
    expect(hex.length).toBe(16);
    expect(/^[0-9a-f]+$/.test(hex)).toBe(true);
  });

  it('produces unique IDs', () => {
    const ids = new Set(Array.from({ length: 100 }, () => generateWorkspaceId()));
    expect(ids.size).toBe(100);
  });
});

// ── KV helper tests with stub ──────────────────────────────────────────────

function makeKvStub(): KVNamespace {
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
    list: vi.fn(() => Promise.resolve({ keys: [], list_complete: true, cursor: undefined })),
    getWithMetadata: vi.fn(),
  } as unknown as KVNamespace;
}

function makeEnv(kvKeys?: KVNamespace, kvWorkspaces?: KVNamespace, kvSubs?: KVNamespace) {
  return {
    KV_KEYS: kvKeys ?? makeKvStub(),
    KV_WORKSPACES: kvWorkspaces ?? makeKvStub(),
    KV_STRIPE_SUBS: kvSubs ?? makeKvStub(),
    KV_CLERK_TO_WORKSPACE: makeKvStub(),
    STRIPE_SECRET_KEY: 'sk_test_placeholder',
    STRIPE_WEBHOOK_SECRET: 'whsec_placeholder',
    PRO_CYCLES_QUOTA: '100000000',
    ENVIRONMENT: 'test',
  };
}

describe('writeKeyRecord / getKeyRecord', async () => {
  // Import dynamically to avoid top-level module issues in test env
  const { writeKeyRecord, getKeyRecord, updateKeyStatus } = await import('../src/keys.js');

  it('round-trips a key record', async () => {
    const env = makeEnv();
    const key = generateApiKey();
    const wsId = generateWorkspaceId();
    await writeKeyRecord(env as any, key, wsId);
    const record = await getKeyRecord(env as any, key);
    expect(record).not.toBeNull();
    expect(record!.workspace_id).toBe(wsId);
    expect(record!.status).toBe('active');
  });

  it('returns null for unknown key', async () => {
    const env = makeEnv();
    const record = await getKeyRecord(env as any, 'lwk_live_UNKNOWNKEY12345678901234567890');
    expect(record).toBeNull();
  });

  it('updates key status', async () => {
    const env = makeEnv();
    const key = generateApiKey();
    const wsId = generateWorkspaceId();
    await writeKeyRecord(env as any, key, wsId);
    await updateKeyStatus(env as any, key, 'canceled');
    const record = await getKeyRecord(env as any, key);
    expect(record!.status).toBe('canceled');
  });
});
