# nRF52840 BLE Lab — prebuilt two-board example: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a "nRF52840 BLE Lab" board-picker entry that opens a canvas with two nRF52840 boards (sensor + collector) both visible, that genuinely talk over the shared virtual-air, with a resizable BLE packet analyzer reachable from a Tools toggle (not an always-on frozen panel).

**Architecture:** Reuse the existing part-based multi-chip engine (`usePerChipSims`, keyed by diagram part id — already wired into App.tsx). The only engine-side change is teaching part→board resolution to honor a per-part `attrs.boardId`, so two parts sharing `mcuComponentType: 'nrf52840-dk'` can run different firmware (sensor TX vs collector RX). The two-MCU canvas is produced by special-casing `makeStarterDiagram` for the new lab config. The analyzer's decode logic is untouched; only its mount/visibility/sizing change.

**Tech Stack:** React + Vite + TypeScript (`packages/playground`), Vitest, Tailwind utility classes, Rust core via WASM bridge (`@labwired/ui`), Playwright MCP for live verification.

---

## Background facts (verified this session — line numbers from main @32c8ae4)

- **Mechanism A = `usePerChipSims`** (`packages/playground/src/multi-mcu/usePerChipSims.ts`): keyed by diagram part id; the selected MCU part is foreground (driven by the main sim loop), every other running MCU part is background-ticked (200k cycles / 16ms). All parts share the Rust-side process-global virtual-air (`OnceLock<Mutex<VirtualAir>>`), so frames cross between instances. Already wired into `App.tsx`: `foregroundPartId` (1135 = `drawerSubject.part?.id ?? 'mcu'`), `mcuPartIds` (1136-1139), the hook (1140-1152), per-part window render (2347-2450), the analyzer (2456-2468).
- **`mcuBoardForPart(part, primaryBoard)`** in `App.tsx` (526-533):
  ```ts
  function mcuBoardForPart(
    part: { id: string; type: string } | undefined,
    primaryBoard: BoardConfig,
  ): BoardConfig | null {
    if (!part) return null;
    if (part.id === 'mcu') return primaryBoard;
    return BOARD_CONFIGS.find((b) => b.mcuComponentType === part.type) ?? null;
  }
  ```
  Callers pass `selectedBoard` as `primaryBoard` (lines 1128, 1137, 2351, 2459). The bug: all four nRF entries share `mcuComponentType: 'nrf52840-dk'`, so the type-match collapses every nRF part to the first nRF config (the DK — which has **no RADIO**).
- **`loadBoardWorkspace(config)`** (App.tsx 437-468) returns `{ diagram, source }`. The diagram comes from **`makeStarterDiagram(config)`** (App.tsx 441), overridden only by a non-empty saved localStorage diagram. **There is NO `diagram` field on `BoardConfig`** — the two-MCU canvas must be emitted by `makeStarterDiagram`. The `Diagram` type has `parts: Part[]` (and connections); each `Part` has `id, type, x, y, rotate, attrs` (and optional `scale`). `Diagram`/`Part` are imported into App.tsx (confirm the exact module in Step 1.1).
- **BLE board entries already exist** in `bundled-configs.ts` (402-424): `nrf52840-ble-sensor` and `nrf52840-ble-collector`. Both use `chipYaml: chipNrf52840Onboarding` + `systemYaml: systemNrf52840Onboarding` (RADIO + CLOCK present; the DK system has neither), share `mcuComponentType: 'nrf52840-dk'`, `kind: 'lab'`, and carry `demoFirmwarePath` (`${BASE}wasm/demo-nrf52840-ble-sensor.elf` / `…-collector.elf`).
- **`BleAnalyzer`** (`packages/playground/src/instruments/BleAnalyzer.tsx`, 102 lines) is healthy: `<div className="flex flex-col h-full min-h-0 …">` with an internal `overflow-auto` table. Props: `{ bridge: SimulatorBridge | null; running: boolean; pollMs?: number }`. Decode logic is correct — **do not change it.** The freeze is its mount (App.tsx 2456-2468): gated on `hasBle && !isMobile`, fixed `w-[460px] h-[300px]`, no close, no resize.
- **`SimDock`** (`packages/playground/src/studio/SimDock.tsx`) holds the toolbar toggle buttons (the run/control row). It is the home for the new "Analyzer" toggle. Confirm its props + button pattern in Step 3.1.
- **Decisions locked:** both boards visible; revert commit `32c8ae4` on the feature branch.

