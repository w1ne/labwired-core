# Hosted MCP build→run loop — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let an agent drive write→compile→build-device→run→read-results headlessly through the hosted MCP, for the single v1 path C/C++ → `stm32l476`.

**Architecture:** The existing Cloudflare Worker (`packages/api`) stays a thin authenticated front door: it routes MCP tool calls, converts a diagram to a `system.yaml`, enforces quota, and proxies compile/run over a Cloudflare Tunnel to a new stateless `labwired-builder` HTTP service on a Hetzner box. The builder runs `arm-none-eabi-gcc` (compile) and the native `labwired` CLI (run) and returns bytes/JSON. No server-side ELF handles.

**Tech Stack:** TypeScript (Worker + builder, Node 20), Cloudflare Workers/Wrangler, `arm-none-eabi-gcc`/`objcopy`, the native `labwired` Rust CLI, `cloudflared`, Vitest.

**Spec:** `docs/superpowers/specs/2026-05-31-hosted-mcp-build-run-loop-design.md`

---

## File Structure

**New — shared converter package (`packages/board-config/`):**
- `src/types.ts` — `Diagram`, `Part`, `Wire`, `BoardIoKind` (copied, dependency-free; no React).
- `src/component-meta.ts` — pure map `componentType → { boardIoKind?, pins? }` extracted from the editor registry (no `.tsx`).
- `src/pin-mapping.ts` — pure `findPinFunction(board, pin, fn)` moved out of `packages/ui`.
- `src/chip-yamls.ts` — the `CHIP_YAMLS` record (string templates), incl. a new `stm32l476` entry.
- `src/diagram-to-config.ts` — `diagramToConfig(diagram, chipYamlOverride?)` (logic moved from `packages/ui/src/editor/diagramToConfig.ts`).
- `src/index.ts` — re-exports.

**New — builder service (`services/labwired-builder/`):**
- `src/server.ts` — HTTP server: `POST /compile`, `POST /run`, `GET /healthz`; secret check.
- `src/compile.ts` — sandboxed `arm-none-eabi-gcc` invocation + diagnostic parsing.
- `src/run.ts` — invokes the native `labwired` CLI, parses `result.json` + `uart.log` + snapshot.
- `scaffold/stm32l476/` — `startup.c` (vector table + reset → `main`), `stm32l476.ld` (linker), adapted from `packages/playground/server/arduino-core/`.
- `test/compile.test.ts`, `test/run.test.ts`, `test/sandbox.test.ts`.
- `deploy/RUNBOOK.md`, `deploy/labwired-builder.service`, `deploy/cloudflared-config.yml`.

**Modified — Worker (`packages/api/`):**
- `src/types.ts` — add `BUILDER_URL`, `BUILDER_SECRET` to `Env`.
- `src/mcp/builder-client.ts` — NEW: typed fetch wrapper to the builder (adds secret header).
- `src/mcp/tools.ts` — add `labwired_compile`, `labwired_run`, `labwired_list_components`; expand `labwired_list_boards`.
- `src/keys.ts` — add `checkAndCountCompile` (rate-limit only) + reuse run metering.
- `package.json` — depend on `@labwired/board-config`.

---

## Phase 1 — Shared `@labwired/board-config` package

### Task 1: Scaffold the package

**Files:**
- Create: `packages/board-config/package.json`
- Create: `packages/board-config/tsconfig.json`
- Create: `packages/board-config/vitest.config.ts`

- [ ] **Step 1: Create `package.json`**

```json
{
  "name": "@labwired/board-config",
  "version": "0.1.0",
  "type": "module",
  "main": "src/index.ts",
  "types": "src/index.ts",
  "exports": { ".": "./src/index.ts" },
  "scripts": { "test": "vitest run", "typecheck": "tsc --noEmit" },
  "devDependencies": { "typescript": "~5.9.3", "vitest": "~3.2.0" }
}
```

- [ ] **Step 2: Create `tsconfig.json`**

```json
{
  "compilerOptions": {
    "target": "ES2022", "module": "ESNext", "moduleResolution": "Bundler",
    "strict": true, "noEmit": true, "skipLibCheck": true, "types": []
  },
  "include": ["src", "test"]
}
```

- [ ] **Step 3: Create `vitest.config.ts`**

```typescript
import { defineConfig } from 'vitest/config';
export default defineConfig({ test: { include: ['test/**/*.test.ts'] } });
```

- [ ] **Step 4: Install & verify**

Run: `cd packages/board-config && npm install`
Expected: installs with no errors.

- [ ] **Step 5: Commit**

```bash
git add packages/board-config/package.json packages/board-config/tsconfig.json packages/board-config/vitest.config.ts
git commit -m "chore(board-config): scaffold shared diagram→config package"
```

### Task 2: Copy dependency-free types

**Files:**
- Create: `packages/board-config/src/types.ts`

- [ ] **Step 1: Write the types** (copied from `packages/ui/src/editor/types.ts`, only what the converter needs)

```typescript
export type BoardIoKind =
  | 'led' | 'button' | 'adc_input' | 'pwm_output' | 'i2c_device' | 'spi_device';

export interface Part { id: string; type: string; attrs?: Record<string, string>; }
export interface WireEndpoint { part: string; pin: string; }
export interface Wire { from: WireEndpoint; to: WireEndpoint; }
export interface Diagram { board: string; parts: Part[]; wires: Wire[]; }
```

- [ ] **Step 2: Typecheck**

Run: `cd packages/board-config && npx tsc --noEmit`
Expected: PASS (no errors).

- [ ] **Step 3: Commit**

```bash
git add packages/board-config/src/types.ts
git commit -m "feat(board-config): dependency-free diagram types"
```

### Task 3: Extract pure component metadata

**Files:**
- Create: `packages/board-config/src/component-meta.ts`
- Test: `packages/board-config/test/component-meta.test.ts`

Read `packages/ui/src/editor/components/*.tsx` and copy ONLY the `type` + `boardIoKind` + pin labels for each component into a plain data map. This is the data the converter needs; it must NOT import the React `.tsx` modules.

- [ ] **Step 1: Write the failing test**

```typescript
import { describe, it, expect } from 'vitest';
import { COMPONENT_META } from '../src/component-meta';

describe('COMPONENT_META', () => {
  it('marks an LED as a board_io led', () => {
    expect(COMPONENT_META['led']?.boardIoKind).toBe('led');
  });
  it('marks the PCD8544 as an spi_device', () => {
    expect(COMPONENT_META['pcd8544']?.boardIoKind).toBe('spi_device');
  });
  it('has no React/render fields', () => {
    expect((COMPONENT_META['led'] as Record<string, unknown>).render).toBeUndefined();
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd packages/board-config && npx vitest run test/component-meta.test.ts`
Expected: FAIL — cannot find `../src/component-meta`.

