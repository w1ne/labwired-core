# LabWired Agent-Loop — Part A (Authoring) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make an agent able to author a runnable LabWired system manifest on the first try — give it the schema, let it name a chip by id, and turn the dead `config_error` into an error that names the offending field and the fix.

**Architecture:** Two repos. The **builder** (`w1ne/labwired`, `services/labwired-builder`) gains chip-by-id resolution, captured-stderr diagnostics, and a `/chips` endpoint. The **gateway** (`w1ne/protocat-private`, `src/lib/mcp/gateway.ts`) gains a `labwired_lookup` tool and points `plan_device` at it. The gateway already forwards the manifest verbatim, so chip-by-id needs no gateway change.

**Tech Stack:** TypeScript, Node, vitest (both repos use `vitest run`). Builder talks to the `labwired` Rust CLI over `execFile`. Gateway is Next.js + MCP SDK.

## Global Constraints

- **Git attribution (both repos):** `Andrii Shylenko <14119286+w1ne@users.noreply.github.com>`. NEVER add `Co-Authored-By: Claude` or "Generated with Claude Code".
- **Never rebase. Always merge.**
- **Start from latest `origin/main`** in each repo before branching. (Builder worktree `~/projects/labwired-agent-loop` already exists on `feat/agent-loop-close` off `origin/main`. For the gateway, create a branch off latest `main` per its CLAUDE.md rule 7.)
- **Gateway is the EXTERNAL CONTRACT** — additive only; never remove or rename an existing tool. Keep all tool output free of internal repo/infra names.
- **Both test commands:** `npx vitest run <file>` from the package root.
- **Builder package root:** `~/projects/labwired-agent-loop/services/labwired-builder`.
- **Gateway repo root:** `~/projects/protocat-private` (branch off `main`, not the builder worktree).

---

## File Structure

**Builder (`services/labwired-builder/`):**
- Modify `src/run.ts` — export `buildDiagnosis`; add `stderrTail` param + capture stderr; add `resolveChipInManifest()` and call it in `run()`.
- Modify `src/server.ts` — add `GET /chips`.
- Create `test/chip-resolve.test.ts` — unit tests for `resolveChipInManifest`.
- Modify `test/run.test.ts` — unit tests for the new `config_error` diagnosis (binary-free).
- Modify `test/server.test.ts` — test `GET /chips`.

**Gateway (`protocat-private/`):**
- Create `src/lib/mcp/labwired-lookup.ts` — the static schema/example/chip-list payloads + the `labwiredLookup()` resolver.
- Modify `src/lib/mcp/gateway.ts` — register `labwired_lookup` in `PROTOCAT_TOOLS`, dispatch it, point `plan_device` electronics phase + `labwired_run_build` description at it.
- Modify `src/lib/mcp/gateway.test.ts` — assert the new tool is exposed and returns schema + chips.

---

## Task 1: Builder — chip-by-id resolution in the manifest

The manifest `chip:` field currently must be a filesystem path or `"inline"`. Let an agent write `chip: "esp32c3"` (a bare id) and resolve it from the already-bundled `CHIP_YAMLS` map. Unknown id → an error that lists the valid ids.

**Files:**
- Modify: `services/labwired-builder/src/run.ts` (chip block at lines 167-173; imports at top)
- Test: `services/labwired-builder/test/chip-resolve.test.ts` (create)

**Interfaces:**
- Produces: `export function resolveChipInManifest(systemYaml: string, chipYamlOverride?: string): { systemYaml: string; chipYaml?: string }` — throws `Error` with a "Known chip ids: …" message on an unknown bare id.
- Consumes: `CHIP_YAMLS` (`Record<string,string>`) from `../../../packages/board-config/src/chip-yamls` (already imported in `test/run.test.ts:7`).

- [ ] **Step 1: Write the failing test**

Create `services/labwired-builder/test/chip-resolve.test.ts`:

