# Wiring Kernel A: Foundation (schema v2, nets, catalog) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Diagram schema v2 with first-class named nets, lossless v1 migration, a deterministic wire→net normalizer, and a typed-pin part catalog — the data layer Plans B (ERC) and C (compiler/surfaces) build on.

**Architecture:** Everything lands in `packages/board-config` (pure TS, source-shipped, vitest). New modules `schema.ts`, `normalize.ts`, `catalog.ts`; additive extension of `pin-mapping.ts`. Existing exports (`COMPONENT_META`, `Diagram` v1 types) keep working — v1 stays accepted forever via migration.

**Tech Stack:** TypeScript ~5.9, vitest ~3.2. No runtime dependencies (Cloudflare-worker bundleable).

**Spec:** `docs/superpowers/specs/2026-06-12-wiring-kernel-slice2-design.md` (Sections 1–3)

**CRITICAL — workspace:** ALL work happens in the worktree `/home/andrii/projects/labwired/.worktrees/feat-wiring-kernel-slice2` on branch `feat/wiring-kernel-slice2`. Another agent is working in the main checkout `/home/andrii/projects/labwired` and its `core/` — NEVER touch those paths. Commit rule: no Claude/AI/assistant references, no Co-Authored-By.

---

### Task 1: Schema v2 types, version detection, v1→v2 migration

**Files:**
- Create: `packages/board-config/src/schema.ts`
- Test: `packages/board-config/src/schema.test.ts`

- [ ] **Step 1: Write the failing tests**

`packages/board-config/src/schema.test.ts`:

```typescript
import { describe, expect, it } from 'vitest';
import { migrateToV2, parsePinRef, type DiagramV2 } from './schema';
import type { Diagram } from './types';

describe('parsePinRef', () => {
  it('parses part:pin', () => {
    expect(parsePinRef('esp1:GPIO8')).toEqual({ part: 'esp1', pin: 'GPIO8' });
  });
  it('keeps .N disambiguation suffix as part of the pin name', () => {
    expect(parsePinRef('esp1:GND.2')).toEqual({ part: 'esp1', pin: 'GND.2' });
  });
  it('returns null on malformed refs', () => {
    expect(parsePinRef('no-colon')).toBeNull();
    expect(parsePinRef(':pin')).toBeNull();
    expect(parsePinRef('part:')).toBeNull();
  });
});

describe('migrateToV2', () => {
  const v1: Diagram = {
    board: 'esp32-s3-zero',
    parts: [
      { id: 'led1', type: 'led' },
      { id: 'pca1', type: 'pca9685', attrs: { i2c_address: '0x40' } },
    ],
    wires: [
      { from: { part: 'mcu', pin: 'GPIO8' }, to: { part: 'pca1', pin: 'SDA' } },
    ],
  };

  it('wraps a v1 diagram losslessly: parts, board, wires preserved', () => {
    const v2 = migrateToV2(v1);
    expect(v2.version).toBe(2);
    expect(v2.board).toBe('esp32-s3-zero');
    expect(v2.parts).toEqual(v1.parts);
    expect(v2.nets).toEqual([]);
    expect(v2.connections).toEqual([]);
    expect(v2.wires).toEqual(v1.wires);
  });

  it('passes a v2 diagram through unchanged (same object content)', () => {
    const v2In: DiagramV2 = {
      version: 2,
      board: 'esp32-s3-zero',
      parts: [{ id: 'pca1', type: 'pca9685' }],
      nets: [{ name: '3V3', kind: 'power', voltage: 3.3 }],
      connections: [['pca1:VCC', '3V3']],
      wires: [],
    };
    expect(migrateToV2(v2In)).toEqual(v2In);
  });

  it('treats versionless input as v1', () => {
    const versionless = { ...v1 } as Record<string, unknown>;
    delete versionless.version;
    const v2 = migrateToV2(versionless as unknown as Diagram);
    expect(v2.version).toBe(2);
  });

  it('does not mutate its input', () => {
    const frozen = Object.freeze({ ...v1, parts: Object.freeze([...v1.parts]) });
    expect(() => migrateToV2(frozen as Diagram)).not.toThrow();
  });
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /home/andrii/projects/labwired/.worktrees/feat-wiring-kernel-slice2/packages/board-config && npx vitest run src/schema.test.ts`
Expected: FAIL — `./schema` module not found.