- [ ] **Step 3: Write the implementation** (enumerate every component type present in `packages/ui/src/editor/components/index.ts`; values from each component's `boardIoKind`)

```typescript
import type { BoardIoKind } from './types';

export interface ComponentMeta { boardIoKind?: BoardIoKind; }

/** Pure metadata mirror of COMPONENT_REGISTRY — no React. Keep in sync when
 *  editor components are added. */
export const COMPONENT_META: Record<string, ComponentMeta> = {
  led: { boardIoKind: 'led' },
  button: { boardIoKind: 'button' },
  pcd8544: { boardIoKind: 'spi_device' },
  'oled-ssd1306': { boardIoKind: 'i2c_device' },
  ultrasonic: {}, // HC-SR04: handled via dedicated trig/echo wiring, not board_io
  potentiometer: { boardIoKind: 'adc_input' },
  'ntc-thermistor': { boardIoKind: 'adc_input' },
  buzzer: { boardIoKind: 'pwm_output' },
  // ... one entry per type in packages/ui/src/editor/components/index.ts
};
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd packages/board-config && npx vitest run test/component-meta.test.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add packages/board-config/src/component-meta.ts packages/board-config/test/component-meta.test.ts
git commit -m "feat(board-config): pure component metadata (no React)"
```

### Task 4: Move pin-mapping

**Files:**
- Create: `packages/board-config/src/pin-mapping.ts`
- Test: `packages/board-config/test/pin-mapping.test.ts`

- [ ] **Step 1: Write the failing test**

```typescript
import { describe, it, expect } from 'vitest';
import { findPinFunction } from '../src/pin-mapping';

describe('findPinFunction', () => {
  it('resolves an ADC function for an stm32l476 analog pin', () => {
    // PA0 is ADC12_IN5 on L476; exact expectation per pin-mapping table.
    expect(findPinFunction('stm32l476', 'PA0', 'adc')).toBeTruthy();
  });
  it('returns null for a pin with no such function', () => {
    expect(findPinFunction('stm32l476', 'PA0', 'i2c')).toBeNull();
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd packages/board-config && npx vitest run test/pin-mapping.test.ts`
Expected: FAIL — module not found.

- [ ] **Step 3: Move the implementation**

Copy `packages/ui/src/editor/pin-mapping.ts` verbatim to `packages/board-config/src/pin-mapping.ts` (it is already pure data + functions). Confirm it has no React imports. Add an `stm32l476` entry to its pin table if absent (mirror the `stm32f401`/`nucleo-f401re` entries).

- [ ] **Step 4: Run test to verify it passes**

Run: `cd packages/board-config && npx vitest run test/pin-mapping.test.ts`
Expected: PASS.

- [ ] **Step 5: Re-export from `packages/ui` to avoid duplication**

Replace the body of `packages/ui/src/editor/pin-mapping.ts` with:
```typescript
export { findPinFunction, getPinMapping } from '@labwired/board-config';
export type { PinFunction, PinMapping } from '@labwired/board-config';
```
Add `"@labwired/board-config": "*"` to `packages/ui/package.json` dependencies.

- [ ] **Step 6: Run UI build to confirm no regression**

Run: `cd packages/ui && npx tsc -b`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add packages/board-config/src/pin-mapping.ts packages/board-config/test/pin-mapping.test.ts packages/ui/src/editor/pin-mapping.ts packages/ui/package.json
git commit -m "refactor(board-config): own pin-mapping; ui re-exports it"
```

### Task 5: Move chip YAMLs + add stm32l476

**Files:**
- Create: `packages/board-config/src/chip-yamls.ts`
- Test: `packages/board-config/test/chip-yamls.test.ts`

- [ ] **Step 1: Write the failing test**

```typescript
import { describe, it, expect } from 'vitest';
import { CHIP_YAMLS } from '../src/chip-yamls';

describe('CHIP_YAMLS', () => {
  it('has an stm32l476 entry with the correct flash/ram base', () => {
    const y = CHIP_YAMLS['stm32l476'];
    expect(y).toContain('0x08000000'); // flash base
    expect(y).toContain('0x20000000'); // ram base
    expect(y).toContain('arch: "arm"');
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd packages/board-config && npx vitest run test/chip-yamls.test.ts`
Expected: FAIL.

- [ ] **Step 3: Write the implementation**

Copy the `CHIP_YAMLS` object from `packages/ui/src/editor/diagramToConfig.ts` into `chip-yamls.ts`. Add an `stm32l476` entry whose body mirrors the committed `core/configs/chips/stm32l476.yaml` (flash `0x08000000` size `1MB`, ram `0x20000000` size `96KB`, plus `rcc`, `gpioa..h`, `systick`, `usart2`, `spi1`). Use that file as the source of truth for peripheral base addresses.

- [ ] **Step 4: Run test to verify it passes**

Run: `cd packages/board-config && npx vitest run test/chip-yamls.test.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add packages/board-config/src/chip-yamls.ts packages/board-config/test/chip-yamls.test.ts
git commit -m "feat(board-config): chip YAML templates incl. stm32l476"
```

### Task 6: Move `diagramToConfig` + re-export from ui

**Files:**
- Create: `packages/board-config/src/diagram-to-config.ts`
- Create: `packages/board-config/src/index.ts`
- Test: `packages/board-config/test/diagram-to-config.test.ts`
- Modify: `packages/ui/src/editor/diagramToConfig.ts`

- [ ] **Step 1: Write the failing test**

```typescript
import { describe, it, expect } from 'vitest';
import { diagramToConfig } from '../src/diagram-to-config';

const diagram = {
  board: 'stm32l476',
  parts: [{ id: 'led1', type: 'led' }],
  wires: [{ from: { part: 'mcu', pin: 'PA5' }, to: { part: 'led1', pin: 'A' } }],
};

describe('diagramToConfig', () => {
  it('emits a system.yaml with the LED as a board_io led on gpioa pin 5', () => {
    const { systemYaml, chipYaml } = diagramToConfig(diagram);
    expect(systemYaml).toContain('id: "led1"');
    expect(systemYaml).toContain('gpioa');
    expect(systemYaml).toContain('pin: 5');
    expect(chipYaml).toContain('0x08000000');
  });
  it('throws on an unknown board', () => {
    expect(() => diagramToConfig({ board: 'nope', parts: [], wires: [] })).toThrow(/Unknown board/);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd packages/board-config && npx vitest run test/diagram-to-config.test.ts`
Expected: FAIL.

- [ ] **Step 3: Move the implementation**

Copy the body of `packages/ui/src/editor/diagramToConfig.ts` into `packages/board-config/src/diagram-to-config.ts`, changing its imports to local ones:
```typescript
import type { Diagram } from './types';
import { COMPONENT_META } from './component-meta';
import { findPinFunction } from './pin-mapping';
import { CHIP_YAMLS } from './chip-yamls';
```
Replace registry lookups `COMPONENT_REGISTRY.get(part.type)?.boardIoKind` with `COMPONENT_META[part.type]?.boardIoKind`. Keep `parseMcuPin`/`board_io` logic identical.

- [ ] **Step 4: Write `src/index.ts`**

```typescript
export { diagramToConfig } from './diagram-to-config';
export { CHIP_YAMLS } from './chip-yamls';
export { findPinFunction, getPinMapping } from './pin-mapping';
export { COMPONENT_META } from './component-meta';
export type { Diagram, Part, Wire, WireEndpoint, BoardIoKind } from './types';
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cd packages/board-config && npx vitest run`
Expected: PASS (all board-config tests).

- [ ] **Step 6: Re-export from ui (DRY)**

Replace the body of `packages/ui/src/editor/diagramToConfig.ts` with:
```typescript
export { diagramToConfig, CHIP_YAMLS } from '@labwired/board-config';
```
Run: `cd packages/ui && npx tsc -b` → Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add packages/board-config/src packages/board-config/test/diagram-to-config.test.ts packages/ui/src/editor/diagramToConfig.ts
git commit -m "refactor(board-config): own diagramToConfig; ui re-exports it"
```

---

## Phase 2 — `labwired-builder` service

### Task 7: stm32l476 C scaffold

**Files:**
- Create: `services/labwired-builder/scaffold/stm32l476/stm32l476.ld`
- Create: `services/labwired-builder/scaffold/stm32l476/startup.c`

- [ ] **Step 1: Write the linker script**

Adapt `packages/playground/server/arduino-core/stm32f103.ld` for L476. Change the memory regions to:
```
MEMORY {
  FLASH (rx) : ORIGIN = 0x08000000, LENGTH = 1024K
  RAM  (rwx) : ORIGIN = 0x20000000, LENGTH = 96K
}
```
Keep the existing `.text/.data/.bss` section layout and `_estack = ORIGIN(RAM) + LENGTH(RAM);`.

- [ ] **Step 2: Write `startup.c`** (minimal Cortex-M4 vector table + reset handler that zeroes .bss, copies .data, calls `main`)

Adapt `packages/playground/server/arduino-core/main.c` (the existing reset/vector wrapper). The vector table's first two entries must be `_estack` then `Reset_Handler`; `Reset_Handler` initializes memory then calls `extern int main(void)`. No libc.

- [ ] **Step 3: Sanity-compile the scaffold with a trivial main**

Run:
```bash
cd services/labwired-builder
echo 'int main(void){ volatile int x=0; for(;;) x++; }' > /tmp/m.c
arm-none-eabi-gcc -mcpu=cortex-m4 -mthumb -ffreestanding -nostdlib -ffunction-sections -fdata-sections \
  -Wl,--gc-sections -T scaffold/stm32l476/stm32l476.ld scaffold/stm32l476/startup.c /tmp/m.c -o /tmp/m.elf && echo OK
```
Expected: `OK`, and `arm-none-eabi-readelf -h /tmp/m.elf` shows `Machine: ARM`, entry in flash.

- [ ] **Step 4: Commit**

```bash
git add services/labwired-builder/scaffold/stm32l476
git commit -m "feat(builder): stm32l476 C scaffold (linker + startup)"
```

### Task 8: Builder package scaffold

**Files:**
- Create: `services/labwired-builder/package.json`
- Create: `services/labwired-builder/tsconfig.json`
- Create: `services/labwired-builder/vitest.config.ts`

- [ ] **Step 1: `package.json`**

```json
{
  "name": "labwired-builder",
  "version": "0.1.0",
  "type": "module",
  "scripts": {
    "start": "node --import tsx src/server.ts",
    "test": "vitest run",
    "typecheck": "tsc --noEmit"
  },
  "dependencies": { "tsx": "^4.20.0" },
  "devDependencies": { "@types/node": "^22.10.0", "typescript": "~5.9.3", "vitest": "~3.2.0" }
}
```

- [ ] **Step 2: `tsconfig.json`** (Node, strict)

```json
{ "compilerOptions": { "target": "ES2022", "module": "ESNext", "moduleResolution": "Bundler", "strict": true, "noEmit": true, "types": ["node"], "skipLibCheck": true }, "include": ["src", "test"] }
```

- [ ] **Step 3: `vitest.config.ts`**

```typescript
import { defineConfig } from 'vitest/config';
export default defineConfig({ test: { include: ['test/**/*.test.ts'], testTimeout: 30000 } });
```

- [ ] **Step 4: Install**

Run: `cd services/labwired-builder && npm install` → Expected: OK.

- [ ] **Step 5: Commit**

```bash
git add services/labwired-builder/package.json services/labwired-builder/tsconfig.json services/labwired-builder/vitest.config.ts
git commit -m "chore(builder): scaffold service package"
```

### Task 9: Compile module

**Files:**
- Create: `services/labwired-builder/src/compile.ts`
- Test: `services/labwired-builder/test/compile.test.ts`

- [ ] **Step 1: Write the failing test**

```typescript
import { describe, it, expect } from 'vitest';
import { compile } from '../src/compile';

describe('compile', () => {
  it('compiles a trivial C main to an ELF', async () => {
    const r = await compile({ source: 'int main(void){for(;;){}}', language: 'c', target: 'stm32l476' });
    expect(r.ok).toBe(true);
    expect(r.elfBase64 && r.elfBase64.length).toBeGreaterThan(0);
    expect(r.errors).toHaveLength(0);
  });
  it('returns structured diagnostics on a syntax error', async () => {
    const r = await compile({ source: 'int main(void){ return }', language: 'c', target: 'stm32l476' });
    expect(r.ok).toBe(false);
    expect(r.errors.some((e) => e.severity === 'error')).toBe(true);
    expect(r.errors[0].message).toBeTruthy();
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd services/labwired-builder && npx vitest run test/compile.test.ts`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement `compile.ts`** (model on `packages/playground/server/compile-server.ts`, retargeted to cortex-m4 + the l476 scaffold, with structured diagnostics)

```typescript
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { writeFile, readFile, mkdir, rm } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { randomUUID } from 'node:crypto';
import { fileURLToPath } from 'node:url';

const execFileAsync = promisify(execFile);
const HERE = dirname(fileURLToPath(import.meta.url));
const SCAFFOLD = join(HERE, '..', 'scaffold');

export interface CompileRequest { source: string; language: 'c' | 'cpp'; target: 'stm32l476'; }
export interface Diagnostic { file?: string; line?: number; col?: number; severity: 'error' | 'warning'; message: string; }
export interface CompileResult { ok: boolean; elfBase64?: string; sizeBytes?: number; errors: Diagnostic[]; }

const TARGETS: Record<string, { cpu: string; dir: string }> = {
  stm32l476: { cpu: 'cortex-m4', dir: 'stm32l476' },
};

function parseDiagnostics(stderr: string, srcName: string): Diagnostic[] {
  const out: Diagnostic[] = [];
  for (const line of stderr.split('\n')) {
    const m = line.match(/^(.*?):(\d+):(\d+):\s+(error|warning):\s+(.*)$/);
    if (m) out.push({ file: srcName, line: +m[2], col: +m[3], severity: m[4] as 'error' | 'warning', message: m[5] });
  }
  return out;
}

export async function compile(req: CompileRequest): Promise<CompileResult> {
  const t = TARGETS[req.target];
  if (!t) return { ok: false, errors: [{ severity: 'error', message: `Unsupported target: ${req.target}` }] };
  const tmp = join('/tmp', `lwb-${randomUUID()}`);
  await mkdir(tmp, { recursive: true });
  try {
    const ext = req.language === 'cpp' ? '.cpp' : '.c';
    const srcName = `sketch${ext}`;
    const src = join(tmp, srcName);
    const elf = join(tmp, 'firmware.elf');
    await writeFile(src, req.source);
    const cc = req.language === 'cpp' ? 'arm-none-eabi-g++' : 'arm-none-eabi-gcc';
    const scaffoldDir = join(SCAFFOLD, t.dir);
    const args = [
      `-mcpu=${t.cpu}`, '-mthumb', '-g', '-O1', '-ffreestanding', '-nostdlib',
      '-ffunction-sections', '-fdata-sections', '-Wall',
      req.language === 'cpp' ? '-std=c++17' : '-std=c11',
      ...(req.language === 'cpp' ? ['-fno-exceptions', '-fno-rtti'] : []),
      join(scaffoldDir, 'startup.c'), src,
      '-Wl,--gc-sections', `-T${join(scaffoldDir, `${t.dir}.ld`)}`, '-o', elf,
    ];
    try {
      await execFileAsync(cc, args, { timeout: 15000, env: { PATH: process.env.PATH } });
    } catch (e) {
      const err = e as { stderr?: string };
      const diags = parseDiagnostics(err.stderr ?? '', srcName);
      return { ok: false, errors: diags.length ? diags : [{ severity: 'error', message: (err.stderr ?? 'compile failed').slice(0, 2000) }] };
    }
    const bytes = await readFile(elf);
    return { ok: true, elfBase64: bytes.toString('base64'), sizeBytes: bytes.length, errors: [] };
  } finally {
    await rm(tmp, { recursive: true, force: true }).catch(() => {});
  }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd services/labwired-builder && npx vitest run test/compile.test.ts`
Expected: PASS (requires `arm-none-eabi-gcc` on PATH — see Task 14).

- [ ] **Step 5: Commit**

```bash
git add services/labwired-builder/src/compile.ts services/labwired-builder/test/compile.test.ts
git commit -m "feat(builder): /compile core — gcc + structured diagnostics"
```

### Task 10: Run module

**Files:**
- Create: `services/labwired-builder/src/run.ts`
- Test: `services/labwired-builder/test/run.test.ts`

The native CLI `Run` subcommand takes `-s <chip.yaml> -f <fw.elf> --max-steps N`; the `Test` subcommand writes `result.json` (schema `TestResult`: `status`, `cycles`, `instructions`, `stop_reason`, `assertions`, `cpu_state`) + `uart.log` to an `--artifacts` dir. Use the `Test` path for structured output. Reference an existing test-script under `core/crates/cli/tests/` (or `core/examples/*/`) for the script YAML format; synthesize a minimal script with a single `max_steps` stop condition and no assertions.

- [ ] **Step 1: Write the failing test** (uses a prebuilt ELF fixture committed under `test/fixtures/`)

```typescript
import { describe, it, expect } from 'vitest';
import { readFile } from 'node:fs/promises';
import { run } from '../src/run';

describe('run', () => {
  it('runs a known stm32l476 ELF and returns status + serial', async () => {
    const elfBase64 = (await readFile(`${__dirname}/fixtures/blink-l476.elf`)).toString('base64');
    const systemYaml = await readFile(`${__dirname}/fixtures/blink-l476.system.yaml`, 'utf8');
    const r = await run({ elfBase64, systemYaml, maxSteps: 200000 });
    expect(['finished', 'step_limit', 'halted']).toContain(r.status);
    expect(typeof r.serial).toBe('string');
    expect(r.cycles).toBeGreaterThan(0);
    expect(r.timedOut).toBe(r.stopReason === 'step_limit');
  });
});
```

- [ ] **Step 2: Generate the fixtures**

Compile a blink with Task 9's `compile()` for stm32l476 → write `test/fixtures/blink-l476.elf`. Hand-write `test/fixtures/blink-l476.system.yaml` (an LED on PA5, mirroring `core/examples/*/system.yaml`).

- [ ] **Step 3: Run test to verify it fails**

Run: `cd services/labwired-builder && npx vitest run test/run.test.ts`
Expected: FAIL — module not found.

- [ ] **Step 4: Implement `run.ts`**

```typescript
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { writeFile, readFile, mkdir, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { randomUUID } from 'node:crypto';

const execFileAsync = promisify(execFile);
const LABWIRED = process.env.LABWIRED_BIN ?? 'labwired';

export interface RunRequest { elfBase64: string; systemYaml: string; maxSteps: number; }
export interface PeripheralState { id: string; type: string; state: unknown; }
export interface RunResult {
  status: string; stopReason: string; stepsExecuted: number; cycles: number; instructions: number;
  serial: string; peripherals: PeripheralState[]; timedOut: boolean;
}

export async function run(req: RunRequest): Promise<RunResult> {
  const tmp = join('/tmp', `lwr-${randomUUID()}`);
  const artifacts = join(tmp, 'artifacts');
  await mkdir(artifacts, { recursive: true });
  try {
    const elf = join(tmp, 'fw.elf');
    const sys = join(tmp, 'system.yaml');
    const script = join(tmp, 'script.yaml');
    await writeFile(elf, Buffer.from(req.elfBase64, 'base64'));
    await writeFile(sys, req.systemYaml);
    // Minimal test script: stop at max_steps, no assertions. Format per core/crates/cli/tests.
    await writeFile(script, `max_steps: ${req.maxSteps}\nassertions: []\n`);
    await execFileAsync(LABWIRED, ['test', '-s', sys, '-f', elf, '--script', script, '--artifacts', artifacts],
      { timeout: 60000, env: { PATH: process.env.PATH } }).catch(() => {});
    const result = JSON.parse(await readFile(join(artifacts, 'result.json'), 'utf8'));
    const serial = await readFile(join(artifacts, 'uart.log'), 'utf8').catch(() => '');
    return {
      status: result.status, stopReason: result.stop_reason,
      stepsExecuted: result.steps_executed, cycles: result.cycles, instructions: result.instructions,
      serial, peripherals: result.peripherals ?? [],
      timedOut: result.stop_reason === 'step_limit',
    };
  } finally {
    await rm(tmp, { recursive: true, force: true }).catch(() => {});
  }
}
```

> **Implementation note:** confirm the exact `test` subcommand flag names (`--script`, `--artifacts`) and the `script.yaml` schema against `core/crates/cli/src/main.rs` (`Commands::Test` → `run_test`) before finalizing; adjust field names in the parse to match the real `TestResult` JSON keys.

- [ ] **Step 5: Run test to verify it passes**

Run: `cd services/labwired-builder && npx vitest run test/run.test.ts`
Expected: PASS (requires the `labwired` binary — see Task 14).

- [ ] **Step 6: Commit**

```bash
git add services/labwired-builder/src/run.ts services/labwired-builder/test/run.test.ts services/labwired-builder/test/fixtures
git commit -m "feat(builder): /run core — native CLI → result.json + uart.log"
```

### Task 11: HTTP server + secret + sandbox limits

**Files:**
- Create: `services/labwired-builder/src/server.ts`
- Test: `services/labwired-builder/test/server.test.ts`
- Test: `services/labwired-builder/test/sandbox.test.ts`

- [ ] **Step 1: Write the failing server test**

```typescript
import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import { makeServer } from '../src/server';
let url: string; let close: () => void;
beforeAll(async () => { const s = await makeServer({ secret: 's3cret' }); url = s.url; close = s.close; });
afterAll(() => close());

describe('builder server', () => {
  it('rejects a missing secret with 401', async () => {
    const r = await fetch(`${url}/compile`, { method: 'POST', body: '{}' });
    expect(r.status).toBe(401);
  });
  it('compiles with the right secret', async () => {
    const r = await fetch(`${url}/compile`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', 'x-builder-secret': 's3cret' },
      body: JSON.stringify({ source: 'int main(void){for(;;){}}', language: 'c', target: 'stm32l476' }),
    });
    expect(r.status).toBe(200);
    expect((await r.json() as any).ok).toBe(true);
  });
  it('healthz is open', async () => {
    expect((await fetch(`${url}/healthz`)).status).toBe(200);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd services/labwired-builder && npx vitest run test/server.test.ts`
Expected: FAIL — `makeServer` not found.

- [ ] **Step 3: Implement `server.ts`** (secret gate; `/healthz`; route to `compile`/`run`; a global concurrency semaphore)

```typescript
import { createServer } from 'node:http';
import type { AddressInfo } from 'node:net';
import { compile } from './compile.js';
import { run } from './run.js';

let active = 0;
const MAX_CONCURRENT = Number(process.env.MAX_CONCURRENT ?? 2);

export async function makeServer(opts: { secret: string; port?: number }) {
  const server = createServer(async (req, res) => {
    const send = (code: number, body: unknown) => {
      res.writeHead(code, { 'content-type': 'application/json' });
      res.end(JSON.stringify(body));
    };
    if (req.url === '/healthz') return send(200, { ok: true });
    if (req.method !== 'POST') return send(404, { error: 'not found' });
    if (req.headers['x-builder-secret'] !== opts.secret) return send(401, { error: 'unauthorized' });
    if (active >= MAX_CONCURRENT) return send(429, { error: 'busy' });
    active++;
    try {
      const chunks: Buffer[] = [];
      for await (const c of req) chunks.push(c as Buffer);
      const body = JSON.parse(Buffer.concat(chunks).toString() || '{}');
      if (req.url === '/compile') return send(200, await compile(body));
      if (req.url === '/run') return send(200, await run(body));
      return send(404, { error: 'not found' });
    } catch (e) {
      return send(400, { error: String(e) });
    } finally {
      active--;
    }
  });
  await new Promise<void>((r) => server.listen(opts.port ?? 0, r));
  const port = (server.address() as AddressInfo).port;
  return { url: `http://127.0.0.1:${port}`, close: () => server.close() };
}

if (process.argv[1] && process.argv[1].endsWith('server.ts')) {
  const secret = process.env.BUILDER_SECRET;
  if (!secret) { console.error('BUILDER_SECRET required'); process.exit(1); }
  makeServer({ secret, port: Number(process.env.PORT ?? 8080) }).then((s) => console.log(`builder on ${s.url}`));
}
```

- [ ] **Step 4: Write the sandbox-containment test**

```typescript
import { describe, it, expect } from 'vitest';
import { compile } from '../src/compile';
describe('sandbox', () => {
  it('a compile cannot read outside the workdir via #include', async () => {
    const r = await compile({ source: '#include "/etc/shadow"\nint main(void){return 0;}', language: 'c', target: 'stm32l476' });
    expect(r.ok).toBe(false); // include path is bounded; gcc errors
  });
  it('an infinite-preprocessor source is killed by the timeout', async () => {
    const r = await compile({ source: '#define A A\nint main(){return 0;}', language: 'c', target: 'stm32l476' });
    expect(r).toBeTruthy(); // returns (ok or error) within the 15s wall, never hangs
  });
});
```

> **Note:** OS-level isolation (no-network, unprivileged user, cgroup mem cap) is enforced by running the service under the systemd unit in Task 14 (`PrivateNetwork=yes`, `DynamicUser=yes`, `MemoryMax=`). The `compile`/`run` `timeout:` options bound wall time in-process.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd services/labwired-builder && npx vitest run`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add services/labwired-builder/src/server.ts services/labwired-builder/test/server.test.ts services/labwired-builder/test/sandbox.test.ts
git commit -m "feat(builder): http server, secret gate, concurrency cap, sandbox test"
```

---

## Phase 3 — Worker tools

### Task 12: Builder client + Env

**Files:**
- Modify: `packages/api/src/types.ts` (add to `Env`)
- Create: `packages/api/src/mcp/builder-client.ts`
- Test: `packages/api/tests/builder-client.test.ts`

- [ ] **Step 1: Add Env fields** — in `packages/api/src/types.ts`, inside `interface Env`, add:

```typescript
  /** Base URL of the labwired-builder service (via Cloudflare Tunnel). */
  BUILDER_URL: string;
  /** Shared secret sent as X-Builder-Secret to the builder. */
  BUILDER_SECRET: string;
```

- [ ] **Step 2: Write the failing test**

```typescript
import { describe, it, expect, vi } from 'vitest';
import { builderCompile } from '../src/mcp/builder-client.js';

describe('builderCompile', () => {
  it('posts to /compile with the secret header', async () => {
    const fetchMock = vi.fn(async () => new Response(JSON.stringify({ ok: true, elfBase64: 'AA==', errors: [] }), { status: 200 }));
    vi.stubGlobal('fetch', fetchMock);
    const env = { BUILDER_URL: 'https://builder.test', BUILDER_SECRET: 'k' } as any;
    const r = await builderCompile(env, { source: 'x', language: 'c', target: 'stm32l476' });
    expect(r.ok).toBe(true);
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe('https://builder.test/compile');
    expect((init as any).headers['x-builder-secret']).toBe('k');
  });
});
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cd packages/api && npx vitest run tests/builder-client.test.ts`
Expected: FAIL — module not found.

- [ ] **Step 4: Implement `builder-client.ts`**

```typescript
import type { Env } from '../types.js';

export interface Diagnostic { file?: string; line?: number; col?: number; severity: 'error' | 'warning'; message: string; }
export interface BuilderCompileResult { ok: boolean; elfBase64?: string; sizeBytes?: number; errors: Diagnostic[]; }
export interface BuilderRunResult {
  status: string; stopReason: string; stepsExecuted: number; cycles: number; instructions: number;
  serial: string; peripherals: { id: string; type: string; state: unknown }[]; timedOut: boolean;
}

async function post<T>(env: Env, path: string, body: unknown): Promise<T> {
  const resp = await fetch(`${env.BUILDER_URL}${path}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', 'x-builder-secret': env.BUILDER_SECRET },
    body: JSON.stringify(body),
  });
  if (!resp.ok) throw new Error(`builder ${path} → ${resp.status}`);
  return resp.json() as Promise<T>;
}

export function builderCompile(env: Env, req: { source: string; language: 'c' | 'cpp'; target: string }) {
  return post<BuilderCompileResult>(env, '/compile', req);
}
export function builderRun(env: Env, req: { elfBase64: string; systemYaml: string; maxSteps: number }) {
  return post<BuilderRunResult>(env, '/run', req);
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cd packages/api && npx vitest run tests/builder-client.test.ts`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add packages/api/src/types.ts packages/api/src/mcp/builder-client.ts packages/api/tests/builder-client.test.ts
git commit -m "feat(api): builder client + Env wiring"
```

### Task 13: Wire the new tools

**Files:**
- Modify: `packages/api/src/mcp/tools.ts`
- Modify: `packages/api/package.json` (add `@labwired/board-config`)
- Test: `packages/api/tests/mcp-tools.test.ts`

- [ ] **Step 1: Add the dependency** — in `packages/api/package.json` add `"@labwired/board-config": "*"`. Run `cd packages/api && npm install`.

- [ ] **Step 2: Write the failing test**

```typescript
import { describe, it, expect, vi } from 'vitest';
import { listHostedTools, callHostedTool } from '../src/mcp/tools.js';

describe('expanded MCP tools', () => {
  it('advertises compile, run, list_components', () => {
    const names = listHostedTools().map((t) => t.name);
    expect(names).toContain('labwired_compile');
    expect(names).toContain('labwired_run');
    expect(names).toContain('labwired_list_components');
  });

  it('labwired_compile proxies to the builder', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify({ ok: true, elfBase64: 'AA==', sizeBytes: 1, errors: [] }), { status: 200 })));
    const env = { BUILDER_URL: 'https://b', BUILDER_SECRET: 'k', ENVIRONMENT: 'test' } as any;
    const res = await callHostedTool({ name: 'labwired_compile', arguments: { source: 'int main(){for(;;){}}', language: 'c', target: 'stm32l476' } }, env, { userId: 'u' });
    const payload = JSON.parse(res.content[0].text);
    expect(payload.ok).toBe(true);
    expect(payload.elf_base64).toBe('AA==');
  });

  it('labwired_run rejects a target/board mismatch', async () => {
    const env = { BUILDER_URL: 'https://b', BUILDER_SECRET: 'k', ENVIRONMENT: 'test' } as any;
    const res = await callHostedTool({ name: 'labwired_run', arguments: { elf_base64: 'AA==', target: 'stm32l476', diagram: { board: 'rp2040', parts: [], wires: [] }, max_steps: 1000 } }, env, { userId: 'u' });
    expect(JSON.parse(res.content[0].text).error).toMatch(/mismatch/i);
  });
});
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cd packages/api && npx vitest run tests/mcp-tools.test.ts`
Expected: FAIL — new tools absent.

- [ ] **Step 4: Implement in `tools.ts`**

Add to the `hostedTools` array:
```typescript
  {
    name: 'labwired_compile',
    description: 'Compile C/C++ firmware source for a supported target to an ELF. Returns elf_base64 on success or structured compile errors.',
    inputSchema: { type: 'object', required: ['source', 'language', 'target'], properties: {
      source: { type: 'string', description: 'Firmware source. Provide application code with a main(); the target scaffold supplies startup/linker.' },
      language: { type: 'string', enum: ['c', 'cpp'] },
      target: { type: 'string', enum: ['stm32l476'] },
    } },
  },
  {
    name: 'labwired_run',
    description: 'Run a compiled ELF on a device (diagram) in the deterministic LabWired engine. Returns status, serial output, cycles, and peripheral state.',
    inputSchema: { type: 'object', required: ['elf_base64', 'target', 'diagram', 'max_steps'], properties: {
      elf_base64: { type: 'string' }, target: { type: 'string', enum: ['stm32l476'] },
      diagram: { type: 'object', description: 'Device: { board, parts[], wires[] }.' },
      max_steps: { type: 'number', description: 'Hard cap; if hit, timed_out=true.' },
    } },
  },
  {
    name: 'labwired_list_components',
    description: 'List peripherals the simulator actually models, so you only wire devices that can run.',
    inputSchema: { type: 'object', properties: {} },
  },
```

Add the imports and handlers:
```typescript
import { diagramToConfig, COMPONENT_META } from '@labwired/board-config';
import { builderCompile, builderRun } from './builder-client.js';
import type { Env } from '../types.js';

// inside callHostedTool's dispatch:
  if (name === 'labwired_compile') {
    const a = parsed?.arguments as { source?: string; language?: 'c' | 'cpp'; target?: string };
    if (!a?.source || !a.language || !a.target)
      return { content: [textContent({ error: 'MISSING_ARGS' })], isError: true };
    const r = await builderCompile(env, { source: a.source, language: a.language, target: a.target });
    return {
      content: [textContent({ ok: r.ok, elf_base64: r.elfBase64, size_bytes: r.sizeBytes, errors: r.errors })],
      isError: r.ok ? undefined : true,
    };
  }

  if (name === 'labwired_run') {
    const a = parsed?.arguments as { elf_base64?: string; target?: string; diagram?: { board: string }; max_steps?: number };
    if (!a?.elf_base64 || !a.diagram || !a.target)
      return { content: [textContent({ error: 'MISSING_ARGS' })], isError: true };
    if (a.diagram.board !== a.target)
      return { content: [textContent({ error: `TARGET_BOARD_MISMATCH: compiled for ${a.target}, diagram board is ${a.diagram.board}` })], isError: true };
    let systemYaml: string;
    try { systemYaml = diagramToConfig(a.diagram as any).systemYaml; }
    catch (e) { return { content: [textContent({ error: 'DIAGRAM_INVALID', detail: String(e) })], isError: true }; }
    const r = await builderRun(env, { elfBase64: a.elf_base64, systemYaml, maxSteps: a.max_steps ?? 1_000_000 });
    return { content: [textContent(r)], isError: r.status === 'error' || undefined };
  }

  if (name === 'labwired_list_components') {
    const components = Object.entries(COMPONENT_META)
      .filter(([, m]) => m.boardIoKind !== undefined)
      .map(([type, m]) => ({ type, board_io_kind: m.boardIoKind }));
    return { content: [textContent({ components })] };
  }
```

Expand `labwired_list_boards` to return `{ board: 'stm32l476', target: 'stm32l476', languages: ['c', 'cpp'] }`.

- [ ] **Step 5: Run test to verify it passes**

Run: `cd packages/api && npx vitest run tests/mcp-tools.test.ts`
Expected: PASS.

- [ ] **Step 6: Full suite + typecheck**

Run: `cd packages/api && npx vitest run && npx tsc --noEmit`
Expected: all green (pre-existing stripe apiVersion warnings excepted).

- [ ] **Step 7: Commit**

```bash
git add packages/api/src/mcp/tools.ts packages/api/package.json packages/api/tests/mcp-tools.test.ts
git commit -m "feat(api): labwired_compile/run/list_components MCP tools + consistency guard"
```

### Task 14: Quota + rate limit

**Files:**
- Modify: `packages/api/src/keys.ts` (add `checkCompileRate`)
- Modify: `packages/api/src/mcp/tools.ts` (call it in compile; meter successful runs)
- Test: `packages/api/tests/quota.test.ts`

- [ ] **Step 1: Write the failing test**

```typescript
import { describe, it, expect } from 'vitest';
import { checkCompileRate } from '../src/keys.js';

function kv() { const m = new Map<string, string>(); return { get: async (k: string) => m.get(k) ?? null, put: async (k: string, v: string) => void m.set(k, v) } as any; }

describe('checkCompileRate', () => {
  it('allows under the per-minute cap and blocks over it', async () => {
    const env = { KV_WORKSPACES: kv() } as any;
    let allowed = 0;
    for (let i = 0; i < 35; i++) if ((await checkCompileRate(env, 'ws_1', 30)).allowed) allowed++;
    expect(allowed).toBe(30);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd packages/api && npx vitest run tests/quota.test.ts`
Expected: FAIL — `checkCompileRate` not found.

- [ ] **Step 3: Implement `checkCompileRate`** in `keys.ts`

```typescript
/** Fixed-window per-minute compile rate limiter, keyed in KV_WORKSPACES. */
export async function checkCompileRate(env: Env, workspaceId: string, perMinute: number): Promise<{ allowed: boolean; used: number }> {
  const windowKey = `crate:${workspaceId}`;
  const raw = await env.KV_WORKSPACES.get(windowKey);
  const now = Math.floor(Date.now() / 60000);
  let rec = raw ? JSON.parse(raw) as { window: number; count: number } : { window: now, count: 0 };
  if (rec.window !== now) rec = { window: now, count: 0 };
  if (rec.count >= perMinute) return { allowed: false, used: rec.count };
  rec.count++;
  await env.KV_WORKSPACES.put(windowKey, JSON.stringify(rec), { expirationTtl: 120 });
  return { allowed: true, used: rec.count };
}
```

- [ ] **Step 4: Enforce in `tools.ts`**

In `labwired_compile`, before calling the builder, resolve the workspace from `identity.workspaceId` and:
```typescript
    if (identity.workspaceId) {
      const rate = await checkCompileRate(env, identity.workspaceId, 30);
      if (!rate.allowed) return { content: [textContent({ error: 'RATE_LIMITED', scope: 'compile', per_minute: 30 })], isError: true };
    }
```
In `labwired_run`, after a successful run, meter cycles against the workspace (reuse the existing `maybeResetMtdCycles` + `writeWorkspaceRecord` path used by `/v1/runs`, incrementing `cycles_used_mtd` by `r.cycles`). Failed compiles and failed runs are not metered.

- [ ] **Step 5: Run test + suite**

Run: `cd packages/api && npx vitest run`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add packages/api/src/keys.ts packages/api/src/mcp/tools.ts packages/api/tests/quota.test.ts
git commit -m "feat(api): compile rate-limit + run cycle metering (failed builds free)"
```

---

## Phase 4 — e2e + ops

### Task 15: Gated end-to-end test

**Files:**
- Create: `services/labwired-builder/test/e2e.test.ts`

- [ ] **Step 1: Write the e2e** (skipped unless `RUN_E2E=1`, requires toolchain + `labwired`)

```typescript
import { describe, it, expect } from 'vitest';
import { compile } from '../src/compile';
import { run } from '../src/run';
import { readFile } from 'node:fs/promises';

const maybe = process.env.RUN_E2E ? describe : describe.skip;
maybe('e2e: compile → run blink on stm32l476', () => {
  it('compiles C and runs to a clean stop with serial captured', async () => {
    const source = await readFile(`${__dirname}/fixtures/blink.c`, 'utf8');
    const c = await compile({ source, language: 'c', target: 'stm32l476' });
    expect(c.ok).toBe(true);
    const systemYaml = await readFile(`${__dirname}/fixtures/blink-l476.system.yaml`, 'utf8');
    const r = await run({ elfBase64: c.elfBase64!, systemYaml, maxSteps: 500000 });
    expect(['finished', 'step_limit']).toContain(r.status);
    expect(r.cycles).toBeGreaterThan(0);
  });
});
```

- [ ] **Step 2: Add `fixtures/blink.c`** — a C blink that toggles PA5 via the GPIOA registers (use the L476 GPIOA base from `core/configs/chips/stm32l476.yaml`).

- [ ] **Step 3: Run gated e2e**

Run: `cd services/labwired-builder && RUN_E2E=1 npx vitest run test/e2e.test.ts`
Expected: PASS (with toolchain + `labwired` installed).

- [ ] **Step 4: Commit**

```bash
git add services/labwired-builder/test/e2e.test.ts services/labwired-builder/test/fixtures/blink.c
git commit -m "test(builder): gated compile→run e2e for stm32l476"
```

### Task 16: Ops runbook (Hetzner box)

**Files:**
- Create: `services/labwired-builder/deploy/RUNBOOK.md`
- Create: `services/labwired-builder/deploy/labwired-builder.service`
- Create: `services/labwired-builder/deploy/cloudflared-config.yml`

- [ ] **Step 1: Write `labwired-builder.service`** (systemd, hardened)

```ini
[Unit]
Description=LabWired builder
After=network.target

[Service]
DynamicUser=yes
WorkingDirectory=/opt/labwired-builder
Environment=PORT=8080 MAX_CONCURRENT=2
EnvironmentFile=/etc/labwired-builder.env
ExecStart=/usr/bin/node --import tsx /opt/labwired-builder/src/server.ts
PrivateTmp=yes
PrivateNetwork=yes
NoNewPrivileges=yes
ProtectSystem=strict
MemoryMax=2G
CPUQuota=200%
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

> `PrivateNetwork=yes` gives the no-network compile sandbox. `/etc/labwired-builder.env` holds `BUILDER_SECRET=`.

- [ ] **Step 2: Write `cloudflared-config.yml`**

```yaml
tunnel: labwired-builder
credentials-file: /etc/cloudflared/labwired-builder.json
ingress:
  - hostname: builder.labwired.com
    service: http://127.0.0.1:8080
  - service: http_status:404
```

- [ ] **Step 3: Write `RUNBOOK.md`** — exact steps:

```markdown
# labwired-builder deploy (Hetzner)

1. Toolchain: `apt-get install -y gcc-arm-none-eabi binutils-arm-none-eabi`
2. Engine: build the native CLI (`cargo build --release -p labwired-cli` in core/) and copy to `/usr/local/bin/labwired`; set `LABWIRED_BIN=/usr/local/bin/labwired`.
3. Node 20 + this service to `/opt/labwired-builder`; `npm ci --omit=dev`.
4. Secret: `openssl rand -hex 32 > /etc/labwired-builder.env` as `BUILDER_SECRET=<hex>`.
5. systemd: install the unit, `systemctl enable --now labwired-builder`; verify `curl localhost:8080/healthz`.
6. Tunnel: `cloudflared tunnel create labwired-builder`; install config; route DNS `cloudflared tunnel route dns labwired-builder builder.labwired.com`; `systemctl enable --now cloudflared`.
7. Worker secrets (uses the OAuth login, not the stale env token):
   `env -u CLOUDFLARE_API_TOKEN -u CLOUDFLARE_ACCOUNT_ID npx wrangler secret put BUILDER_SECRET --name labwired-api` (same hex)
   and add `BUILDER_URL = "https://builder.labwired.com"` to `packages/api/wrangler.toml [vars]`.
8. Smoke test: `curl -X POST https://builder.labwired.com/healthz` → 200.
```

- [ ] **Step 4: Commit**

```bash
git add services/labwired-builder/deploy
git commit -m "docs(builder): Hetzner deploy runbook + systemd + tunnel config"
```

### Task 17: Worker config + deploy

**Files:**
- Modify: `packages/api/wrangler.toml`

- [ ] **Step 1: Add `BUILDER_URL`** under `[vars]`:

```toml
BUILDER_URL = "https://builder.labwired.com"
```
(`BUILDER_SECRET` is a `wrangler secret`, not a var.)

- [ ] **Step 2: Dry-run bundle**

Run: `cd packages/api && env -u CLOUDFLARE_API_TOKEN -u CLOUDFLARE_ACCOUNT_ID npx wrangler deploy --dry-run --outdir /tmp/api-dry`
Expected: bundles; bindings show `BUILDER_URL`.

- [ ] **Step 3: Commit**

```bash
git add packages/api/wrangler.toml
git commit -m "chore(api): BUILDER_URL var for the build-run loop"
```

- [ ] **Step 4: Deploy decision** — Deploying the Worker replaces the live `/mcp` deployment, so reconcile first (the live tools.ts may differ from repo). Do NOT blind-deploy: confirm repo `tools.ts` == intended live surface, then merge through `main` and let `.github/workflows/api-worker-deploy.yml` run `npm test` + `npx wrangler deploy`. Use manual `wrangler deploy` only as an emergency fallback. Verify `labwired_list_boards`/`labwired_compile` appear via an authenticated `tools/list`.

---

## Self-review notes

- **Spec coverage:** shared converter (T1–6), builder compile/run/sandbox (T7–11), Worker tools + guard (T12–13), quota/rate-limit (T14), tests incl. e2e + sandbox (T9–11,15), ops runbook (T16–17). `list_components` (T13) covers the "unmodeled component" gap; `timed_out` (T10,13) covers silent truncation; consistency guard (T13) covers target/board mismatch.
- **Known impl confirmations (not placeholders, but verify against source at build time):** exact `Commands::Test` flag names + `script.yaml` schema (`core/crates/cli/src/main.rs`); the full `COMPONENT_META` enumeration (`packages/ui/src/editor/components/index.ts`); the L476 startup/linker adaptation (`packages/playground/server/arduino-core/`).
- **Deferred (phase 2, not in this plan):** Rust, RISC-V/ESP32/Xtensa, one-shot wrapper, live browser-watch.