```typescript
import { describe, it, expect } from 'vitest';
import { resolveChipInManifest } from '../src/run';
import { CHIP_YAMLS } from '../../../packages/board-config/src/chip-yamls';

const knownId = Object.keys(CHIP_YAMLS)[0]; // a guaranteed-valid id

describe('resolveChipInManifest', () => {
  it('resolves a bare chip id to its bundled YAML and rewrites the field', () => {
    const sys = `name: "t"\nchip: "${knownId}"\nboard_io: []\n`;
    const out = resolveChipInManifest(sys);
    expect(out.chipYaml).toBe(CHIP_YAMLS[knownId]);
    expect(out.systemYaml).toMatch(/^chip:\s*"chip\.yaml"/m);
  });

  it('throws a listing error on an unknown bare id', () => {
    const sys = `name: "t"\nchip: "totally-not-a-chip"\nboard_io: []\n`;
    expect(() => resolveChipInManifest(sys)).toThrow(/unknown chip id .*Known chip ids:/s);
  });

  it('leaves a path-style chip untouched', () => {
    const sys = `name: "t"\nchip: "../../configs/chips/stm32f103.yaml"\nboard_io: []\n`;
    const out = resolveChipInManifest(sys);
    expect(out.chipYaml).toBeUndefined();
    expect(out.systemYaml).toBe(sys);
  });

  it('honors an explicit chipYaml override and rewrites the inline placeholder', () => {
    const sys = `name: "t"\nchip: "inline"\nboard_io: []\n`;
    const out = resolveChipInManifest(sys, 'name: custom\n');
    expect(out.chipYaml).toBe('name: custom\n');
    expect(out.systemYaml).toMatch(/^chip:\s*"chip\.yaml"/m);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd ~/projects/labwired-agent-loop/services/labwired-builder && npx vitest run test/chip-resolve.test.ts`
Expected: FAIL — `resolveChipInManifest` is not exported from `../src/run`.

- [ ] **Step 3: Write minimal implementation**

In `services/labwired-builder/src/run.ts`, add the import near the top (after line 6):

```typescript
import { CHIP_YAMLS } from '../../../packages/board-config/src/chip-yamls';
```

Add the exported helper above `export async function run(` (i.e. before line 154):

```typescript
/** Resolve the manifest's `chip:` field so the CLI can load it.
 *  - explicit chipYamlOverride  → write it as chip.yaml, rewrite `chip: "inline"`.
 *  - bare id (e.g. "esp32c3")   → resolve from CHIP_YAMLS, rewrite to chip.yaml.
 *  - path / ".yaml" / "inline"  → leave untouched (CLI resolves on disk).
 *  Throws a listing error on an unknown bare id. */
export function resolveChipInManifest(
  systemYaml: string,
  chipYamlOverride?: string,
): { systemYaml: string; chipYaml?: string } {
  const rewriteToFile = (s: string) =>
    s.replace(/^chip:\s*["']?[A-Za-z0-9_.\-/]+["']?\s*$/m, 'chip: "chip.yaml"');

  if (chipYamlOverride) {
    return { systemYaml: rewriteToFile(systemYaml), chipYaml: chipYamlOverride };
  }
  const m = systemYaml.match(/^chip:\s*["']?([A-Za-z0-9_.\-/]+)["']?\s*$/m);
  if (!m) return { systemYaml };
  const val = m[1];
  if (val === 'inline' || val.includes('/') || val.endsWith('.yaml')) {
    return { systemYaml };
  }
  const yaml = CHIP_YAMLS[val];
  if (!yaml) {
    const known = Object.keys(CHIP_YAMLS).sort().join(', ');
    throw new Error(
      `unknown chip id "${val}". Known chip ids: ${known}. ` +
        'Call labwired_lookup with of:"chips" for ids and their peripheral names.',
    );
  }
  return { systemYaml: rewriteToFile(systemYaml), chipYaml: yaml };
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd ~/projects/labwired-agent-loop/services/labwired-builder && npx vitest run test/chip-resolve.test.ts`
Expected: PASS (4 tests).

- [ ] **Step 5: Wire it into `run()`**

In `run()`, replace the existing chip block (current lines 169-173):