- [ ] **Step 3: Implement schema.ts**

```typescript
// Diagram schema v2: first-class named nets with point-to-point wires kept
// as accepted legacy sugar. v1 diagrams (and versionless input) migrate
// losslessly; validation and compilation operate on v2 only.

import type { Diagram, Part, Wire } from './types';

/** Kind of a declared net. */
export type NetKind = 'signal' | 'power';

/** Protocol meaning attached to a signal net. */
export type NetProtocol =
  | 'i2c_sda' | 'i2c_scl'
  | 'spi_mosi' | 'spi_miso' | 'spi_sck' | 'spi_cs'
  | 'uart_tx' | 'uart_rx'
  | 'pwm' | 'adc' | 'gpio' | 'irq';

/** A first-class named net. */
export interface NetDecl {
  name: string;
  kind: NetKind;
  /** Rail voltage in volts; meaningful when kind === 'power'. */
  voltage?: number;
  /** Protocol role of the net; meaningful when kind === 'signal'. */
  protocol?: NetProtocol;
}

/** A connection binds "partId:pinName" to a declared net name. */
export type Connection = [pinRef: string, netName: string];

/** Diagram schema v2. `wires` carries accepted v1 point-to-point sugar. */
export interface DiagramV2 {
  version: 2;
  board: string;
  parts: Part[];
  nets: NetDecl[];
  connections: Connection[];
  wires: Wire[];
}

/** A parsed "part:pin" reference. The pin segment may carry a ".N" suffix. */
export interface PinRef {
  part: string;
  pin: string;
}

/** Parse "partId:pinName" (pin may include a ".N" suffix). Null if malformed. */
export function parsePinRef(ref: string): PinRef | null {
  const idx = ref.indexOf(':');
  if (idx <= 0 || idx === ref.length - 1) return null;
  return { part: ref.slice(0, idx), pin: ref.slice(idx + 1) };
}

/**
 * Migrate any accepted diagram input to v2. v1 (or versionless) diagrams
 * wrap losslessly: parts/board/wires preserved, empty nets/connections.
 * v2 input passes through. Never mutates its input.
 */
export function migrateToV2(input: Diagram | DiagramV2): DiagramV2 {
  if ('version' in input && (input as DiagramV2).version === 2) {
    const v2 = input as DiagramV2;
    return {
      version: 2,
      board: v2.board,
      parts: [...v2.parts],
      nets: [...(v2.nets ?? [])],
      connections: [...(v2.connections ?? [])],
      wires: [...(v2.wires ?? [])],
    };
  }
  const v1 = input as Diagram;
  return {
    version: 2,
    board: v1.board,
    parts: [...v1.parts],
    nets: [],
    connections: [],
    wires: [...(v1.wires ?? [])],
  };
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `npx vitest run src/schema.test.ts`
Expected: PASS (8 tests).

- [ ] **Step 5: Commit**

```bash
git add packages/board-config/src/schema.ts packages/board-config/src/schema.test.ts
git commit -m "feat(board-config): diagram schema v2 — named nets, pin refs, lossless v1 migration"
```

---

### Task 2: Deterministic net normalizer

**Files:**
- Create: `packages/board-config/src/normalize.ts`
- Test: `packages/board-config/src/normalize.test.ts`

- [ ] **Step 1: Write the failing tests**

`packages/board-config/src/normalize.test.ts`:

```typescript
import { describe, expect, it } from 'vitest';
import { resolveNets, type ResolvedNet } from './normalize';
import type { DiagramV2 } from './schema';

function base(over: Partial<DiagramV2>): DiagramV2 {
  return {
    version: 2,
    board: 'esp32-s3-zero',
    parts: [],
    nets: [],
    connections: [],
    wires: [],
    ...over,
  };
}