---

## File Structure

- **Create** `packages/playground/src/board-resolve.ts` — pure, testable `resolveBoardForPart(part, primaryBoard, boards)`. One job: map a diagram part to its `BoardConfig`, honoring `attrs.boardId` first. Extracted from App.tsx so it is unit-testable without the React tree.
- **Create** `packages/playground/src/board-resolve.test.ts` — Vitest; the collision case is the headline test.
- **Modify** `packages/playground/src/App.tsx` — (a) `mcuBoardForPart` delegates to `resolveBoardForPart`; (b) `makeStarterDiagram` special-cases the lab to emit two MCU parts; (c) replace the always-on analyzer mount with a Tools-toggle + resizable host; (d) thread `showAnalyzer` to the SimDock toggle.
- **Modify** `packages/playground/src/bundled-configs.ts` — add the `nrf52840-ble-lab` config entry.
- **Modify** `packages/playground/src/studio/SimDock.tsx` — add the "Analyzer" toggle button + props.
- **Modify (git)** revert commit `32c8ae4` on the feature branch.

---

## Task 0: Branch setup + revert the dead junk-tile fix

**Files:** none (git only)

- [ ] **Step 1: Clean base + feature branch**

```bash
cd /home/andrii/Projects/labwired
git fetch origin
git switch main && git pull --ff-only
git switch -c feat/nrf-ble-lab-example
git log --oneline -3
```
Expected: tip is `32c8ae4 fix(playground): board pick replaces untouched default chip (no junk tile)` (or newer if main advanced — fine).

- [ ] **Step 2: Revert the broken junk-tile commit**

```bash
git revert --no-edit 32c8ae4
git log --oneline -2
```
Expected: a new `Revert "fix(playground): board pick replaces untouched default chip…"` commit. If it conflicts (main advanced and touched the same `ChipsProvider.addChip` lines), resolve by keeping the pre-`32c8ae4` append-only / refocus-existing `addChip` (no scratch-replace), then `git revert --continue`.

- [ ] **Step 3: Sanity build**

```bash
cd packages/playground && npx tsc --noEmit
```
Expected: exit 0.

---

## Task 1: `resolveBoardForPart` — honor `attrs.boardId` (fixes the nRF collision)

**Files:**
- Create: `packages/playground/src/board-resolve.ts`
- Test: `packages/playground/src/board-resolve.test.ts`
- Modify: `packages/playground/src/App.tsx` (`mcuBoardForPart`, 526-533)

- [ ] **Step 1: Confirm the `Part` import path**

```bash
cd /home/andrii/Projects/labwired/packages/playground/src
grep -rn "interface Part\|type Part\b\|export interface Diagram" . --include=*.ts --include=*.tsx 2>/dev/null | grep -v node_modules | head
grep -n "import.*\bPart\b\|import.*\bDiagram\b" App.tsx | head
```
Confirm `Part` has `id: string`, `type: string`, `attrs` (a record). Note the module it is exported from (needed only if you choose to import the `Part` type into the new file — the resolver uses a structural param type, so an import is optional).

- [ ] **Step 2: Write the failing test**