```typescript
    let systemYaml = req.systemYaml;
    if (req.chipYaml) {
      await writeFile(join(tmp, 'chip.yaml'), req.chipYaml);
      systemYaml = systemYaml.replace(/^chip:\s*"inline"\s*$/m, 'chip: "chip.yaml"');
    }
```

with:

```typescript
    const resolved = resolveChipInManifest(req.systemYaml, req.chipYaml);
    const systemYaml = resolved.systemYaml;
    if (resolved.chipYaml) {
      await writeFile(join(tmp, 'chip.yaml'), resolved.chipYaml);
    }
```

- [ ] **Step 6: Run the builder unit suite (binary-free tests must pass)**

Run: `cd ~/projects/labwired-agent-loop/services/labwired-builder && npx vitest run test/chip-resolve.test.ts`
Expected: PASS. (The binary-gated `run.test.ts` cases self-skip when `labwired` is absent.)

- [ ] **Step 7: Commit**

```bash
cd ~/projects/labwired-agent-loop
git add services/labwired-builder/src/run.ts services/labwired-builder/test/chip-resolve.test.ts
git -c user.name=w1ne -c user.email=14119286+w1ne@users.noreply.github.com \
  commit -m "feat(builder): resolve manifest chip by bare id from CHIP_YAMLS"
```

---

## Task 2: Builder — capture stderr and make `config_error` actionable

The Rust CLI's parse error (the field that's wrong) goes to stderr, which `run()` throws away at line 207. Capture it and surface it in the diagnosis, and point the agent at `labwired_lookup`. Export `buildDiagnosis` so it can be unit-tested without the binary.

**Files:**
- Modify: `services/labwired-builder/src/run.ts` (`buildDiagnosis` 80-152; the `config_error` branch 116-121; the exec call 207; result.json read 209-210)
- Test: `services/labwired-builder/test/run.test.ts` (add a binary-free describe block)

**Interfaces:**
- Produces: `export async function buildDiagnosis(stopReason: string, maxSteps: number, cpuState: Record<string, unknown> | null, trace: unknown[] | null, elfPath: string, stderrTail?: string): Promise<RunDiagnosis | undefined>` — the new trailing `stderrTail` param.

- [ ] **Step 1: Write the failing test**

Append to `services/labwired-builder/test/run.test.ts`:

```typescript
import { buildDiagnosis } from '../src/run';

describe('buildDiagnosis config_error (binary-free)', () => {
  it('surfaces the captured stderr detail and points at labwired_lookup', async () => {
    const stderr =
      'Error: unknown field `peripherals`, expected one of `name`, `chip`, `external_devices`, `board_io`';
    const d = await buildDiagnosis('config_error', 1000, null, null, '/nope.elf', stderr);
    expect(d).toBeDefined();
    expect(d!.summary).toContain('unknown field `peripherals`');
    expect(d!.hint).toMatch(/labwired_lookup/);
  });

  it('still returns a useful summary when stderr is empty', async () => {
    const d = await buildDiagnosis('config_error', 1000, null, null, '/nope.elf', '');
    expect(d!.summary.length).toBeGreaterThan(0);
    expect(d!.hint).toMatch(/labwired_lookup/);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd ~/projects/labwired-agent-loop/services/labwired-builder && npx vitest run test/run.test.ts -t config_error`
Expected: FAIL — `buildDiagnosis` is not exported / signature lacks `stderrTail`.

- [ ] **Step 3: Export and extend `buildDiagnosis`**

Change the signature (line 81) from `async function buildDiagnosis(` to `export async function buildDiagnosis(` and add the trailing param. The header becomes:

```typescript
export async function buildDiagnosis(
  stopReason: string,
  maxSteps: number,
  cpuState: Record<string, unknown> | null,
  trace: unknown[] | null,
  elfPath: string,
  stderrTail?: string,
): Promise<RunDiagnosis | undefined> {
```

Replace the `config_error` branch (current lines 116-121) with:

