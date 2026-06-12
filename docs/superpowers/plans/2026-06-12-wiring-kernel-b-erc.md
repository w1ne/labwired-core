# Wiring Kernel B: ERC Engine Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The electrical-rule-check engine: `erc(diagram) → Diagnostic[]` covering schema integrity, the pin-pair matrix, power rules, and bus rules — every rule with a triggering fixture and its corrected twin.

**Architecture:** New `src/erc/` modules in `packages/board-config` (pure functions over Plan A's `resolveNets` + `CATALOG` + `getPinEtype`). One file per rule family, one shared `Diagnostic` type, one `erc()` entry point that runs all families. Legacy parts (no catalog `pins`) are skipped by pin-level rules — incremental adoption by design.

**Tech Stack:** TypeScript ~5.9, vitest ~3.2, no runtime deps. Tests in `test/` (vitest include pattern), imports from `../src/...`.

**Spec:** `docs/superpowers/specs/2026-06-12-wiring-kernel-slice2-design.md` (Section 4)

**CRITICAL — workspace:** ALL work in `/home/andrii/projects/labwired/.worktrees/feat-wiring-kernel-slice2` (branch `feat/wiring-kernel-slice2`). Another agent works in the main checkout — NEVER touch `/home/andrii/projects/labwired` or its `core/`. Commits: no Claude/AI/assistant references, no Co-Authored-By.

**Carry-overs folded in (from Plan A reviews):** Task 1 adds `deviceClass`; Task 2 adds the undeclared-net diagnostic the normalizer silently tolerates; S3 I2C matrix-routability means capability mismatches stay warnings.

---

### Task 1: `deviceClass` on the catalog

**Files:**
- Modify: `packages/board-config/src/catalog.ts`
- Modify: `packages/board-config/test/catalog.test.ts`

- [ ] **Step 1: Failing tests** — append to `test/catalog.test.ts`:

```typescript
describe('deviceClass', () => {
  it('classifies the key part families', () => {
    expect(getCatalogPart('esp32-s3-zero')!.deviceClass).toBe('mcu');
    expect(getCatalogPart('pca9685')!.deviceClass).toBe('i2c_device');
    expect(getCatalogPart('bme280')!.deviceClass).toBe('i2c_device');
    expect(getCatalogPart('resistor')!.deviceClass).toBe('passive');
    expect(getCatalogPart('led')!.deviceClass).toBe('board_io');
    expect(getCatalogPart('button')!.deviceClass).toBe('board_io');
    expect(getCatalogPart('neo6m-gps')!.deviceClass).toBe('uart_device');
  });
  it('every catalog entry has a deviceClass', () => {
    for (const [type, part] of Object.entries(CATALOG)) {
      expect(part.deviceClass, type).toBeDefined();
    }
  });
});
```

(If `neo6m-gps` is not a catalog key, check the actual key name in CATALOG — the GPS part exists under some name; adapt the literal.)

- [ ] **Step 2: Run to fail**, then implement: add to `catalog.ts`:

```typescript
/** Compiler/ERC dispatch class. */
export type DeviceClass =
  | 'mcu' | 'board_io' | 'i2c_device' | 'spi_device' | 'uart_device' | 'passive';
```

Add `deviceClass: DeviceClass;` (required) to `CatalogPart`. Populate every entry: MCU board keys → `mcu`; entries with `boardIoKind: 'i2c_device'` → `i2c_device`; `spi_device` → `spi_device`; led/button/adc/pwm board-io kinds → `board_io`; resistor/capacitor/diode-style passives → `passive`; UART-attached modules (GPS, modem-style parts — judge per part from its name and original META grouping comments) → `uart_device`. When unsure for a legacy part, `board_io` if it had any boardIoKind, else `passive` for two-terminal primitives, else `board_io`. Export `DeviceClass` from `src/index.ts`.

- [ ] **Step 3: Verify + commit** — `npx vitest run && npm run typecheck` all green.

```bash
git add packages/board-config/src/catalog.ts packages/board-config/test/catalog.test.ts packages/board-config/src/index.ts
git commit -m "feat(board-config): deviceClass on catalog parts for ERC/compiler dispatch"
```

---

### Task 2: Diagnostic type, `erc()` entry point, schema-integrity rules

**Files:**
- Create: `packages/board-config/src/erc/diagnostic.ts`
- Create: `packages/board-config/src/erc/context.ts`
- Create: `packages/board-config/src/erc/schema-rules.ts`
- Create: `packages/board-config/src/erc/index.ts`
- Test: `packages/board-config/test/erc-schema.test.ts`

- [ ] **Step 1: Failing tests** — `test/erc-schema.test.ts`:

```typescript
import { describe, expect, it } from 'vitest';
import { erc } from '../src/erc';
import type { DiagramV2 } from '../src/schema';

const base = (over: Partial<DiagramV2>): DiagramV2 => ({
  version: 2, board: 'esp32-s3-zero',
  parts: [{ id: 'mcu', type: 'esp32-s3-zero' }],
  nets: [], connections: [], wires: [], ...over,
});

const codes = (d: DiagramV2) => erc(d).map((x) => x.code);

describe('schema-integrity rules', () => {
  it('clean minimal diagram has no errors', () => {
    expect(erc(base({})).filter((d) => d.severity === 'error')).toEqual([]);
  });
  it('SCHEMA_PINREF_MALFORMED for unparseable connection refs', () => {
    expect(codes(base({ nets: [{ name: 'N', kind: 'signal' }], connections: [['nocolon', 'N']] })))
      .toContain('SCHEMA_PINREF_MALFORMED');
  });
  it('SCHEMA_NET_UNDECLARED when a connection names a net not in nets[]', () => {
    expect(codes(base({ connections: [['mcu:GPIO8', 'GHOST']] })))
      .toContain('SCHEMA_NET_UNDECLARED');
  });
  it('SCHEMA_NET_DUPLICATE for duplicate net names', () => {
    expect(codes(base({ nets: [{ name: 'A', kind: 'signal' }, { name: 'A', kind: 'power' }] })))
      .toContain('SCHEMA_NET_DUPLICATE');
  });
  it('SCHEMA_PART_UNKNOWN for a part type missing from the catalog, with closest-match hint', () => {
    const out = erc(base({ parts: [{ id: 'mcu', type: 'esp32-s3-zero' }, { id: 'x', type: 'bme28' }] }));
    const d = out.find((x) => x.code === 'SCHEMA_PART_UNKNOWN')!;
    expect(d).toBeDefined();
    expect(d.hint).toContain('bme280');
  });
  it('SCHEMA_CONN_UNKNOWN_PART when a connection references a part id not in parts[]', () => {
    expect(codes(base({ nets: [{ name: 'N', kind: 'signal' }], connections: [['ghost:1', 'N']] })))
      .toContain('SCHEMA_CONN_UNKNOWN_PART');
  });
  it('SCHEMA_BOARD_UNKNOWN when diagram.board has no pin map', () => {
    expect(codes(base({ board: 'imaginary-board-9000' }))).toContain('SCHEMA_BOARD_UNKNOWN');
  });
  it('accepts v1 input (wires-only) via migration', () => {
    const v1 = { board: 'esp32-s3-zero', parts: [{ id: 'mcu', type: 'esp32-s3-zero' }], wires: [] };
    expect(() => erc(v1 as never)).not.toThrow();
  });
});
```

- [ ] **Step 2: Run to fail, then implement.**

`src/erc/diagnostic.ts`:

```typescript
/** Severity of an ERC finding. Errors block compile; warnings do not. */
export type Severity = 'error' | 'warning';

/** A machine-readable ERC finding (same shape philosophy as ICOMP_*). */
export interface Diagnostic {
  code: string;
  severity: Severity;
  message: string;
  hint: string;
  /** "part:pin" or net names this finding is about, when identifiable. */
  subjects?: string[];
}

export const diag = (
  code: string, severity: Severity, message: string, hint: string, subjects?: string[],
): Diagnostic => (subjects ? { code, severity, message, hint, subjects } : { code, severity, message, hint });
```

`src/erc/context.ts` — the shared resolution layer every rule family consumes:

```typescript
import type { DiagramV2, PinRef } from '../schema';
import type { ResolvedNet } from '../normalize';
import { resolveNets } from '../normalize';
import { CATALOG, getCatalogPart, type CatalogPart, type PinDecl, type PinEtype } from '../catalog';
import type { NetProtocol } from '../schema';
import { getPinEtype, getPinMapping, PIN_MAPS } from '../pin-mapping';
import type { Part } from '../types';

/** Everything a rule needs, resolved once. */
export interface ErcContext {
  diagram: DiagramV2;
  nets: ResolvedNet[];
  partsById: Map<string, Part>;
  /** Resolved net(s) per "part:pin" member key. */
  netsByPin: Map<string, ResolvedNet[]>;
}

/** Effective pin info: catalog decl, or MCU map lookup, or null (legacy/unknown). */
export interface EffectivePin {
  etype: PinEtype;
  role?: NetProtocol;
  required?: boolean;
  internalPullup?: boolean;
}

export function buildContext(diagram: DiagramV2): ErcContext {
  const nets = resolveNets(diagram);
  const partsById = new Map(diagram.parts.map((p) => [p.id, p]));
  const netsByPin = new Map<string, ResolvedNet[]>();
  for (const net of nets) {
    for (const m of net.members) {
      const k = `${m.part}:${m.pin}`;
      const arr = netsByPin.get(k) ?? [];
      arr.push(net);
      netsByPin.set(k, arr);
    }
  }
  return { diagram, nets, partsById, netsByPin };
}

/** True when the part is the MCU (its type has a pin map, or it is 'mcu'). */
export function isMcuPart(ctx: ErcContext, part: Part): boolean {
  return part.type === 'mcu' || PIN_MAPS[part.type] !== undefined;
}

/** Board key used for pin lookups of an MCU part. */
export function mcuBoardKey(ctx: ErcContext, part: Part): string {
  return PIN_MAPS[part.type] ? part.type : ctx.diagram.board;
}

/** Effective electrical pin info for a member; null = legacy/unknown (skip pin rules). */
export function effectivePin(ctx: ErcContext, member: PinRef): EffectivePin | null {
  const part = ctx.partsById.get(member.part);
  if (!part) return null;
  if (isMcuPart(ctx, part)) {
    const el = getPinEtype(mcuBoardKey(ctx, part), member.pin);
    if (!el) return null;
    const fn = getPinMapping(mcuBoardKey(ctx, part), member.pin);
    // Role from the pin map's declared functions, when unambiguous.
    const role = roleFromFunctions(fn);
    return { etype: el.etype, internalPullup: el.internalPullup, ...(role ? { role } : {}) };
  }
  const cat: CatalogPart | undefined = getCatalogPart(part.type);
  const decl: PinDecl | undefined = cat?.pins?.find((p) => p.name === member.pin);
  if (!decl) return null;
  return { etype: decl.etype, ...(decl.role ? { role: decl.role } : {}), ...(decl.required ? { required: true } : {}) };
}

function roleFromFunctions(fn: ReturnType<typeof getPinMapping>): NetProtocol | undefined {
  // Read the actual PinMapping shape (functions array with type+role fields)
  // and map i2c sda/scl, spi mosi/miso/sck/nss, uart tx/rx to NetProtocol.
  // Return undefined when no protocol function is declared.
  // (Implement against the real interface — see pin-mapping.ts.)
  if (!fn) return undefined;
  for (const f of fn.functions ?? []) {
    if (f.type === 'i2c' && f.role === 'sda') return 'i2c_sda';
    if (f.type === 'i2c' && f.role === 'scl') return 'i2c_scl';
    if (f.type === 'spi' && f.role === 'mosi') return 'spi_mosi';
    if (f.type === 'spi' && f.role === 'miso') return 'spi_miso';
    if (f.type === 'spi' && f.role === 'sck') return 'spi_sck';
    if (f.type === 'spi' && f.role === 'nss') return 'spi_cs';
    if (f.type === 'uart' && f.role === 'tx') return 'uart_tx';
    if (f.type === 'uart' && f.role === 'rx') return 'uart_rx';
  }
  return undefined;
}
```

(`roleFromFunctions` is written against the audited `PinFunction` shape — `{type, peripheral, channel?, role?}`. READ `pin-mapping.ts` and adjust field access if the real shape differs; keep the mapping table identical.)

`src/erc/schema-rules.ts`:

```typescript
import type { DiagramV2 } from '../schema';
import { parsePinRef } from '../schema';
import { getCatalogPart, CATALOG } from '../catalog';
import { PIN_MAPS } from '../pin-mapping';
import { diag, type Diagnostic } from './diagnostic';

/** Cheap edit-distance for closest-match hints (insert/delete/replace = 1). */
function editDistance(a: string, b: string): number {
  const dp = Array.from({ length: a.length + 1 }, (_, i) => [i, ...Array(b.length).fill(0)]);
  for (let j = 1; j <= b.length; j++) dp[0][j] = j;
  for (let i = 1; i <= a.length; i++)
    for (let j = 1; j <= b.length; j++)
      dp[i][j] = Math.min(dp[i - 1][j] + 1, dp[i][j - 1] + 1, dp[i - 1][j - 1] + (a[i - 1] === b[j - 1] ? 0 : 1));
  return dp[a.length][b.length];
}

function closest(input: string, candidates: string[]): string | undefined {
  let best: string | undefined;
  let bestD = 3; // suggest only near-misses
  for (const c of candidates) {
    const d = editDistance(input, c);
    if (d < bestD) { bestD = d; best = c; }
  }
  return best;
}

export function schemaRules(d: DiagramV2): Diagnostic[] {
  const out: Diagnostic[] = [];
  if (!PIN_MAPS[d.board]) {
    out.push(diag('SCHEMA_BOARD_UNKNOWN', 'error',
      `board '${d.board}' has no pin map`,
      `Known boards: ${Object.keys(PIN_MAPS).sort().join(', ')}`));
  }
  const netNames = new Set<string>();
  for (const n of d.nets) {
    if (netNames.has(n.name)) {
      out.push(diag('SCHEMA_NET_DUPLICATE', 'error',
        `net '${n.name}' declared more than once`, 'Give each net a unique name', [n.name]));
    }
    netNames.add(n.name);
  }
  const partIds = new Set(d.parts.map((p) => p.id));
  for (const p of d.parts) {
    if (!getCatalogPart(p.type) && !PIN_MAPS[p.type] && p.type !== 'mcu') {
      const suggestion = closest(p.type, [...Object.keys(CATALOG), ...Object.keys(PIN_MAPS)]);
      out.push(diag('SCHEMA_PART_UNKNOWN', 'error',
        `part '${p.id}' has unknown type '${p.type}'`,
        suggestion ? `Did you mean '${suggestion}'?` : 'See the part catalog for valid types', [p.id]));
    }
  }
  for (const [ref, netName] of d.connections) {
    const pin = parsePinRef(ref);
    if (!pin) {
      out.push(diag('SCHEMA_PINREF_MALFORMED', 'error',
        `connection ref '${ref}' is not 'partId:pinName'`, "Use the form 'partId:pinName'", [ref]));
      continue;
    }
    if (!partIds.has(pin.part)) {
      out.push(diag('SCHEMA_CONN_UNKNOWN_PART', 'error',
        `connection '${ref}' references missing part '${pin.part}'`, 'Add the part or fix the id', [ref]));
    }
    if (!netNames.has(netName)) {
      out.push(diag('SCHEMA_NET_UNDECLARED', 'error',
        `connection '${ref}' references undeclared net '${netName}'`,
        `Declare the net in nets[] (known: ${[...netNames].join(', ') || 'none'})`, [ref, netName]));
    }
  }
  return out;
}
```

`src/erc/index.ts`:

```typescript
import type { Diagram } from '../types';
import type { DiagramV2 } from '../schema';
import { migrateToV2 } from '../schema';
import { buildContext } from './context';
import { schemaRules } from './schema-rules';
import type { Diagnostic } from './diagnostic';

export type { Diagnostic, Severity } from './diagnostic';

/** Run all ERC rule families. Accepts v1 or v2; migrates internally. */
export function erc(input: Diagram | DiagramV2): Diagnostic[] {
  const d = migrateToV2(input);
  const out: Diagnostic[] = [...schemaRules(d)];
  const ctx = buildContext(d);
  void ctx; // matrix/power/bus families plug in here (Tasks 3-5)
  return out;
}
```

- [ ] **Step 3: Verify + commit** — `npx vitest run && npm run typecheck` green.

```bash
git add packages/board-config/src/erc packages/board-config/test/erc-schema.test.ts
git commit -m "feat(board-config): erc entry point + schema-integrity rules with closest-match hints"
```

---

### Task 3: Pin-pair matrix rules

**Files:**
- Create: `packages/board-config/src/erc/matrix-rules.ts`
- Modify: `packages/board-config/src/erc/index.ts`
- Test: `packages/board-config/test/erc-matrix.test.ts`

- [ ] **Step 1: Failing tests** — `test/erc-matrix.test.ts`. Build small diagrams with two single-pin probe parts. Probe parts must come from the catalog with declared pins; for controlled etypes ADD (in this task) two internal test-only catalog entries — NO: do not pollute the catalog. Instead use real parts: MCU pins (`bidirectional`/`power_out`), `pca9685` (`open_drain` SDA, `output` LED0, `power_in` VCC), `resistor` (`passive`), `ultrasonic` (`output` ECHO, `input` TRIG). These cover every cell we act on:

```typescript
import { describe, expect, it } from 'vitest';
import { erc } from '../src/erc';
import type { DiagramV2 } from '../src/schema';

const two = (aPart: string, aType: string, aPin: string, bPart: string, bType: string, bPin: string): DiagramV2 => ({
  version: 2, board: 'esp32-s3-zero',
  parts: [
    { id: 'mcu', type: 'esp32-s3-zero' },
    ...(aPart === 'mcu' ? [] : [{ id: aPart, type: aType }]),
    ...(bPart === 'mcu' || bPart === aPart ? [] : [{ id: bPart, type: bType }]),
  ],
  nets: [{ name: 'N', kind: 'signal' as const }],
  connections: [[`${aPart}:${aPin}`, 'N'], [`${bPart}:${bPin}`, 'N']],
  wires: [],
});

const codesOf = (d: DiagramV2) => erc(d).map((x) => x.code);

describe('pin-pair matrix', () => {
  it('NET_DRIVER_CONFLICT: two outputs on one net', () => {
    // pca9685 LED0 (output) + ultrasonic ECHO (output)
    expect(codesOf(two('p1', 'pca9685', 'LED0', 'u1', 'ultrasonic', 'ECHO')))
      .toContain('NET_DRIVER_CONFLICT');
  });
  it('NET_DRIVER_CONFLICT: output driving a power_out rail pin', () => {
    expect(codesOf(two('p1', 'pca9685', 'LED0', 'mcu', '', '3V3')))
      .toContain('NET_DRIVER_CONFLICT');
  });
  it('NET_RAIL_SHORT: two power_out pins shorted', () => {
    expect(codesOf(two('mcu', '', '3V3', 'mcu', '', '5V'))).toContain('NET_RAIL_SHORT');
  });
  it('no finding for passive + anything', () => {
    expect(codesOf(two('r1', 'resistor', '1', 'p1', 'pca9685', 'LED0')))
      .not.toContain('NET_DRIVER_CONFLICT');
  });
  it('no finding for input + output', () => {
    expect(codesOf(two('u1', 'ultrasonic', 'TRIG', 'p1', 'pca9685', 'LED0')))
      .not.toContain('NET_DRIVER_CONFLICT');
  });
  it('legacy parts (no pin decls) are skipped silently', () => {
    expect(codesOf(two('k1', 'keypad', 'X', 'p1', 'pca9685', 'LED0'))).toEqual(
      expect.not.arrayContaining(['NET_DRIVER_CONFLICT', 'NET_UNSPECIFIED_PIN']),
    );
  });
  it('NET_RAIL_SHORT: two power nets with different voltages bridged by one pin', () => {
    const d: DiagramV2 = {
      version: 2, board: 'esp32-s3-zero',
      parts: [{ id: 'mcu', type: 'esp32-s3-zero' }, { id: 'p1', type: 'pca9685' }],
      nets: [
        { name: '3V3', kind: 'power', voltage: 3.3 },
        { name: '5V0', kind: 'power', voltage: 5 },
      ],
      connections: [['p1:VCC', '3V3'], ['p1:VCC', '5V0']],
      wires: [],
    };
    expect(codesOf(d)).toContain('NET_RAIL_SHORT');
  });
});
```

(`two('mcu','','3V3', ...)` uses the MCU part for rail pins. NET_NC_CONNECTED and NET_UNSPECIFIED_PIN have no real catalog parts with those etypes yet — cover them with unit tests on the exported `pairFinding()` helper directly instead of diagram fixtures; add those to the test file.)

- [ ] **Step 2: Implement `matrix-rules.ts`:**

```typescript
import { diag, type Diagnostic } from './diagnostic';
import { effectivePin, type ErcContext } from './context';
import type { PinEtype } from '../catalog';

type Cell = { code: string; severity: 'error' | 'warning' } | null;

/** The acted-on cells of the (symmetric) pin-pair matrix; everything else OK. */
export function pairFinding(a: PinEtype, b: PinEtype): Cell {
  const pair = (x: PinEtype, y: PinEtype) => (a === x && b === y) || (a === y && b === x);
  if (a === 'nc' || b === 'nc') return { code: 'NET_NC_CONNECTED', severity: 'error' };
  if (pair('output', 'output')) return { code: 'NET_DRIVER_CONFLICT', severity: 'error' };
  if (pair('output', 'power_out')) return { code: 'NET_DRIVER_CONFLICT', severity: 'error' };
  if (pair('power_out', 'power_out')) return { code: 'NET_RAIL_SHORT', severity: 'error' };
  if (pair('open_drain', 'output')) return { code: 'NET_DRIVER_CONFLICT', severity: 'warning' };
  if ((a === 'unspecified') !== (b === 'unspecified'))
    return { code: 'NET_UNSPECIFIED_PIN', severity: 'warning' };
  return null;
}

export function matrixRules(ctx: ErcContext): Diagnostic[] {
  const out: Diagnostic[] = [];
  const seen = new Set<string>();
  for (const net of ctx.nets) {
    const typed = net.members
      .map((m) => ({ m, pin: effectivePin(ctx, m) }))
      .filter((x): x is { m: (typeof x)['m']; pin: NonNullable<(typeof x)['pin']> } => x.pin !== null);
    for (let i = 0; i < typed.length; i++) {
      for (let j = i + 1; j < typed.length; j++) {
        const f = pairFinding(typed[i].pin.etype, typed[j].pin.etype);
        if (!f) continue;
        const subj = [`${typed[i].m.part}:${typed[i].m.pin}`, `${typed[j].m.part}:${typed[j].m.pin}`];
        const key = `${f.code}|${net.name}|${subj.join('|')}`;
        if (seen.has(key)) continue;
        seen.add(key);
        out.push(diag(f.code, f.severity,
          `${f.code === 'NET_RAIL_SHORT' ? 'rail short' : f.code === 'NET_NC_CONNECTED' ? 'NC pin connected' : 'driver conflict'} on net '${net.name}': ${subj[0]} (${typed[i].pin.etype}) with ${subj[1]} (${typed[j].pin.etype})`,
          'Rewire so the net has at most one push-pull driver and no shorted rails',
          [...subj, net.name]));
      }
    }
  }
  // Bridge clause: one pin member of two declared power nets at different voltages.
  for (const [pinKey, nets] of ctx.netsByPin) {
    const powers = nets.filter((n) => n.declared && n.kind === 'power' && n.voltage !== undefined);
    for (let i = 0; i < powers.length; i++)
      for (let j = i + 1; j < powers.length; j++)
        if (powers[i].voltage !== powers[j].voltage)
          out.push(diag('NET_RAIL_SHORT', 'error',
            `pin ${pinKey} bridges power nets '${powers[i].name}' (${powers[i].voltage}V) and '${powers[j].name}' (${powers[j].voltage}V)`,
            'A pin cannot sit on two rails of different voltage', [pinKey, powers[i].name, powers[j].name]));
  }
  return out;
}
```

Wire into `erc/index.ts` (`out.push(...matrixRules(ctx))`). Export `pairFinding` for the unit tests.

- [ ] **Step 3: Verify + commit** — full suite + typecheck green.

```bash
git add packages/board-config/src/erc packages/board-config/test/erc-matrix.test.ts
git commit -m "feat(board-config): erc pin-pair matrix rules (driver conflicts, rail shorts, NC)"
```

---

### Task 4: Power rules

**Files:**
- Create: `packages/board-config/src/erc/power-rules.ts`
- Modify: `packages/board-config/src/erc/index.ts`
- Test: `packages/board-config/test/erc-power.test.ts`

- [ ] **Step 1: Failing tests** — fixture pairs (trigger + corrected twin):

```typescript
import { describe, expect, it } from 'vitest';
import { erc } from '../src/erc';
import type { DiagramV2 } from '../src/schema';

const codesOf = (d: DiagramV2) => erc(d).map((x) => x.code);

const powered = (connections: DiagramV2['connections'], nets: DiagramV2['nets']): DiagramV2 => ({
  version: 2, board: 'esp32-s3-zero',
  parts: [{ id: 'mcu', type: 'esp32-s3-zero' }, { id: 'b1', type: 'bme280' }],
  nets, connections, wires: [],
});

describe('power rules', () => {
  it('PWR_RAIL_UNDRIVEN: power_in pins with no power_out on the net — and its corrected twin', () => {
    const bad = powered([['b1:VCC', 'V']], [{ name: 'V', kind: 'power', voltage: 3.3 }]);
    expect(codesOf(bad)).toContain('PWR_RAIL_UNDRIVEN');
    const good = powered(
      [['b1:VCC', 'V'], ['mcu:3V3', 'V']],
      [{ name: 'V', kind: 'power', voltage: 3.3 }],
    );
    expect(codesOf(good)).not.toContain('PWR_RAIL_UNDRIVEN');
  });

  it('PWR_VOLTAGE_MISMATCH: 5V rail feeding a 3.6V-max part — twin on 3V3 passes', () => {
    const bad = powered(
      [['b1:VCC', 'V5'], ['mcu:5V', 'V5']],
      [{ name: 'V5', kind: 'power', voltage: 5 }],
    );
    expect(codesOf(bad)).toContain('PWR_VOLTAGE_MISMATCH');
    const good = powered(
      [['b1:VCC', 'V3'], ['mcu:3V3', 'V3']],
      [{ name: 'V3', kind: 'power', voltage: 3.3 }],
    );
    expect(codesOf(good)).not.toContain('PWR_VOLTAGE_MISMATCH');
  });

  it('PWR_NO_GROUND: powered part with no pin on a 0V net — twin with GND passes', () => {
    const bad = powered(
      [['b1:VCC', 'V3'], ['mcu:3V3', 'V3']],
      [{ name: 'V3', kind: 'power', voltage: 3.3 }],
    );
    expect(codesOf(bad)).toContain('PWR_NO_GROUND');
    const good = powered(
      [['b1:VCC', 'V3'], ['mcu:3V3', 'V3'], ['b1:GND', 'G'], ['mcu:GND', 'G']],
      [{ name: 'V3', kind: 'power', voltage: 3.3 }, { name: 'G', kind: 'power', voltage: 0 }],
    );
    expect(codesOf(good)).not.toContain('PWR_NO_GROUND');
  });
});
```

- [ ] **Step 2: Implement `power-rules.ts`:**

```typescript
import { diag, type Diagnostic } from './diagnostic';
import { effectivePin, type ErcContext } from './context';
import { getCatalogPart } from '../catalog';

export function powerRules(ctx: ErcContext): Diagnostic[] {
  const out: Diagnostic[] = [];

  for (const net of ctx.nets) {
    const pins = net.members.map((m) => ({ m, pin: effectivePin(ctx, m) }));
    const hasPowerIn = pins.some((x) => x.pin?.etype === 'power_in');
    const hasPowerOut = pins.some((x) => x.pin?.etype === 'power_out');
    if (hasPowerIn && !hasPowerOut) {
      out.push(diag('PWR_RAIL_UNDRIVEN', 'error',
        `net '${net.name}' powers parts but has no supply (no power_out pin)`,
        'Connect an MCU rail pin (3V3/5V/GND) or a supply part to the net', [net.name]));
    }
    // Voltage mismatch: declared power net with voltage>0 feeding power_in pins
    // of parts whose operatingVoltage excludes it.
    if (net.declared && net.kind === 'power' && net.voltage !== undefined && net.voltage > 0) {
      for (const { m, pin } of pins) {
        if (pin?.etype !== 'power_in') continue;
        const part = ctx.partsById.get(m.part);
        const range = part && getCatalogPart(part.type)?.operatingVoltage;
        if (range && (net.voltage < range.min || net.voltage > range.max)) {
          out.push(diag('PWR_VOLTAGE_MISMATCH', 'error',
            `${m.part}:${m.pin} on ${net.voltage}V net '${net.name}' but '${part!.type}' operates ${range.min}-${range.max}V`,
            'Move the part to a rail inside its operating range', [`${m.part}:${m.pin}`, net.name]));
        }
      }
    }
  }

  // PWR_NO_GROUND: a part with declared power_in pins and an operating range
  // must touch a 0V net somewhere.
  const groundNets = new Set(
    ctx.nets.filter((n) => n.kind === 'power' && n.voltage === 0).map((n) => n.name),
  );
  for (const part of ctx.diagram.parts) {
    const cat = getCatalogPart(part.type);
    if (!cat?.pins?.some((p) => p.etype === 'power_in') || !cat.operatingVoltage) continue;
    const touchesGround = [...ctx.netsByPin.entries()].some(
      ([k, nets]) => k.startsWith(`${part.id}:`) && nets.some((n) => groundNets.has(n.name)),
    );
    if (!touchesGround) {
      out.push(diag('PWR_NO_GROUND', 'warning',
        `powered part '${part.id}' (${part.type}) has no pin on a 0V net`,
        'Wire its GND pin to the ground net', [part.id]));
    }
  }
  return out;
}
```

Wire into `erc/index.ts`.

- [ ] **Step 3: Verify + commit.**

```bash
git add packages/board-config/src/erc packages/board-config/test/erc-power.test.ts
git commit -m "feat(board-config): erc power rules (undriven rails, voltage mismatch, missing ground)"
```

---

### Task 5: Bus rules + floating required inputs

**Files:**
- Create: `packages/board-config/src/erc/bus-rules.ts`
- Modify: `packages/board-config/src/erc/index.ts`
- Test: `packages/board-config/test/erc-bus.test.ts`

- [ ] **Step 1: Failing tests** — fixture pairs again. Use bme280 + pca9685 (I2C), ultrasonic for inputs, neo6m-gps-class part only if it has catalog pins (it does not yet — UART rule tests therefore use two MCU-pin endpoints via pin-map roles: `esp32-s3-zero` GPIO43=uart0 tx, GPIO44=uart0 rx — verify in ESP32S3_PINS and adapt):

```typescript
import { describe, expect, it } from 'vitest';
import { erc } from '../src/erc';
import type { DiagramV2 } from '../src/schema';

const codesOf = (d: DiagramV2) => erc(d).map((x) => x.code);

const i2cPair = (extra: Partial<DiagramV2>): DiagramV2 => ({
  version: 2, board: 'esp32-s3-zero',
  parts: [
    { id: 'mcu', type: 'esp32-s3-zero' },
    { id: 'a', type: 'bme280', attrs: { i2c_address: '0x76' } },
    { id: 'b', type: 'pca9685', attrs: { i2c_address: '0x76' } },
  ],
  nets: [
    { name: 'SDA', kind: 'signal', protocol: 'i2c_sda' },
    { name: 'SCL', kind: 'signal', protocol: 'i2c_scl' },
  ],
  connections: [
    ['a:SDA', 'SDA'], ['b:SDA', 'SDA'], ['mcu:GPIO8', 'SDA'],
    ['a:SCL', 'SCL'], ['b:SCL', 'SCL'], ['mcu:GPIO9', 'SCL'],
  ],
  wires: [],
  ...extra,
});

describe('bus rules', () => {
  it('I2C_ADDR_CONFLICT when two devices share a bus and an address — distinct addresses pass', () => {
    expect(codesOf(i2cPair({}))).toContain('I2C_ADDR_CONFLICT');
    const ok = i2cPair({});
    ok.parts = ok.parts.map((p) => (p.id === 'b' ? { ...p, attrs: { i2c_address: '0x40' } } : p));
    expect(codesOf(ok)).not.toContain('I2C_ADDR_CONFLICT');
  });

  it('I2C_NO_PULLUP on open-drain nets without pull path — resistor to rail OR mcu internal pullups satisfy it', () => {
    expect(codesOf(i2cPair({}))).toContain('I2C_NO_PULLUP');
    // Satisfied by internal pullups:
    const internal = i2cPair({});
    internal.parts = internal.parts.map((p) =>
      p.id === 'mcu' ? { ...p, attrs: { internal_pullups: 'GPIO8,GPIO9' } } : p,
    );
    expect(codesOf(internal)).not.toContain('I2C_NO_PULLUP');
    // Satisfied by physical resistors to a power net:
    const resistored = i2cPair({});
    resistored.parts.push({ id: 'r1', type: 'resistor' }, { id: 'r2', type: 'resistor' });
    resistored.nets.push({ name: 'V3', kind: 'power', voltage: 3.3 });
    resistored.connections.push(
      ['r1:1', 'SDA'], ['r1:2', 'V3'],
      ['r2:1', 'SCL'], ['r2:2', 'V3'],
      ['mcu:3V3', 'V3'],
    );
    expect(codesOf(resistored)).not.toContain('I2C_NO_PULLUP');
  });

  it('PIN_INPUT_FLOATING for required inputs on no net — wired twin passes', () => {
    const bad: DiagramV2 = {
      version: 2, board: 'esp32-s3-zero',
      parts: [{ id: 'mcu', type: 'esp32-s3-zero' }, { id: 'u1', type: 'ultrasonic' }],
      nets: [], connections: [], wires: [],
    };
    expect(codesOf(bad)).toContain('PIN_INPUT_FLOATING');
    const good: DiagramV2 = {
      ...bad,
      nets: [{ name: 'T', kind: 'signal' }],
      connections: [['u1:TRIG', 'T'], ['mcu:GPIO4', 'T']],
    };
    expect(codesOf(good)).not.toContain('PIN_INPUT_FLOATING');
  });

  it('UART_CROSSOVER: two TX pins on one net (mcu uart0 TX + a second board TX via wires-only twin part)', () => {
    // Single-MCU diagrams can't produce two TX role pins without a UART part;
    // exercise the rule through the exported helper instead (unit-style):
    // see uartCrossover unit tests below in the same file.
    expect(true).toBe(true);
  });
});
```

Plus direct unit tests of the exported `uartCrossover(netProtocolPins)` helper for tx/tx, rx/rx, tx/rx (no real catalog UART part exists yet — the helper is the testable unit; the diagram-level path is exercised when a UART device gains catalog pins).

- [ ] **Step 2: Implement `bus-rules.ts`:**

```typescript
import { diag, type Diagnostic } from './diagnostic';
import { effectivePin, type ErcContext } from './context';
import { getCatalogPart } from '../catalog';
import type { PinRef } from '../schema';

/** Parse "0x40"/"64" to a number; undefined when absent/invalid. */
function parseAddr(s: string | undefined): number | undefined {
  if (!s) return undefined;
  const n = s.trim().toLowerCase().startsWith('0x') ? parseInt(s, 16) : parseInt(s, 10);
  return Number.isFinite(n) ? n : undefined;
}

export function busRules(ctx: ErcContext): Diagnostic[] {
  const out: Diagnostic[] = [];

  // --- I2C: group device SDA pins by their resolved net (= the bus) ---
  const busDevices = new Map<string, { id: string; addr: number | undefined }[]>();
  for (const part of ctx.diagram.parts) {
    const cat = getCatalogPart(part.type);
    const sda = cat?.pins?.find((p) => p.role === 'i2c_sda');
    if (!sda) continue;
    const nets = ctx.netsByPin.get(`${part.id}:${sda.name}`) ?? [];
    for (const net of nets) {
      const arr = busDevices.get(net.name) ?? [];
      arr.push({ id: part.id, addr: parseAddr(part.attrs?.i2c_address) });
      busDevices.set(net.name, arr);
    }
  }
  for (const [netName, devs] of busDevices) {
    const byAddr = new Map<number, string[]>();
    for (const d of devs) {
      if (d.addr === undefined) continue; // unknown address: cannot judge
      byAddr.set(d.addr, [...(byAddr.get(d.addr) ?? []), d.id]);
    }
    for (const [addr, ids] of byAddr) {
      if (ids.length > 1) {
        out.push(diag('I2C_ADDR_CONFLICT', 'error',
          `devices ${ids.join(', ')} share I2C address 0x${addr.toString(16)} on net '${netName}'`,
          'Change one device address (attrs.i2c_address / address-select pins)', [...ids, netName]));
      }
    }
  }

  // --- I2C pull-ups: every open-drain i2c net needs a pull path ---
  const powerNets = new Set(
    ctx.nets.filter((n) => n.kind === 'power' && (n.voltage ?? 0) > 0).map((n) => n.name),
  );
  for (const net of ctx.nets) {
    const isI2c =
      net.protocol === 'i2c_sda' || net.protocol === 'i2c_scl' ||
      net.members.some((m) => {
        const p = effectivePin(ctx, m);
        return p?.role === 'i2c_sda' || p?.role === 'i2c_scl';
      });
    if (!isI2c) continue;
    const pulled = hasPullPath(ctx, net.name, net.members, powerNets);
    if (!pulled) {
      out.push(diag('I2C_NO_PULLUP', 'warning',
        `open-drain net '${net.name}' has no pull-up (no resistor to a rail, no MCU internal pullup enabled)`,
        "Add a pull-up resistor to a power net, or set the MCU part's attrs.internal_pullups to include the pin",
        [net.name]));
    }
  }

  // --- SPI CS coverage ---
  for (const part of ctx.diagram.parts) {
    const cat = getCatalogPart(part.type);
    const cs = cat?.pins?.find((p) => p.role === 'spi_cs');
    if (!cs) continue;
    const nets = ctx.netsByPin.get(`${part.id}:${cs.name}`) ?? [];
    const driven = nets.some((n) =>
      n.members.some((m) => {
        if (m.part === part.id) return false;
        const p = effectivePin(ctx, m);
        return p?.etype === 'output' || p?.etype === 'bidirectional';
      }),
    );
    if (!driven) {
      out.push(diag('SPI_NO_CS', 'warning',
        `SPI device '${part.id}' chip-select '${cs.name}' is not driven by any MCU/output pin`,
        'Wire the CS pin to a free MCU GPIO', [`${part.id}:${cs.name}`]));
    }
  }

  // --- UART crossover ---
  for (const net of ctx.nets) {
    const roles = net.members
      .map((m) => ({ m, p: effectivePin(ctx, m) }))
      .filter((x) => x.p?.role === 'uart_tx' || x.p?.role === 'uart_rx');
    out.push(...uartCrossover(net.name, roles.map((x) => ({ key: `${x.m.part}:${x.m.pin}`, role: x.p!.role! }))));
  }

  // --- Floating required inputs ---
  for (const part of ctx.diagram.parts) {
    const cat = getCatalogPart(part.type);
    for (const pin of cat?.pins ?? []) {
      if (!pin.required) continue;
      if (!(ctx.netsByPin.get(`${part.id}:${pin.name}`)?.length)) {
        out.push(diag('PIN_INPUT_FLOATING', 'warning',
          `required input ${part.id}:${pin.name} is not connected to any net`,
          'Wire the pin (it must be driven for the part to function)', [`${part.id}:${pin.name}`]));
      }
    }
  }
  return out;
}

/** Exported for unit tests: two same-direction UART pins on one net = error. */
export function uartCrossover(
  netName: string,
  pins: { key: string; role: string }[],
): Diagnostic[] {
  const out: Diagnostic[] = [];
  const tx = pins.filter((p) => p.role === 'uart_tx');
  const rx = pins.filter((p) => p.role === 'uart_rx');
  for (const group of [tx, rx]) {
    if (group.length > 1) {
      out.push(diag('UART_CROSSOVER', 'error',
        `net '${netName}' connects ${group.length} UART ${group === tx ? 'TX' : 'RX'} pins together (${group.map((g) => g.key).join(', ')})`,
        'UART wiring crosses over: TX connects to RX', [...group.map((g) => g.key), netName]));
    }
  }
  return out;
}

/** A pull path exists when a passive part bridges this net to a positive rail,
 * or an MCU member pin has internal pullups enabled via attrs. */
function hasPullPath(
  ctx: ErcContext,
  netName: string,
  members: PinRef[],
  powerNets: Set<string>,
): boolean {
  for (const m of members) {
    const part = ctx.partsById.get(m.part);
    if (!part) continue;
    const cat = getCatalogPart(part.type);
    // passive bridge: the part's OTHER pins sit on a positive power net
    if (cat?.deviceClass === 'passive' && cat.pins) {
      for (const other of cat.pins) {
        if (other.name === m.pin) continue;
        const otherNets = ctx.netsByPin.get(`${part.id}:${other.name}`) ?? [];
        if (otherNets.some((n) => powerNets.has(n.name))) return true;
      }
    }
    // MCU internal pullup, opted in via attrs
    const p = effectivePin(ctx, m);
    if (p?.internalPullup) {
      const list = (part.attrs?.internal_pullups ?? '').split(',').map((s) => s.trim());
      if (list.includes(m.pin)) return true;
    }
  }
  return false;
}
```

Wire `busRules` into `erc/index.ts`. Also export `erc` and `Diagnostic` from `src/index.ts` (package surface): `export { erc } from './erc'; export type { Diagnostic, Severity } from './erc';`.

- [ ] **Step 3: Verify + commit** — full suite + typecheck. Also add one integration fixture test (in `test/erc-bus.test.ts`): the SpiceDispenser-shaped wiring (esp32-s3-zero + pca9685 0x40 + two servos on LED8/LED12 + 3V3/5V/GND + pull-up resistors on SDA/SCL) produces **zero errors** (warnings allowed only if justified — assert the exact set, e.g. servo VCC on 5V rail must NOT trip PWR_VOLTAGE_MISMATCH since 5.0 ∈ [4.8,6.0]).

```bash
git add packages/board-config/src/erc packages/board-config/src/index.ts packages/board-config/test/erc-bus.test.ts
git commit -m "feat(board-config): erc bus rules (i2c address/pullups, spi cs, uart, floating inputs)"
```

---

## Self-review notes

- Spec §4 coverage: schema-integrity (Task 2, extends spec with SCHEMA_* family — the spec's error-handling section requires malformed input → diagnostics), matrix (Task 3: NET_DRIVER_CONFLICT, NET_RAIL_SHORT both clauses, NET_NC_CONNECTED, NET_UNSPECIFIED_PIN), power (Task 4: PWR_RAIL_UNDRIVEN, PWR_VOLTAGE_MISMATCH, PWR_NO_GROUND), bus (Task 5: I2C_ADDR_CONFLICT, I2C_NO_PULLUP with both satisfaction paths, SPI_NO_CS, UART_CROSSOVER via helper, PIN_INPUT_FLOATING). IRQ_SOURCE_ORDINAL is compile-time → Plan C. Legacy 14 codes remain in the adapters until Plan C unifies them.
- Open-drain pushed by push-pull is folded into the matrix as warning (open_drain×output) per spec rule 6.
- Known check-at-implementation points: real `PinFunction` field names in `roleFromFunctions`; GPS part key name; ESP32S3 uart role entries; vitest `as const` nuances in fixtures.
- Type consistency: `Diagnostic` defined once in erc/diagnostic.ts; `EffectivePin` in context.ts consumed by all rule families; `pairFinding`/`uartCrossover` exported for unit tests.