Create `packages/playground/src/board-resolve.test.ts`:
```ts
import { describe, it, expect } from 'vitest';
import { resolveBoardForPart } from './board-resolve';
import type { BoardConfig } from './bundled-configs';

const dk = { boardId: 'nrf52840-dk', mcuComponentType: 'nrf52840-dk' } as BoardConfig;
const sensor = { boardId: 'nrf52840-ble-sensor', mcuComponentType: 'nrf52840-dk' } as BoardConfig;
const collector = { boardId: 'nrf52840-ble-collector', mcuComponentType: 'nrf52840-dk' } as BoardConfig;
const boards = [dk, sensor, collector];

const part = (id: string, type: string, attrs: Record<string, unknown> = {}) =>
  ({ id, type, attrs } as { id: string; type: string; attrs: Record<string, unknown> });

describe('resolveBoardForPart', () => {
  it('prefers attrs.boardId over the mcuComponentType first-match (the nRF collision)', () => {
    expect(resolveBoardForPart(part('mcu', 'nrf52840-dk', { boardId: 'nrf52840-ble-sensor' }), dk, boards)).toBe(sensor);
    expect(resolveBoardForPart(part('mcu-collector', 'nrf52840-dk', { boardId: 'nrf52840-ble-collector' }), dk, boards)).toBe(collector);
  });
  it('falls back to primaryBoard for the legacy id==="mcu" part with no boardId', () => {
    expect(resolveBoardForPart(part('mcu', 'nrf52840-dk'), dk, boards)).toBe(dk);
  });
  it('falls back to mcuComponentType match for a non-mcu part with no boardId', () => {
    expect(resolveBoardForPart(part('mcu-collector', 'nrf52840-dk'), dk, boards)).toBe(dk);
  });
  it('returns null when nothing matches', () => {
    expect(resolveBoardForPart(part('mcu-collector', 'no-such-type'), dk, boards)).toBeNull();
  });
  it('ignores a non-existent attrs.boardId, falling through', () => {
    expect(resolveBoardForPart(part('mcu-collector', 'nrf52840-dk', { boardId: 'ghost' }), dk, boards)).toBe(dk);
  });
});
```

- [ ] **Step 3: Run the test — verify it fails**

```bash
cd /home/andrii/Projects/labwired/packages/playground && npx vitest run src/board-resolve.test.ts
```
Expected: FAIL — `Failed to resolve import "./board-resolve"`.

- [ ] **Step 4: Implement**

Create `packages/playground/src/board-resolve.ts`:
```ts
import { type BoardConfig } from './bundled-configs';

/**
 * Resolve which BoardConfig a diagram part represents.
 *
 * Order:
 *   1. attrs.boardId — explicit, set on MCU parts in multi-board labs so two
 *      parts sharing an mcuComponentType (e.g. both 'nrf52840-dk') run their
 *      own firmware. This makes the BLE sensor + collector distinguishable on
 *      one canvas.
 *   2. id === 'mcu' -> the workspace's primary board.
 *   3. first board whose mcuComponentType matches the part's type.
 *   4. null.
 */
export function resolveBoardForPart(
  part: { id: string; type: string; attrs?: Record<string, unknown> | null },
  primaryBoard: BoardConfig,
  boards: readonly BoardConfig[],
): BoardConfig | null {
  const boardId = part.attrs?.boardId;
  if (typeof boardId === 'string') {
    const byId = boards.find((b) => b.boardId === boardId);
    if (byId) return byId;
  }
  if (part.id === 'mcu') return primaryBoard;
  return boards.find((b) => b.mcuComponentType === part.type) ?? null;
}
```

- [ ] **Step 5: Run the test — verify it passes**

```bash
cd /home/andrii/Projects/labwired/packages/playground && npx vitest run src/board-resolve.test.ts
```
Expected: PASS (5 tests).

- [ ] **Step 6: Delegate from App.tsx**

