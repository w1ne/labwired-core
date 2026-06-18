// Share preview-image storage + serving tests.
// Per-lab images are stored ONLY for authenticated shares; anonymous/missing
// images fall back (302) to the brand logo. We mock the Clerk verifier so a
// request with an Authorization header counts as signed-in.
import { describe, it, expect, vi } from 'vitest';

vi.mock('../src/clerk.js', () => ({
  verifyClerkRequest: vi.fn(async (req: Request) =>
    req.headers.get('Authorization') ? { userId: 'test-user' } : null,
  ),
}));

const FALLBACK_IMAGE_URL = 'https://app.labwired.com/icon-512.png';

// 1×1 white PNG (valid signature + chunks), base64.
const TINY_PNG_B64 =
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4//8/AAX+Av4N70a4AAAAAElFTkSuQmCC';

function b64ToBytes(b64: string): Uint8Array {
  const binary = atob(b64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

// ── KV stub that stores raw values and supports arrayBuffer reads ───────────
function makeKvStub() {
  const store = new Map<string, unknown>();
  return {
    get: (key: string, type?: string) => {
      const value = store.get(key);
      if (value === undefined) return Promise.resolve(null);
      if (type === 'arrayBuffer') {
        if (value instanceof Uint8Array) {
          return Promise.resolve(value.buffer.slice(value.byteOffset, value.byteOffset + value.byteLength));
        }
        return Promise.resolve(value);
      }
      return Promise.resolve(value);
    },
    put: (key: string, value: unknown) => {
      store.set(key, value);
      return Promise.resolve();
    },
    delete: () => Promise.resolve(),
    list: () => Promise.resolve({ keys: [], list_complete: true }),
    getWithMetadata: () => Promise.resolve({ value: null, metadata: null }),
    _store: store,
  };
}

function makeEnv(kvProjects: ReturnType<typeof makeKvStub>) {
  return {
    KV_KEYS: makeKvStub() as unknown as KVNamespace,
    KV_WORKSPACES: makeKvStub() as unknown as KVNamespace,
    KV_STRIPE_SUBS: makeKvStub() as unknown as KVNamespace,
    KV_CLERK_TO_WORKSPACE: makeKvStub() as unknown as KVNamespace,
    KV_PROJECTS: kvProjects as unknown as KVNamespace,
    STRIPE_SECRET_KEY: 'sk_test_placeholder',
    STRIPE_WEBHOOK_SECRET: 'whsec_placeholder',
    PRO_CYCLES_QUOTA: '100000000',
    ENVIRONMENT: 'test',
    CLERK_PUBLISHABLE_KEY: 'pk_live_Y2xlcmsubGFid2lyZWQuY29tJA',
    MCP_AUTHORIZATION_SERVER: 'https://clerk.labwired.com',
  };
}

const worker = await import('../src/index.js');

const DIAGRAM = {
  board: 'stm32l476',
  parts: [{ id: 'mcu', type: 'mcu' }],
  wires: [],
};

/** Create a share. `authed: true` attaches a bearer token (signed-in user). */
async function createShare(env: unknown, extra: Record<string, unknown>, authed = false) {
  return worker.default.fetch(
    new Request('https://api.labwired.com/v1/shares', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        ...(authed ? { Authorization: 'Bearer test-token' } : {}),
      },
      body: JSON.stringify({ diagram: DIAGRAM, source: 'int main(){}', ...extra }),
    }),
    env as any,
  );
}

function getImage(env: unknown, id: string) {
  return worker.default.fetch(
    new Request(`https://api.labwired.com/v1/shares/${id}/image`, { redirect: 'manual' }),
    env as any,
  );
}