```typescript
  if (stopReason === 'config_error') {
    const detail = (stderrTail ?? '').trim();
    return {
      summary: detail
        ? `Simulation failed at configuration time — the system manifest or chip could not be loaded:\n${detail}`
        : 'Simulation failed at configuration time — the system manifest or chip descriptor could not be loaded.',
      hint:
        'Call labwired_lookup with of:"manifest_schema" for the exact schema + a worked example, ' +
        'and of:"chips" for valid chip ids and their peripheral names. ' +
        'When present, the detail above names the offending field.',
    };
  }
```

- [ ] **Step 4: Capture stderr in `run()` and pass it through**

Replace the exec call (current line 207):

```typescript
    await execFileAsync(bin, args, { timeout: 60000, env: safeEnv() }).catch(() => {});
```

with:

```typescript
    let stderrTail = '';
    try {
      const { stderr } = await execFileAsync(bin, args, { timeout: 60000, env: safeEnv() });
      stderrTail = (stderr ?? '').slice(-2000);
    } catch (e) {
      // The CLI exits non-zero on a sim/config error but still writes result.json.
      stderrTail = ((e as { stderr?: string }).stderr ?? '').slice(-2000);
    }
```

Guard the result.json read (current lines 209-210) so a config error that wrote nothing still yields a diagnosis instead of throwing:

```typescript
    let result: Record<string, unknown>;
    try {
      result = JSON.parse(await readFile(join(outputDir, 'result.json'), 'utf8'));
    } catch {
      // No result.json — the CLI failed before the sim started (a config error).
      return {
        status: 'error',
        stopReason: 'config_error',
        stepsExecuted: 0,
        cycles: 0,
        instructions: 0,
        serial: '',
        peripherals: [],
        timedOut: false,
        diagnosis: await buildDiagnosis('config_error', req.maxSteps, null, null, elfPath, stderrTail),
      };
    }
```

Update the `buildDiagnosis` call (current lines 236-242) to pass `stderrTail`:

```typescript
    const diagnosis = await buildDiagnosis(
      stopReason,
      req.maxSteps,
      cpuState,
      trace,
      elfPath,
      stderrTail,
    );
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cd ~/projects/labwired-agent-loop/services/labwired-builder && npx vitest run test/run.test.ts -t config_error`
Expected: PASS (2 tests).

- [ ] **Step 6: Commit**

```bash
cd ~/projects/labwired-agent-loop
git add services/labwired-builder/src/run.ts services/labwired-builder/test/run.test.ts
git -c user.name=w1ne -c user.email=14119286+w1ne@users.noreply.github.com \
  commit -m "feat(builder): surface captured stderr in config_error diagnosis"
```

---

## Task 3: Builder — `GET /chips` endpoint

Expose the valid chip ids (and their YAML so peripheral names are discoverable) so the gateway's `labwired_lookup of:"chips"` has an authoritative source.

**Files:**
- Modify: `services/labwired-builder/src/server.ts` (add a route; the run route is at lines 70-82)
- Test: `services/labwired-builder/test/server.test.ts`

**Interfaces:**
- Produces: `GET /chips` → `{ chips: Array<{ id: string }> }` (200, no auth — it is non-secret catalog data).

- [ ] **Step 1: Write the failing test**