Add the import beside the `bundled-configs` import:
```ts
import { resolveBoardForPart } from './board-resolve';
```
Replace the body of `mcuBoardForPart` (526-533), keeping the `!part` guard and adding `attrs` to the param type so `attrs.boardId` type-checks:
```ts
function mcuBoardForPart(
  part: { id: string; type: string; attrs?: Record<string, unknown> | null } | undefined,
  primaryBoard: BoardConfig,
): BoardConfig | null {
  if (!part) return null;
  return resolveBoardForPart(part, primaryBoard, BOARD_CONFIGS);
}
```

- [ ] **Step 7: Typecheck + commit**

```bash
cd /home/andrii/Projects/labwired/packages/playground && npx tsc --noEmit && npx vitest run src/board-resolve.test.ts
git add packages/playground/src/board-resolve.ts packages/playground/src/board-resolve.test.ts packages/playground/src/App.tsx
git commit -m "feat(playground): resolve MCU part board via attrs.boardId (fix nRF collision)"
```
Expected: tsc exit 0; 5 tests pass.

---

## Task 2: `nrf52840-ble-lab` config + two-MCU starter diagram

**Files:**
- Modify: `packages/playground/src/bundled-configs.ts` (new config entry)
- Modify: `packages/playground/src/App.tsx` (`makeStarterDiagram`)

- [ ] **Step 1: Read `makeStarterDiagram` to learn the exact part shape**

```bash
cd /home/andrii/Projects/labwired/packages/playground/src
sed -n '/function makeStarterDiagram/,/^}/p' App.tsx | head -120
```
Note: how the default `{ id: 'mcu', type: <component>, x, y, rotate, attrs }` part is constructed, the `Diagram` return shape (`{ parts, connections? }`), default coordinates, and how `config.mcuComponentType` maps to a part `type` (App.tsx:143 sets `type: config.mcuComponentType`). The nRF lab's MCU parts use `type: 'nrf52840-dk'` (the component type), disambiguated by `attrs.boardId`.

- [ ] **Step 2: Add the lab config to `bundled-configs.ts`**

Insert into `BOARD_CONFIGS` right after the `nrf52840-ble-collector` entry (before the closing `];` at line 425). The lab's own `chipYaml`/`systemYaml` are the onboarding (RADIO-equipped) manifests; per-part firmware comes from each part's resolved board, so the lab itself needs no `demoFirmwarePath`:
```ts
  {
    boardId: 'nrf52840-ble-lab',
    chipId: 'nrf52840',
    name: 'nRF52840 BLE Lab (2 boards)',
    description: 'Two nRF52840s on one canvas: a Sensor advertising an incrementing reading over the BLE 1 Mbit PHY and a Collector receiving it — both running at once over the shared virtual air. Run and open the Packet Analyzer (Tools) to watch the frames.',
    arch: 'ARM Cortex-M4F',
    chipYaml: chipNrf52840Onboarding,
    systemYaml: systemNrf52840Onboarding,
    mcuComponentType: 'nrf52840-dk',
    kind: 'lab',
  },
```

- [ ] **Step 3: Special-case `makeStarterDiagram` for the lab**

In `makeStarterDiagram` (App.tsx, body starts at line 140), add a branch alongside the other `if (config.boardId === …)` blocks. The verified diagram shape is `{ ...createEmptyDiagram(config.chipId), parts: Part[], wires: Wire[] }` (it uses `wires`, NOT `connections`), and `Part = { id, type, x, y, rotate, attrs }` (optional `scale`). The two MCU parts have no wires between them — they talk over the virtual air, not copper. Keep one part id === `'mcu'` so `foregroundPartId`'s default (`'mcu'`) has a target on first load:
```ts
  if (config.boardId === 'nrf52840-ble-lab') {
    return {
      ...createEmptyDiagram(config.chipId),
      parts: [
        { id: 'mcu', type: config.mcuComponentType, x: 100, y: 160, rotate: 0,
          attrs: { boardId: 'nrf52840-ble-sensor' } },
        { id: 'mcu-collector', type: config.mcuComponentType, x: 560, y: 160, rotate: 0,
          attrs: { boardId: 'nrf52840-ble-collector' } },
      ],
      wires: [],
    };
  }
```
(`config.mcuComponentType` is `'nrf52840-dk'` for this lab — same as the top-of-function `mcu` const uses.)

