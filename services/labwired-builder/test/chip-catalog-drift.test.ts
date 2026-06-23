// Drift guard: CHIP_YAMLS is the single source of truth for the chip catalog.
// Three agent-facing surfaces derive from it — GET /chips, the resolveChipInManifest
// "unknown chip id" error, and the resolver's accept set. If any one is ever
// reworked to hardcode, filter, or reorder the list, an agent gets a catalog from
// one surface that the others reject. These tests lock all three to CHIP_YAMLS so
// the divergence fails CI instead of shipping.
import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import { makeServer } from '../src/server';
import { resolveChipInManifest } from '../src/run';
import { CHIP_YAMLS } from '../../../packages/board-config/src/chip-yamls';
import type { Server } from 'node:http';
import { AddressInfo } from 'node:net';

const EXPECTED = Object.keys(CHIP_YAMLS).sort();

let server: Server;
let base: string;

beforeAll(() => {
  server = makeServer({ secret: 'drift-test' });
  return new Promise<void>((resolve) => {
    server.listen(0, '127.0.0.1', () => {
      base = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
      resolve();
    });
  });
});

afterAll(() => new Promise<void>((resolve) => server.close(() => resolve())));

describe('chip catalog drift guard', () => {
  it('the catalog is non-empty (an empty CHIP_YAMLS would silently empty every surface)', () => {
    expect(EXPECTED.length).toBeGreaterThan(0);
  });

  it('GET /chips lists EXACTLY the CHIP_YAMLS ids (sorted) — no extras, none missing', async () => {
    const res = await fetch(`${base}/chips`);
    expect(res.status).toBe(200);
    const body = (await res.json()) as { chips: { id: string }[] };
    expect(body.chips.map((c) => c.id)).toEqual(EXPECTED);
  });

  it('the unknown-chip error lists EXACTLY the CHIP_YAMLS ids and points at labwired_lookup', () => {
    let msg = '';
    try {
      resolveChipInManifest('name: "t"\nchip: "definitely-not-a-real-chip"\nboard_io: []\n');
    } catch (e) {
      msg = (e as Error).message;
    }
    expect(msg).toContain(`Known chip ids: ${EXPECTED.join(', ')}.`);
    expect(msg).toContain('labwired_lookup with of:"chips"');
  });

  it('every advertised id actually resolves to its bundled YAML (no id is advertised but unresolvable)', () => {
    for (const id of EXPECTED) {
      const out = resolveChipInManifest(`name: "t"\nchip: "${id}"\nboard_io: []\n`);
      expect(out.chipYaml).toBe(CHIP_YAMLS[id]);
    }
  });
});