Append to `services/labwired-builder/test/server.test.ts` (match the file's existing app-bootstrap pattern — reuse however it already obtains the server/handler; the assertion is what matters):

```typescript
import { CHIP_YAMLS } from '../../../packages/board-config/src/chip-yamls';

describe('GET /chips', () => {
  it('lists the bundled chip ids', async () => {
    const res = await fetch(`${BASE}/chips`); // BASE: the test server base already used in this file
    expect(res.status).toBe(200);
    const body = await res.json();
    const ids = body.chips.map((c: { id: string }) => c.id);
    expect(ids).toEqual(expect.arrayContaining(Object.keys(CHIP_YAMLS)));
  });
});
```

> If `server.test.ts` does not already spin up a listening server with a `BASE` URL, call the route handler directly the same way the existing tests in this file invoke `/run` — keep the `expect` on `{ chips: [...] }`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cd ~/projects/labwired-agent-loop/services/labwired-builder && npx vitest run test/server.test.ts -t /chips`
Expected: FAIL — 404 / route not found.

- [ ] **Step 3: Add the route in `src/server.ts`**

Add the import at the top of `src/server.ts`:

```typescript
import { CHIP_YAMLS } from '../../../packages/board-config/src/chip-yamls';
```

Register the handler next to the existing `/run` route (around lines 70-82), following the file's existing routing style (raw `http` switch or framework router — mirror what `/run` uses):

```typescript
    if (req.method === 'GET' && url.pathname === '/chips') {
      const chips = Object.keys(CHIP_YAMLS).sort().map((id) => ({ id }));
      res.writeHead(200, { 'content-type': 'application/json' });
      res.end(JSON.stringify({ chips }));
      return;
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd ~/projects/labwired-agent-loop/services/labwired-builder && npx vitest run test/server.test.ts -t /chips`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cd ~/projects/labwired-agent-loop
git add services/labwired-builder/src/server.ts services/labwired-builder/test/server.test.ts
git -c user.name=w1ne -c user.email=14119286+w1ne@users.noreply.github.com \
  commit -m "feat(builder): add GET /chips catalog endpoint"
```

---

## Task 4: Gateway — `labwired_lookup` tool

One read-only tool the agent calls before authoring: `of:"manifest_schema"` (static schema + worked example using a bare chip id), `of:"chips"` (proxied from builder `/chips`), `of:"examples"` (static curated list).

**Files:**
- Create: `protocat-private/src/lib/mcp/labwired-lookup.ts`
- Modify: `protocat-private/src/lib/mcp/gateway.ts` (PROTOCAT_TOOLS array 246-385; dispatch 567-727; reuse `builderBase`/`builderHeaders` at 37-45)
- Test: `protocat-private/src/lib/mcp/gateway.test.ts`

**Interfaces:**
- Consumes: `builderBase()` and `builderHeaders()` from `gateway.ts` — export them from `gateway.ts` so the lookup module can reach `/chips`.
- Produces: `export async function labwiredLookup(of: string): Promise<unknown>`; `export const MANIFEST_SCHEMA_DOC`, `export const CURATED_EXAMPLES`.

- [ ] **Step 1: Write the failing test**

Append to `protocat-private/src/lib/mcp/gateway.test.ts`:

```typescript
describe("labwired_lookup", () => {
  it("is exposed in the tool list", async () => {
    const client = await connectedClient();
    const names = (await client.listTools()).tools.map((t: { name: string }) => t.name);
    expect(names).toContain("labwired_lookup");
  });

  it("returns the manifest schema with a worked example using a bare chip id", async () => {
    const client = await connectedClient();
    const r = await client.callTool({ name: "labwired_lookup", arguments: { of: "manifest_schema" } });
    const text = r.content.find((c: { type: string }) => c.type === "text").text;
    expect(text).toContain("board_io");
    expect(text).toContain("external_devices");
    // The example must show chip as a bare id, never a filesystem path.
    expect(text).not.toMatch(/chip:\s*["']?\.\.\//);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd ~/projects/protocat-private && npx vitest run src/lib/mcp/gateway.test.ts -t labwired_lookup`
Expected: FAIL — tool not in list.

- [ ] **Step 3: Create the lookup module**

Create `protocat-private/src/lib/mcp/labwired-lookup.ts`:

```typescript
// labwired_lookup payloads: the authoring help an agent needs BEFORE it writes a
// system manifest. Schema + worked example are static (cheap, high-value); the
// chip list is proxied from the builder so ids stay authoritative.
import { builderBase, builderHeaders } from "./gateway";

export const MANIFEST_SCHEMA_DOC = {
  summary:
    "A LabWired system manifest (YAML) describes the chip and what is wired to it. " +
    "Pass it as `system_manifest` to labwired_run_build.",
  schema: {
    name: "string — a label for the build",
    chip: 'string — a bare chip id (e.g. "esp32c3"). Call labwired_lookup of:"chips" for valid ids. NOT a file path.',
    external_devices:
      "array of { id, type, connection (e.g. \"i2c1\"), config:{...} } — peripherals on a bus",
    board_io:
      "array of { id, kind, peripheral (e.g. \"gpioa\"), pin, signal (\"input\"|\"output\"), active_high, i2c_address?, device_type? }",
  },
  example: [
    'name: "blink-and-oled"',
    'chip: "stm32f103"',
    "external_devices:",
    '  - id: "oled"',
    '    type: "oled-ssd1306"',
    '    connection: "i2c1"',
    "    config:",
    "      i2c_address: 0x3C",
    "board_io:",
    '  - id: "led"',
    '    kind: "led"',
    '    peripheral: "gpioa"',
    "    pin: 5",
    '    signal: "output"',
    "    active_high: true",
  ].join("\n"),
};

export const CURATED_EXAMPLES = [
  { example_id: "esp32c3-mlx90640-thermal", summary: "ESP32-C3 + MLX90640 thermal camera over I2C." },
];

export async function labwiredLookup(of: string): Promise<unknown> {
  if (of === "manifest_schema") return MANIFEST_SCHEMA_DOC;
  if (of === "examples") return { examples: CURATED_EXAMPLES };
  if (of === "chips") {
    const resp = await fetch(`${builderBase()}/chips`, { headers: builderHeaders() });
    if (!resp.ok) throw new Error(`chip catalog unavailable (${resp.status})`);
    return await resp.json();
  }
  throw new Error(`unknown lookup target "${of}". Use one of: manifest_schema, chips, examples.`);
}
```

- [ ] **Step 4: Export the builder helpers from `gateway.ts`**

In `gateway.ts`, change `function builderBase()` (line 37) to `export function builderBase()` and `function builderHeaders()` (line 40) to `export function builderHeaders()`.

- [ ] **Step 5: Register the tool in `PROTOCAT_TOOLS`**

Add this object to the `PROTOCAT_TOOLS` array (after the `labwired_run_build` entry, before `labwired_run_example`, around line 314):

```typescript
  {
    name: "labwired_lookup",
    description:
      "Read-only authoring help — CALL THIS BEFORE writing a system manifest. " +
      "of:'manifest_schema' returns the manifest schema + a worked example (chip is a bare id, " +
      "never a path). of:'chips' lists valid chip ids and their peripheral names. " +
      "of:'examples' lists curated example ids for labwired_run_example.",
    inputSchema: {
      type: "object",
      required: ["of"],
      properties: {
        of: { type: "string", enum: ["manifest_schema", "chips", "examples"], description: "What to look up." },
      },
    },
  },
```

- [ ] **Step 6: Dispatch the tool**

Add the import at the top of `gateway.ts` (near line 16):

```typescript
import { labwiredLookup } from "./labwired-lookup";
```

Add this handler inside the `CallToolRequestSchema` handler (e.g. right after the `protocat_plan_device` block, around line 574):

```typescript
      if (name === "labwired_lookup") {
        const a = (args ?? {}) as { of?: string };
        if (!a.of) throw new Error("missing of");
        return text(await labwiredLookup(a.of));
      }
```

- [ ] **Step 7: Run test to verify it passes**

Run: `cd ~/projects/protocat-private && npx vitest run src/lib/mcp/gateway.test.ts -t labwired_lookup`
Expected: PASS (2 tests). (The `of:"chips"` path is covered by the live smoke in Task 6, since it needs the builder.)

- [ ] **Step 8: Commit**

```bash
cd ~/projects/protocat-private
git add src/lib/mcp/labwired-lookup.ts src/lib/mcp/gateway.ts src/lib/mcp/gateway.test.ts
git -c user.name="Andrii Shylenko" -c user.email=14119286+w1ne@users.noreply.github.com \
  commit -m "feat(gateway): add labwired_lookup authoring tool"
```

---

## Task 5: Gateway — point `plan_device` and `labwired_run_build` at the lookup

Close the discovery loop: the planner and the run tool must tell the agent to call `labwired_lookup` first, and stop implying the manifest is self-evident.

**Files:**
- Modify: `protocat-private/src/lib/mcp/gateway.ts` (`planDevice` electronics phase line 237; `labwired_run_build` description 294-300)
- Test: `protocat-private/src/lib/mcp/gateway.test.ts`

- [ ] **Step 1: Write the failing test**

Append to `gateway.test.ts`:

```typescript
describe("plan_device points at labwired_lookup", () => {
  it("the electronics phase lists labwired_lookup", async () => {
    const client = await connectedClient();
    const r = await client.callTool({ name: "protocat_plan_device", arguments: { goal: "blink an LED" } });
    const text = r.content.find((c: { type: string }) => c.type === "text").text;
    const plan = JSON.parse(text);
    const electronics = plan.phases.find((p: { id: string }) => p.id === "electronics");
    expect(electronics.tools).toContain("labwired_lookup");
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd ~/projects/protocat-private && npx vitest run src/lib/mcp/gateway.test.ts -t "points at labwired_lookup"`
Expected: FAIL — `labwired_lookup` not in the electronics `tools` array.

- [ ] **Step 3: Update the electronics phase**

In `planDevice` (line 237), change the electronics phase to:

```typescript
      { id: "electronics", title: "Electronics — assemble hardware & validate firmware on it", intent: "FIRST call labwired_lookup (of:'manifest_schema' and of:'chips') to learn the manifest schema and valid chip ids. Then assemble the system manifest, RUN the compiled ELF against it, and require real execution proof.", tools: ["labwired_lookup", "labwired_run_build"], done_when: "The oracle reports the firmware actually exercised the hardware. No false positives." },
```

- [ ] **Step 4: Update the `labwired_run_build` description**

In the `labwired_run_build` tool definition (lines 294-300), append to the description string (before `"Use this to PROVE…"`):

```typescript
      "Author system_manifest from labwired_lookup of:'manifest_schema'; set `chip` to a bare id from " +
      "labwired_lookup of:'chips' (a path is NOT accepted). " +
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cd ~/projects/protocat-private && npx vitest run src/lib/mcp/gateway.test.ts`
Expected: PASS (full gateway suite green).

- [ ] **Step 6: Commit**

```bash
cd ~/projects/protocat-private
git add src/lib/mcp/gateway.ts src/lib/mcp/gateway.test.ts
git -c user.name="Andrii Shylenko" -c user.email=14119286+w1ne@users.noreply.github.com \
  commit -m "feat(gateway): point plan_device + run_build at labwired_lookup"
```

---

## Task 6: End-to-end regression — replay the dogfood against live services

The original dead-end was found by driving the live MCP. Prove it's closed the same way. This is a manual/integration smoke (needs the deployed gateway + builder), captured as a script so it can be re-run.

**Files:**
- Create: `services/labwired-builder/scripts/agent-loop-smoke.sh` (a documented, re-runnable smoke)

- [ ] **Step 1: Write the smoke script**

Create `services/labwired-builder/scripts/agent-loop-smoke.sh`:

```bash
#!/usr/bin/env bash
# Replays the agent authoring loop against the live MCP gateway to prove the
# Part-A dead-ends are closed. Requires curl + python3. Override MCP_URL to test
# a non-prod gateway.
set -euo pipefail
MCP_URL="${MCP_URL:-https://mcp.proto.cat/mcp}"
call() { # $1=tool $2=json-args
  curl -s --max-time 90 -X POST "$MCP_URL" \
    -H 'content-type: application/json' \
    -H 'accept: application/json, text/event-stream' \
    -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"$1\",\"arguments\":$2}}"
}

echo "1) labwired_lookup of:manifest_schema must return a schema"
call labwired_lookup '{"of":"manifest_schema"}' | grep -q board_io && echo "  OK: schema returned" || { echo "  FAIL"; exit 1; }

echo "2) labwired_lookup of:chips must list chip ids"
call labwired_lookup '{"of":"chips"}' | grep -q '"chips"' && echo "  OK: chips returned" || { echo "  FAIL"; exit 1; }

echo "3) A bad manifest must now name the offending field (not the generic string)"
BAD='{"firmware_source":"#include <Arduino.h>\nvoid setup(){}\nvoid loop(){}\n","chip_id":"esp32","system_manifest":"chip: esp32\nperipherals:\n  - type: led\n    pin: 2\n"}'
OUT="$(call labwired_run_build "$BAD")"
echo "$OUT" | grep -qiE 'labwired_lookup|unknown field|chip id' \
  && echo "  OK: actionable error" \
  || { echo "  FAIL — still generic:"; echo "$OUT" | head -c 500; exit 1; }

echo "ALL SMOKE CHECKS PASSED"
```

- [ ] **Step 2: Make it executable and run it (after both repos are deployed)**

Run:
```bash
chmod +x ~/projects/labwired-agent-loop/services/labwired-builder/scripts/agent-loop-smoke.sh
~/projects/labwired-agent-loop/services/labwired-builder/scripts/agent-loop-smoke.sh
```
Expected: `ALL SMOKE CHECKS PASSED`. (Run against a staging `MCP_URL` first if prod deploy is gated.)

- [ ] **Step 3: Commit**

```bash
cd ~/projects/labwired-agent-loop
git add services/labwired-builder/scripts/agent-loop-smoke.sh
git -c user.name=w1ne -c user.email=14119286+w1ne@users.noreply.github.com \
  commit -m "test(builder): add agent-loop authoring smoke script"
```

---

## Investigation (feeds Part B — NOT a code task)

Before Part B (async long-runs) can be planned with concrete steps, confirm where the ~60s drop actually is. It is in neither repo's code (gateway fetch timeouts are 600s; Next route `maxDuration=300`). Hypothesis: a reverse-proxy idle timeout on Europa (157.180.86.12) in front of the gateway and/or the builder.

Probe (read-only):
- Time the failure precisely: `for s in 30 55 70 120; do curl -s -o /dev/null -w "%{http_code} %{time_total}\n" --max-time $((s+30)) ... ` against a run that takes `s` seconds; find the exact cliff.
- On Europa, inspect the reverse proxy: look for Traefik/Caddy/nginx config and any `readTimeout`/`proxy_read_timeout`/`responseTimeout` (≈60s default). Check `protocat-next.service` and the builder container's front proxy.
- Deliverable: a one-page note in `docs/superpowers/specs/` naming the exact component + timeout value, which decides Part B's design (job-queue+poll vs raise-the-ceiling).

---

## Self-Review

**Spec coverage:**
- Spec gap 1 (manifest unguessable) → Task 4 (`labwired_lookup of:manifest_schema`) + Task 5 (planner points at it).
- Spec gap 2 (chip is a path) → Task 1 (bare-id resolution) + Task 4/5 (schema says "id, not path").
- Spec gap 3 (non-actionable `config_error`) → Task 2 (captured stderr + lookup hint).
- Spec gap 4 (60s timeout) → explicitly deferred to Part B; investigation task scopes it. Not coded here (by design — measure first).
- Spec gap 5 (ESP32 support matrix) → Task 4 `of:"chips"` returns the authoritative id list (folds in as data); honest matrix is the chip list itself.
- Regression (re-run dogfood) → Task 6.

**Placeholder scan:** No TBD/TODO. Task 3's test note ("mirror how `/run` is invoked") is a real instruction tied to reading the existing file, not a placeholder — the assertion and route code are concrete.

**Type consistency:** `resolveChipInManifest` (Task 1) returns `{ systemYaml, chipYaml? }`, consumed verbatim in `run()` Step 5. `buildDiagnosis` trailing `stderrTail?: string` (Task 2) matches every call site updated. `labwiredLookup(of: string)` (Task 4) matches the dispatch in Task 4 Step 6. `builderBase`/`builderHeaders` exported in Task 4 Step 4 and consumed in `labwired-lookup.ts`.

**Scope:** Part A only — authoring. Part B (async) is a separate plan, gated on the investigation. Each task ends with an independently testable deliverable and a commit.