- [ ] **Step 4: Ensure each MCU part runs its OWN firmware**

```bash
cd /home/andrii/Projects/labwired/packages/playground/src
grep -n "demoFirmwarePath\|drawerSubject.board\|launchSimulation\|loadFirmware\|fromConfig" App.tsx | head -30
```
Confirm the path that builds a part's bridge resolves firmware from the part's board (`mcuBoardForPart`/`drawerSubject.board`), not unconditionally from `selectedBoard`. The foreground part is `drawerSubject.part`, and `drawerSubject.board = mcuBoardForPart(part, selectedBoard) ?? selectedBoard` (1128) — so the foreground already loads the resolved board's firmware. Verify `usePerChipSims`/background bridges do the same for the non-foreground part; if a background bridge is built from `selectedBoard` rather than its part's resolved board, fix it to use the resolved board's `demoFirmwarePath`. Note any code change in the commit.

- [ ] **Step 5: Typecheck + commit**

```bash
cd /home/andrii/Projects/labwired/packages/playground && npx tsc --noEmit
git add packages/playground/src/bundled-configs.ts packages/playground/src/App.tsx
git commit -m "feat(playground): nRF52840 BLE Lab — two-board starter diagram"
```
Expected: exit 0.

---

## Task 3: Move the analyzer into a Tools toggle + make it resizable

**Files:**
- Modify: `packages/playground/src/App.tsx` (analyzer mount 2456-2468; `showAnalyzer` state; pass to SimDock)
- Modify: `packages/playground/src/studio/SimDock.tsx` (toggle button + props)

- [ ] **Step 1: Read SimDock's props + button pattern, and the analyzer mount**

```bash
cd /home/andrii/Projects/labwired/packages/playground/src
sed -n '1,130p' studio/SimDock.tsx
grep -n "SimDock" App.tsx
sed -n '2452,2470p' App.tsx
```
Note SimDock's prop interface and an existing toggle button's exact `className`/markup to mirror, and where `<SimDock … />` is rendered in App so the new props can be passed.

- [ ] **Step 2: Add visibility state in App**

Near the other `useState` hooks, add:
```ts
// Analyzer is an opt-in instrument now (was an always-on panel that froze the
// canvas). Toggled from the SimDock Tools control; hidden by default.
const [showAnalyzer, setShowAnalyzer] = useState(false);
```

- [ ] **Step 3: Add the toggle to SimDock**

In `SimDock.tsx`, extend the props interface with:
```ts
  analyzerOpen: boolean;
  onToggleAnalyzer: () => void;
```
Add a button just before the `<div className="flex-1" />` spacer (line ~119), styled like the existing Step/Reset secondary buttons (verified classes below; uses `clsx` which is already imported):
```tsx
<button
  type="button"
  onClick={onToggleAnalyzer}
  aria-pressed={analyzerOpen}
  title="Packet Analyzer (BLE air)"
  style={{ borderRadius: 999 }}
  className={clsx(
    'h-10 sm:h-8 px-3 text-[13px] outline-none border-0 shrink-0',
    analyzerOpen
      ? 'bg-accent/20 text-fg-primary'
      : 'bg-white/[0.05] hover:bg-white/[0.10] text-fg-secondary hover:text-fg-primary',
  )}
>
  Analyzer
</button>
```
At the `<SimDock … />` call site (App.tsx:1770), pass `analyzerOpen={showAnalyzer}` and `onToggleAnalyzer={() => setShowAnalyzer((v) => !v)}`. Update `SimDock.test.tsx` to supply the two new props (it renders SimDock with a fixed prop set).

- [ ] **Step 4: Replace the always-on mount with a resizable, dismissable host**