describe('share preview image', () => {
  it('stores a valid PNG for a SIGNED-IN share and serves it (immutable + nosniff)', async () => {
    const kvProjects = makeKvStub();
    const env = makeEnv(kvProjects);

    const create = await createShare(env, { preview: `data:image/png;base64,${TINY_PNG_B64}` }, true);
    expect(create.status).toBe(201);
    const { id } = (await create.json()) as { id: string };

    const stored = kvProjects._store.get(`shareimg:${id}`);
    expect(stored).toBeInstanceOf(Uint8Array);
    expect(Array.from(stored as Uint8Array)).toEqual(Array.from(b64ToBytes(TINY_PNG_B64)));

    const img = await getImage(env, id);
    expect(img.status).toBe(200);
    expect(img.headers.get('Content-Type')).toBe('image/png');
    expect(img.headers.get('X-Content-Type-Options')).toBe('nosniff');
    expect(img.headers.get('Content-Disposition')).toBe('inline');
    expect(img.headers.get('Cache-Control')).toBe('public, max-age=31536000, immutable');
    expect(Array.from(new Uint8Array(await img.arrayBuffer()))).toEqual(Array.from(b64ToBytes(TINY_PNG_B64)));
  });

  it('accepts a raw (non-data-URL) base64 PNG for a signed-in share', async () => {
    const kvProjects = makeKvStub();
    const env = makeEnv(kvProjects);
    const create = await createShare(env, { preview: TINY_PNG_B64 }, true);
    expect(create.status).toBe(201);
    const { id } = (await create.json()) as { id: string };
    expect(kvProjects._store.get(`shareimg:${id}`)).toBeInstanceOf(Uint8Array);
  });

  it('IGNORES a preview from an ANONYMOUS share; image falls back to the logo', async () => {
    const kvProjects = makeKvStub();
    const env = makeEnv(kvProjects);

    // Valid PNG, but no auth → must NOT be stored (this is the abuse gate).
    const create = await createShare(env, { preview: `data:image/png;base64,${TINY_PNG_B64}` }, false);
    expect(create.status).toBe(201);
    const { id } = (await create.json()) as { id: string };

    expect(kvProjects._store.has(`shareimg:${id}`)).toBe(false);

    const img = await getImage(env, id);
    expect(img.status).toBe(302);
    expect(img.headers.get('Location')).toBe(FALLBACK_IMAGE_URL);
    expect(img.headers.get('Cache-Control')).not.toContain('immutable');
  });

  it('skips a non-PNG preview (signed-in) but still creates the share → logo fallback', async () => {
    const kvProjects = makeKvStub();
    const env = makeEnv(kvProjects);
    const create = await createShare(env, { preview: btoa('not a png at all, just text') }, true);
    expect(create.status).toBe(201);
    const { id } = (await create.json()) as { id: string };
    expect(kvProjects._store.has(`shareimg:${id}`)).toBe(false);
    expect((await getImage(env, id)).status).toBe(302);
  });

  it('skips an oversized preview (signed-in) but still creates the share → logo fallback', async () => {
    const kvProjects = makeKvStub();
    const env = makeEnv(kvProjects);
    const big = new Uint8Array(600 * 1024);
    big.set(b64ToBytes(TINY_PNG_B64), 0);
    let binary = '';
    for (let i = 0; i < big.length; i++) binary += String.fromCharCode(big[i]);
    const create = await createShare(env, { preview: btoa(binary) }, true);
    expect(create.status).toBe(201);
    const { id } = (await create.json()) as { id: string };
    expect(kvProjects._store.has(`shareimg:${id}`)).toBe(false);
    expect((await getImage(env, id)).status).toBe(302);
  });

  it('a share with no preview → image endpoint 302s to the logo', async () => {
    const kvProjects = makeKvStub();
    const env = makeEnv(kvProjects);
    const create = await createShare(env, {}, true);
    expect(create.status).toBe(201);
    const { id } = (await create.json()) as { id: string };
    const img = await getImage(env, id);
    expect(img.status).toBe(302);
    expect(img.headers.get('Location')).toBe(FALLBACK_IMAGE_URL);
  });
});