describe('resolveNets', () => {
  it('resolves declared nets with their members', () => {
    const d = base({
      nets: [{ name: 'I2C0_SDA', kind: 'signal', protocol: 'i2c_sda' }],
      connections: [
        ['mcu:GPIO8', 'I2C0_SDA'],
        ['pca1:SDA', 'I2C0_SDA'],
      ],
    });
    const nets = resolveNets(d);
    expect(nets).toHaveLength(1);
    expect(nets[0]).toEqual<ResolvedNet>({
      name: 'I2C0_SDA',
      kind: 'signal',
      protocol: 'i2c_sda',
      voltage: undefined,
      declared: true,
      members: [
        { part: 'mcu', pin: 'GPIO8' },
        { part: 'pca1', pin: 'SDA' },
      ],
    });
  });

  it('folds legacy wires into synthetic nets via transitive closure', () => {
    const d = base({
      wires: [
        { from: { part: 'a', pin: '1' }, to: { part: 'b', pin: '2' } },
        { from: { part: 'b', pin: '2' }, to: { part: 'c', pin: '3' } },
        { from: { part: 'x', pin: '9' }, to: { part: 'y', pin: '8' } },
      ],
    });
    const nets = resolveNets(d);
    expect(nets).toHaveLength(2);
    const abc = nets.find((n) => n.members.some((m) => m.part === 'a'))!;
    expect(abc.members).toEqual([
      { part: 'a', pin: '1' },
      { part: 'b', pin: '2' },
      { part: 'c', pin: '3' },
    ]);
    expect(abc.declared).toBe(false);
    // Synthetic name derives from the lexicographically smallest member.
    expect(abc.name).toBe('net@a:1');
  });

  it('merges a wire touching a declared net into that net', () => {
    const d = base({
      nets: [{ name: 'GND', kind: 'power', voltage: 0 }],
      connections: [['mcu:GND', 'GND']],
      wires: [{ from: { part: 'mcu', pin: 'GND' }, to: { part: 'led1', pin: 'C' } }],
    });
    const nets = resolveNets(d);
    expect(nets).toHaveLength(1);
    expect(nets[0].name).toBe('GND');
    expect(nets[0].members).toEqual([
      { part: 'led1', pin: 'C' },
      { part: 'mcu', pin: 'GND' },
    ]);
  });

  it('is deterministic: shuffled input order yields identical output', () => {
    const wires = [
      { from: { part: 'a', pin: '1' }, to: { part: 'b', pin: '2' } },
      { from: { part: 'b', pin: '2' }, to: { part: 'c', pin: '3' } },
      { from: { part: 'd', pin: '4' }, to: { part: 'a', pin: '1' } },
    ];
    const a = resolveNets(base({ wires }));
    const b = resolveNets(base({ wires: [...wires].reverse() }));
    expect(a).toEqual(b);
  });

  it('errors are not its job: unknown parts pass through (ERC judges them)', () => {
    const d = base({ wires: [{ from: { part: 'ghost', pin: '1' }, to: { part: 'g2', pin: '2' } }] });
    expect(resolveNets(d)).toHaveLength(1);
  });

  it('two declared nets bridged by wires stay distinct nets but share members', () => {
    // Bridging declared nets is an ERC matter (NET_RAIL_SHORT), not a merge:
    // resolveNets must NOT silently union two declared nets.
    const d = base({
      nets: [
        { name: 'A', kind: 'signal' },
        { name: 'B', kind: 'signal' },
      ],
      connections: [
        ['p:1', 'A'],
        ['p:1', 'B'],
      ],
    });
    const nets = resolveNets(d);
    expect(nets.map((n) => n.name).sort()).toEqual(['A', 'B']);
  });
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `npx vitest run src/normalize.test.ts`
Expected: FAIL — `./normalize` not found.

- [ ] **Step 3: Implement normalize.ts**

```typescript
// Deterministic wires→nets resolution. Declared nets are authoritative and
// are never merged with each other; legacy point-to-point wires union into
// the declared net they touch, or into synthetic nets named after their
// lexicographically smallest member ("net@part:pin"). Same input always
// produces the same output regardless of array order.

import type { DiagramV2, NetKind, NetProtocol, PinRef } from './schema';
import { parsePinRef } from './schema';

/** A net after resolution: declaration metadata plus its member pins. */
export interface ResolvedNet {
  name: string;
  kind: NetKind;
  voltage: number | undefined;
  protocol: NetProtocol | undefined;
  /** True when the net was declared in `nets`; false for wire-synthesized. */
  declared: boolean;
  /** Member pins, sorted by "part:pin" for determinism. */
  members: PinRef[];
}

const key = (m: PinRef) => `${m.part}:${m.pin}`;

/** Resolve declared nets + legacy wires into the canonical net set. */
export function resolveNets(diagram: DiagramV2): ResolvedNet[] {
  // Union-find over pin keys, seeded so that pins bound to a declared net
  // belong to that net's component and components of two declared nets are
  // never merged (bridges are preserved as shared members instead).
  const parent = new Map<string, string>();
  const find = (k: string): string => {
    let p = parent.get(k) ?? k;
    if (p !== k) {
      p = find(p);
      parent.set(k, p);
    }
    return p;
  };
  const union = (a: string, b: string) => {
    const ra = find(a);
    const rb = find(b);
    if (ra === rb) return;
    // Deterministic root: lexicographically smaller key wins.
    if (ra < rb) parent.set(rb, ra);
    else parent.set(ra, rb);
  };

  // Declared memberships: net name -> sorted unique member set.
  const declaredMembers = new Map<string, Map<string, PinRef>>();
  for (const net of diagram.nets) declaredMembers.set(net.name, new Map());
  for (const [ref, netName] of diagram.connections) {
    const pin = parsePinRef(ref);
    if (!pin) continue; // malformed refs are ERC's to report (Plan B)
    declaredMembers.get(netName)?.set(key(pin), pin);
  }

  // Wire closure over pins NOT bound to any declared net; wires touching a
  // declared-net pin attach their other endpoint to that net.
  const declaredPinToNet = new Map<string, string>();
  for (const [name, members] of declaredMembers) {
    for (const k of members.keys()) {
      // A pin connected to several declared nets keeps all memberships
      // (bridge case); first net wins for wire-attachment purposes.
      if (!declaredPinToNet.has(k)) declaredPinToNet.set(k, name);
    }
  }

  const wirePins = new Map<string, PinRef>();
  for (const w of diagram.wires) {
    const a = w.from;
    const b = w.to;
    const ka = key(a);
    const kb = key(b);
    const netA = declaredPinToNet.get(ka);
    const netB = declaredPinToNet.get(kb);
    if (netA && netB) continue; // both ends declared: nothing to synthesize
    if (netA) {
      declaredMembers.get(netA)!.set(kb, b);
      declaredPinToNet.set(kb, netA);
      continue;
    }
    if (netB) {
      declaredMembers.get(netB)!.set(ka, a);
      declaredPinToNet.set(ka, netB);
      continue;
    }
    wirePins.set(ka, a);
    wirePins.set(kb, b);
    union(ka, kb);
  }

  const out: ResolvedNet[] = [];
  for (const net of diagram.nets) {
    const members = [...declaredMembers.get(net.name)!.values()].sort((x, y) =>
      key(x).localeCompare(key(y)),
    );
    out.push({
      name: net.name,
      kind: net.kind,
      voltage: net.voltage,
      protocol: net.protocol,
      declared: true,
      members,
    });
  }

  // Group synthetic components.
  const groups = new Map<string, PinRef[]>();
  for (const [k, pin] of wirePins) {
    const root = find(k);
    const g = groups.get(root) ?? [];
    g.push(pin);
    groups.set(root, g);
  }
  const synthetic = [...groups.values()]
    .map((members) => members.sort((x, y) => key(x).localeCompare(key(y))))
    .sort((a, b) => key(a[0]).localeCompare(key(b[0])))
    .map<ResolvedNet>((members) => ({
      name: `net@${key(members[0])}`,
      kind: 'signal',
      voltage: undefined,
      protocol: undefined,
      declared: false,
      members,
    }));

  return [...out, ...synthetic];
}
```

Note on the wire-attach loop: a chain of wires where only a later wire touches a declared net can leave earlier links in the synthetic pool (single-pass attachment). Make the wire pass iterate until a fixpoint: wrap the `for (const w of diagram.wires)` loop in `let changed = true; while (changed) { changed = false; ... set changed = true whenever a declared attachment happens and remove that wire from the pending list ... }`, with remaining pending wires falling through to union(). Implement this fixpoint — the test in Step 1 ("merges a wire touching a declared net") plus a chain variant you should add (`mcu:GND—led1:C` then `led1:C—r1:2`: both led1:C and r1:2 must end in GND) pin it. Add that chain test:

```typescript
  it('attaches whole wire chains to a declared net (fixpoint, any order)', () => {
    const d = base({
      nets: [{ name: 'GND', kind: 'power', voltage: 0 }],
      connections: [['mcu:GND', 'GND']],
      wires: [
        { from: { part: 'r1', pin: '2' }, to: { part: 'led1', pin: 'C' } },
        { from: { part: 'led1', pin: 'C' }, to: { part: 'mcu', pin: 'GND' } },
      ],
    });
    const nets = resolveNets(d);
    expect(nets).toHaveLength(1);
    expect(nets[0].members.map((m) => `${m.part}:${m.pin}`)).toEqual([
      'led1:C',
      'mcu:GND',
      'r1:2',
    ]);
  });
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `npx vitest run src/normalize.test.ts`
Expected: PASS (8 tests).

- [ ] **Step 5: Commit**

```bash
git add packages/board-config/src/normalize.ts packages/board-config/src/normalize.test.ts
git commit -m "feat(board-config): deterministic net resolution (declared nets + wire closure)"
```

---

### Task 3: Pin electrical types + part catalog

**Files:**
- Create: `packages/board-config/src/catalog.ts`
- Modify: `packages/board-config/src/component-meta.ts` (re-export shim)
- Test: `packages/board-config/src/catalog.test.ts`

- [ ] **Step 1: Write the failing tests**

`packages/board-config/src/catalog.test.ts`:

```typescript
import { describe, expect, it } from 'vitest';
import { CATALOG, getCatalogPart, type PinDecl } from './catalog';
import { COMPONENT_META } from './component-meta';

describe('catalog', () => {
  it('every legacy COMPONENT_META key exists in the catalog with the same boardIoKind', () => {
    for (const [type, meta] of Object.entries(COMPONENT_META)) {
      const part = getCatalogPart(type);
      expect(part, `catalog missing legacy type '${type}'`).toBeDefined();
      expect(part!.boardIoKind).toEqual(meta.boardIoKind);
    }
  });

  it('pca9685 declares typed pins including open-drain I2C and 16 outputs', () => {
    const pins = getCatalogPart('pca9685')!.pins!;
    const byName = Object.fromEntries(pins.map((p) => [p.name, p]));
    expect(byName.SDA).toEqual<PinDecl>({ name: 'SDA', etype: 'open_drain', role: 'i2c_sda' });
    expect(byName.SCL).toEqual<PinDecl>({ name: 'SCL', etype: 'open_drain', role: 'i2c_scl' });
    expect(byName.VCC.etype).toBe('power_in');
    expect(byName.GND.etype).toBe('power_in');
    expect(pins.filter((p) => p.name.startsWith('LED')).length).toBe(16);
  });

  it('resistor pins are passive; led pins are passive; button pins are passive', () => {
    for (const type of ['resistor', 'led', 'button']) {
      const part = getCatalogPart(type);
      expect(part?.pins?.every((p) => p.etype === 'passive'), type).toBe(true);
    }
  });

  it('bme280 declares operating voltage range', () => {
    expect(getCatalogPart('bme280')!.operatingVoltage).toEqual({ min: 1.71, max: 3.6 });
  });

  it('parts without pin declarations are explicitly legacy (pins undefined)', () => {
    // Incremental adoption: ERC pin rules only run where pins are declared.
    expect(getCatalogPart('keypad')!.pins).toBeUndefined();
  });

  it('unknown type returns undefined', () => {
    expect(getCatalogPart('definitely-not-a-part')).toBeUndefined();
  });
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `npx vitest run src/catalog.test.ts`
Expected: FAIL — `./catalog` not found.

- [ ] **Step 3: Implement catalog.ts**

First READ the current `src/component-meta.ts` in full — the catalog must cover every key it has (the test in Step 1 enforces this). Then:

```typescript
// The single declarative part catalog: every part type's device class,
// board_io mapping, and (incrementally) typed pin declarations. Replaces
// the metadata previously split across component-meta.ts here, the copy in
// packages/mcp, and the UI registry. Parts without `pins` are legacy:
// existing diagnostics still apply to them, pin-pair ERC does not (yet).

import type { BoardIoKind } from './types';
import type { NetProtocol } from './schema';

/** KiCad-vocabulary pin electrical types. */
export type PinEtype =
  | 'input' | 'output' | 'bidirectional' | 'tri_state' | 'passive'
  | 'open_drain' | 'open_emitter' | 'power_in' | 'power_out'
  | 'nc' | 'unspecified' | 'not_internally_connected';

/** A declared part pin. */
export interface PinDecl {
  name: string;
  etype: PinEtype;
  /** Protocol meaning, when the pin has one. */
  role?: NetProtocol;
  /** Pin must be on a net (floating-input ERC, Plan B). */
  required?: boolean;
}

/** A catalog entry for one part type. */
export interface CatalogPart {
  type: string;
  /** Legacy board_io mapping (same meaning as COMPONENT_META). */
  boardIoKind?: BoardIoKind;
  /** Typed pins; undefined = legacy part, pin-level ERC skipped. */
  pins?: PinDecl[];
  /** Supply range in volts for PWR_VOLTAGE_MISMATCH (Plan B). */
  operatingVoltage?: { min: number; max: number };
}

const p = (name: string, etype: PinEtype, role?: NetProtocol, required?: boolean): PinDecl =>
  required ? { name, etype, role, required } : role ? { name, etype, role } : { name, etype };

const pca9685Pins: PinDecl[] = [
  p('VCC', 'power_in'),
  p('GND', 'power_in'),
  p('SDA', 'open_drain', 'i2c_sda'),
  p('SCL', 'open_drain', 'i2c_scl'),
  p('OE', 'input'),
  ...Array.from({ length: 16 }, (_, i) => p(`LED${i}`, 'output', 'pwm')),
];

export const CATALOG: Record<string, CatalogPart> = {
  // --- MCU boards: pins come from PIN_MAPS, not the catalog ---
  // (One entry per MCU key currently in COMPONENT_META, e.g.:)
  mcu: { type: 'mcu' },
  'arduino-uno': { type: 'arduino-uno' },
  // ... every other MCU key from component-meta.ts, verbatim ...

  // --- Typed parts (initial set; grows incrementally) ---
  led: {
    type: 'led',
    boardIoKind: 'led',
    pins: [p('A', 'passive'), p('C', 'passive')],
  },
  button: {
    type: 'button',
    boardIoKind: 'button',
    pins: [p('1', 'passive'), p('2', 'passive')],
  },
  resistor: {
    type: 'resistor',
    pins: [p('1', 'passive'), p('2', 'passive')],
  },
  servo: {
    type: 'servo',
    boardIoKind: 'pwm_output',
    pins: [p('PWM', 'input', 'pwm', true), p('VCC', 'power_in'), p('GND', 'power_in')],
    operatingVoltage: { min: 4.8, max: 6.0 },
  },
  pca9685: {
    type: 'pca9685',
    boardIoKind: 'i2c_device',
    pins: pca9685Pins,
    operatingVoltage: { min: 2.3, max: 5.5 },
  },
  bme280: {
    type: 'bme280',
    boardIoKind: 'i2c_device',
    pins: [
      p('VCC', 'power_in'),
      p('GND', 'power_in'),
      p('SDA', 'open_drain', 'i2c_sda'),
      p('SCL', 'open_drain', 'i2c_scl'),
    ],
    operatingVoltage: { min: 1.71, max: 3.6 },
  },
  ultrasonic: {
    type: 'ultrasonic',
    boardIoKind: 'button',
    pins: [
      p('VCC', 'power_in'),
      p('GND', 'power_in'),
      p('TRIG', 'input', 'gpio', true),
      p('ECHO', 'output', 'gpio'),
    ],
    operatingVoltage: { min: 4.5, max: 5.5 },
  },

  // --- Legacy parts: boardIoKind carried over, no pins yet ---
  // (Every remaining COMPONENT_META key, verbatim boardIoKind, e.g.:)
  keypad: { type: 'keypad', boardIoKind: 'button' },
  // ... etc ...
};

/** Look up a catalog part by diagram part type. */
export function getCatalogPart(type: string): CatalogPart | undefined {
  return CATALOG[type];
}
```

The `// ... etc ...` markers above mean: enumerate EVERY key from the current `component-meta.ts` (read it; ~40 keys), carrying `boardIoKind` verbatim. If `pca9685` is not currently a COMPONENT_META key (check!), add it as a new catalog entry anyway — the Step 1 test only requires the legacy direction (META key → catalog), not the reverse. Same for `resistor`.

Then convert `component-meta.ts` to a derivation shim (public API unchanged):

```typescript
import type { BoardIoKind } from './types';
import { CATALOG } from './catalog';

export interface ComponentMeta { boardIoKind?: BoardIoKind; }

/** Derived from the catalog; kept for backward compatibility. */
export const COMPONENT_META: Record<string, ComponentMeta> = Object.fromEntries(
  Object.entries(CATALOG).map(([k, v]) => [
    k,
    v.boardIoKind ? { boardIoKind: v.boardIoKind } : {},
  ]),
);
```

- [ ] **Step 4: Run tests — new and existing**

Run: `npx vitest run`
Expected: catalog tests PASS and every pre-existing board-config test still PASSES (the COMPONENT_META shim must be behavior-identical; if any existing test snapshot compares the object, fix the catalog data, not the test).

- [ ] **Step 5: Commit**

```bash
git add packages/board-config/src/catalog.ts packages/board-config/src/catalog.test.ts packages/board-config/src/component-meta.ts
git commit -m "feat(board-config): typed-pin part catalog; COMPONENT_META derived from it"
```

---

### Task 4: Pin-map electrical extension

**Files:**
- Modify: `packages/board-config/src/pin-mapping.ts`
- Test: `packages/board-config/src/pin-mapping-etype.test.ts`

- [ ] **Step 1: Read, then write the failing tests**

READ `src/pin-mapping.ts` in full first (interfaces `PinFunction`/`PinMapping` at lines ~6–16, `PIN_MAPS` at ~333). Then `packages/board-config/src/pin-mapping-etype.test.ts`:

```typescript
import { describe, expect, it } from 'vitest';
import { getPinEtype, getPinMapping, PIN_MAPS } from './pin-mapping';

describe('pin map electrical extension', () => {
  it('esp32-s3-zero GPIO pins are bidirectional with internal pullups', () => {
    expect(getPinEtype('esp32-s3-zero', 'GPIO8')).toEqual({
      etype: 'bidirectional',
      internalPullup: true,
    });
  });

  it('power pins carry power_out etype', () => {
    expect(getPinEtype('esp32-s3-zero', '3V3')).toEqual({
      etype: 'power_out',
      internalPullup: false,
    });
    expect(getPinEtype('esp32-s3-zero', 'GND')).toEqual({
      etype: 'power_out',
      internalPullup: false,
    });
  });

  it('unknown pin or board returns null', () => {
    expect(getPinEtype('esp32-s3-zero', 'NOPE')).toBeNull();
    expect(getPinEtype('not-a-board', 'GPIO8')).toBeNull();
  });

  it('every board in PIN_MAPS resolves every mapped pin to an etype (default bidirectional)', () => {
    for (const board of Object.keys(PIN_MAPS)) {
      for (const pin of Object.keys(PIN_MAPS[board])) {
        expect(getPinEtype(board, pin), `${board}:${pin}`).not.toBeNull();
      }
    }
  });

  it('existing lookups unchanged', () => {
    // Regression guard: extension must not break the legacy surface.
    expect(getPinMapping('esp32-s3-zero', 'GPIO8')).not.toBeNull();
  });
});
```

Adapt the literal pin names ('GPIO8', '3V3', 'GND') to whatever the actual `PIN_MAPS['esp32-s3-zero']` keys are — read them first; if the map lacks power pins entirely, ADD `3V3` and `GND` entries to it as part of this task (they are real pins on the board).

- [ ] **Step 2: Run tests to verify they fail**

Run: `npx vitest run src/pin-mapping-etype.test.ts`
Expected: FAIL — `getPinEtype` not exported.

- [ ] **Step 3: Implement the extension (additive)**

In `pin-mapping.ts`, add — without changing any existing export's shape:

```typescript
import type { PinEtype } from './catalog';

/** Electrical info for an MCU pin. */
export interface PinElectrical {
  etype: PinEtype;
  internalPullup: boolean;
}

/** Per-board overrides; pins absent here default by rule below. */
const PIN_ELECTRICAL_OVERRIDES: Record<string, Record<string, PinElectrical>> = {
  'esp32-s3-zero': {
    '3V3': { etype: 'power_out', internalPullup: false },
    '5V': { etype: 'power_out', internalPullup: false },
    GND: { etype: 'power_out', internalPullup: false },
  },
  // Other boards get the defaults until extended.
};

/**
 * Electrical type of an MCU pin. Defaults: any mapped GPIO-capable pin is
 * `bidirectional` with `internalPullup: true` (every MCU in PIN_MAPS has
 * per-pin pullups); power pins via overrides. Null for unknown pin/board.
 */
export function getPinEtype(board: string, pinLabel: string): PinElectrical | null {
  const override = PIN_ELECTRICAL_OVERRIDES[board]?.[pinLabel];
  if (override) return override;
  const mapping = getPinMapping(board, pinLabel);
  if (!mapping) return null;
  return { etype: 'bidirectional', internalPullup: true };
}
```

If `PIN_MAPS['esp32-s3-zero']` lacks `3V3`/`GND` keys, add them with an empty/power-appropriate `PinMapping` consistent with the existing entry shape (read neighboring entries and mirror their structure), so the "every mapped pin resolves" test holds.

- [ ] **Step 4: Run all tests**

Run: `npx vitest run`
Expected: PASS (all board-config tests).

- [ ] **Step 5: Commit**

```bash
git add packages/board-config/src/pin-mapping.ts packages/board-config/src/pin-mapping-etype.test.ts
git commit -m "feat(board-config): pin electrical types + internal-pullup capability for MCU pins"
```

---

### Task 5: Public exports, typecheck, package verification

**Files:**
- Modify: `packages/board-config/src/index.ts`

- [ ] **Step 1: Extend exports**

Append to `src/index.ts` (keep all existing exports untouched):

```typescript
export { migrateToV2, parsePinRef } from './schema';
export type { Connection, DiagramV2, NetDecl, NetKind, NetProtocol, PinRef } from './schema';
export { resolveNets } from './normalize';
export type { ResolvedNet } from './normalize';
export { CATALOG, getCatalogPart } from './catalog';
export type { CatalogPart, PinDecl, PinEtype } from './catalog';
export { getPinEtype } from './pin-mapping';
export type { PinElectrical } from './pin-mapping';
```

- [ ] **Step 2: Full package verification**

Run, from `packages/board-config`:
```bash
npx vitest run && npm run typecheck
```
Expected: all tests pass, `tsc --noEmit` clean.

Then verify dependents still build/test — board-config is source-shipped, so its consumers compile it directly:
```bash
cd ../mcp && npm test -- --run 2>&1 | tail -3 && npm run build 2>&1 | tail -1
cd ../ui && npm run typecheck 2>&1 | tail -2 || npx tsc --noEmit 2>&1 | tail -2
cd ../playground && npm run build 2>&1 | tail -2
```
(Use each package's actual script names — check their package.json `scripts`; `packages/mcp` tests shell out to the labwired CLI: set `LABWIRED_CLI` to a prebuilt binary if available in THIS worktree's core only — if no binary exists here, build it once: `cd <worktree>/core && cargo build -p labwired-cli`. NEVER use the main checkout's core.)
Expected: no breakage anywhere — this plan is purely additive plus the behavior-identical COMPONENT_META shim.

- [ ] **Step 3: Commit**

```bash
git add packages/board-config/src/index.ts
git commit -m "feat(board-config): export wiring-kernel v2 surface"
```

---

## Self-review notes

- Spec coverage (Plan A's share): schema v2 + lossless migration (Task 1, spec §2), deterministic normalizer with declared-nets-not-merged semantics (Task 2, spec §2/§4 NET_RAIL_SHORT precondition), typed-pin catalog + COMPONENT_META consolidation (Task 3, spec §1/§3), pin-map electrical extension as canonical source per the 2026-06-12 spec amendment (Task 4, spec §1/§3), exports + dependent verification (Task 5). ERC rules, compiler, MCP/API adapters, IR-component catalog mapping, and fixtures are Plans B/C by design.
- Known check-at-implementation points are marked inline: exact COMPONENT_META key list, actual PIN_MAPS pin names/shape for esp32-s3-zero, dependent packages' script names, mcp's CLI binary in the isolated worktree.
- Type consistency verified: `PinEtype`/`PinDecl` defined in catalog.ts and imported by pin-mapping.ts; `NetProtocol` defined in schema.ts and imported by catalog.ts; `ResolvedNet.members` uses schema's `PinRef`.