Replace the analyzer block (App.tsx 2456-2468). Drop the `hasBle` auto-show gate — visibility is now the explicit `showAnalyzer` toggle (keep `!isMobile`). Native CSS `resize` needs `overflow` set on the same element; the analyzer fills it via its own `h-full`:
```tsx
{!isMobile && showAnalyzer && (
  <div
    className="absolute bottom-4 right-4 z-30 flex flex-col rounded-lg border border-border bg-bg-base shadow-xl overflow-hidden"
    style={{ resize: 'both', width: 520, height: 320, minWidth: 320, minHeight: 160, maxWidth: '90vw', maxHeight: '80vh' }}
  >
    <div className="flex items-center justify-between px-2 py-1 border-b border-border">
      <span className="text-[11px] font-semibold text-fg-secondary">Tools · Packet Analyzer</span>
      <button
        type="button"
        className="text-fg-tertiary hover:text-fg-primary text-[12px] px-1"
        title="Close analyzer"
        onClick={() => setShowAnalyzer(false)}
      >
        ✕
      </button>
    </div>
    <div className="flex-1 min-h-0 overflow-hidden">
      <BleAnalyzer bridge={bridge} running={running} />
    </div>
  </div>
)}
```
NOTE: the outer `overflow-hidden` satisfies the `resize` requirement; the inner `min-h-0 overflow-hidden` lets `BleAnalyzer`'s internal `overflow-auto` table scroll. Confirm the parent of this block is positioned (`relative`/the canvas wrapper) so `absolute` anchors correctly; if not, switch `absolute` to `fixed`.

- [ ] **Step 5: Typecheck + tests + commit**

```bash
cd /home/andrii/Projects/labwired/packages/playground && npx tsc --noEmit && npx vitest run src/studio/SimDock.test.tsx
git add packages/playground/src/App.tsx packages/playground/src/studio/SimDock.tsx packages/playground/src/studio/SimDock.test.tsx
git commit -m "fix(playground): analyzer is a toggled, resizable Tools instrument (no more frozen panel)"
```
Expected: exit 0; SimDock tests pass.

---

## Task 4: Live verification — the proof gate (do NOT merge before this passes)

**Files:** none (Playwright MCP)

- [ ] **Step 1: Dev server, no auth (background)**

```bash
cd /home/andrii/Projects/labwired/packages/playground
VITE_DISABLE_AUTH=true npx vite --port 5173 --strictPort
```
Run with `run_in_background: true`. Stop later with `fuser -k 5173/tcp` — **never** `pkill -f vite` (matches the agent's own shell, exits 144).

- [ ] **Step 2: Open the lab → two boards visible**

Playwright MCP: `browser_navigate` to `http://localhost:5173`, open the board picker / ⌘K palette, choose **"nRF52840 BLE Lab (2 boards)"**. `browser_snapshot` + `browser_take_screenshot`.
Expected: **two nRF52840 boards** on one canvas (sensor + collector).

- [ ] **Step 3: Analyzer from Tools is usable**

Click the **Analyzer** toggle in SimDock. Verify the panel appears, drag its corner (`browser_drag`) to resize, and it scrolls — no UI freeze. Toggle it off and on again.
Expected: toggles, resizes, does not lock the page.

- [ ] **Step 4: Run → packets cross**

Press **Run**. `browser_wait_for` ~2-3s. `browser_take_screenshot` of the analyzer.
Expected: the frame counter climbs; rows show a de-whitened **Reading** that **increments** (the sensor's counter) and populated **Address**/**Freq** — proof the collector hears the sensor over shared air (not one board echoing itself). `browser_console_messages`: no errors.

- [ ] **Step 5: Reload persistence**

`browser_navigate` to the same URL (reload); re-open the lab if needed.
Expected: two boards still load and Run still produces packets; no junk tile, no broken state.

- [ ] **Step 6: Record evidence**

Save the key screenshot(s); note frame counts + a couple of incrementing Reading values for the PR. If ANY step fails: stop, fix the relevant task, re-run — do not merge.

- [ ] **Step 7: Stop dev server**

```bash
fuser -k 5173/tcp
```

---

## Task 5: Cleanup + PR

**Files:** none (git + GitHub)

- [ ] **Step 1: Full local check**

```bash
cd /home/andrii/Projects/labwired/packages/playground && npx tsc --noEmit && npx vitest run
```
Expected: tsc exit 0; all tests pass (incl. `board-resolve.test.ts`).

- [ ] **Step 2: Abandon dead scratch branches**

```bash
cd /home/andrii/Projects/labwired
git branch -D fix/multichip-junk-tile fix/multichip-scratch-replace 2>/dev/null || true
git branch --list 'fix/multichip-*'
```

- [ ] **Step 3: Push + PR**

```bash
git push -u origin feat/nrf-ble-lab-example
gh pr create --fill --title "nRF52840 BLE Lab — prebuilt two-board example + analyzer Tools fix"
```
PR body: the proof-gate screenshots/notes from Task 4; that `32c8ae4` is reverted here; that BLE decode logic is unchanged. **No AI/Claude attribution or Co-Authored-By line** (project convention).

- [ ] **Step 4: Re-check main hasn't moved**

```bash
git fetch origin && git log --oneline origin/main -3
```
If main advanced, rebase and re-run Task 4's proof gate before merge.

---

## Follow-up work (tracked — NOT dropped)

Per the user: the general **Add-MCU-onto-canvas UX** is **not deferred indefinitely** — it ships as a **separate follow-up** after this lab lands. Capture it as its own issue/PR:

- **Add-MCU general path:** make the ⌘K "Add MCU" command append a new MCU *part* to the current diagram (Mechanism A: `editor.addPart` with a fresh id + `attrs.boardId`), instead of routing through Mechanism B (`ChipsProvider.addChip`, which created the junk-tile/clobber problems). This builds directly on Task 1's `attrs.boardId` resolution — any added MCU part already resolves to its own board/firmware.
- **Retire Mechanism B (optional, second follow-up):** once Add-MCU uses parts, the `ChipsProvider`/`McuStrip`/`ChipBridgeSync`/`useBackgroundChips` tab system is redundant for the playground; remove or quarantine it (self-contained — only App.tsx + its own files + `useCommandPaletteItems.ts` consume `useChips`). Do this only after confirming nothing else relies on it; coordinate since another agent is active.

Create the follow-up issue when this PR opens; link it in the PR body.

---

## Self-review notes

- **Spec coverage:** prebuilt lab (Task 2) ✓; both-boards-visible via Mechanism A (Tasks 1-2, verified Task 4) ✓; part→board identity fix (Task 1) ✓; analyzer Tools-toggle + resizable (Task 3) ✓; revert 32c8ae4 (Task 0) ✓; live proof gate (Task 4) ✓; vitest for resolver (Task 1) ✓; Add-MCU captured as tracked follow-up (Follow-up section) ✓.
- **Recon-first steps (1.1, 2.1, 3.1, 2.4):** read the exact existing shapes (`Part` import, `makeStarterDiagram` body + `Diagram`/`Part` literal, `SimDock` props/button, per-part firmware path) before writing code against them — concrete reads, not "TBD".
- **Type consistency:** resolver is `resolveBoardForPart` throughout; `mcuBoardForPart` keeps name + `(part, primaryBoard)` signature and delegates; `attrs.boardId` is the single disambiguation key; the lab is `nrf52840-ble-lab`; the second part id is `mcu-collector` everywhere.
- **Corrected from v1:** the starter diagram is synthesized by `makeStarterDiagram` (App.tsx:441), NOT a `BoardConfig.diagram` field — Task 2 now edits `makeStarterDiagram`. `mcuBoardForPart` takes `primaryBoard` as a parameter (not closure) — Task 1 Step 6 matches the real signature.
