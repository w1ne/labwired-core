# nRF52840 BLE Lab — prebuilt two-board example: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a "nRF52840 BLE Lab" board-picker entry that opens a canvas with two nRF52840 boards (sensor + collector) both visible, that genuinely talk over the shared virtual-air, with a fixed/resizable BLE packet analyzer reachable from a Tools menu.

**Architecture:** Reuse the existing part-based multi-chip engine (`usePerChipSims`, keyed by diagram part id — already wired into App.tsx). The only engine-side change is teaching part→board resolution to honor a per-part `attrs.boardId`, so two parts sharing `mcuComponentType: 'nrf52840-dk'` can run different firmware (sensor TX vs collector RX). The lab is a bundled config whose starter diagram pre-places both MCU parts. The analyzer's decode logic is untouched; only its mount/visibility/sizing change.

**Tech Stack:** React + Vite + TypeScript (`packages/playground`), Vitest, Tailwind utility classes, Rust core via WASM bridge (`@labwired/ui`), Playwright MCP for live verification.

---

## Background facts (verified earlier this session)

- **Mechanism A = `usePerChipSims`** (`packages/playground/src/multi-mcu/usePerChipSims.ts`): keyed by diagram part id; the selected MCU part is foreground (driven by the main sim loop), every other running MCU part is background-ticked (200k cycles / 16ms). All parts share the Rust-side process-global virtual-air (`OnceLock<Mutex<VirtualAir>>`), so frames cross between instances. Already wired into `App.tsx`: `foregroundPartId` (~1135 = `drawerSubject.part?.id ?? 'mcu'`), the hook (~1140), `mcuPartIds` (~1136), per-part render `isFg` (~2352), BLE detection (~2457), `useBackgroundChips(true)` (~2475).
- **`mcuBoardForPart`** in `App.tsx` (~498–532): currently
  ```ts
  // (inside the App component; primaryBoard is in scope)
  function mcuBoardForPart(part: Part): BoardConfig | null {
    if (part.id === 'mcu') return primaryBoard;
    return BOARD_CONFIGS.find((b) => b.mcuComponentType === part.type) ?? null;
  }
  ```
  All four nRF board entries share `mcuComponentType: 'nrf52840-dk'`, so the type-match collapses every nRF part to the first nRF config (the DK — which has **no RADIO**). This is the collision to fix.
- **BLE board entries already exist** in `packages/playground/src/bundled-configs.ts`: `nrf52840-ble-sensor` and `nrf52840-ble-collector`. Both use `chipYaml: chipNrf52840Onboarding` + `systemYaml: systemNrf52840Onboarding` (these have RADIO + CLOCK; the DK system does not), share `mcuComponentType: 'nrf52840-dk'`, and carry a `demoFirmwarePath` (sensor = TX firmware, collector = RX firmware).
- **`BleAnalyzer`** (`packages/playground/src/instruments/BleAnalyzer.tsx`, 102 lines) is a healthy component: `<div className="flex flex-col h-full min-h-0 …">` with an internal `overflow-auto` table region. Props: `{ bridge: SimulatorBridge | null; running: boolean; pollMs?: number }`. It polls `bridge.airTraceSnapshot()` → `decodeBleTrace()` and renders rows. **Decode logic is correct and must not change.** The "frozen / unresizable / useless" problem is purely its **mount** at `App.tsx` (~2452–2465): an always-on panel with no size control and no way to hide it.
- **Decisions locked:** both boards visible; revert commit `32c8ae4` on the feature branch.
- **Deferred (YAGNI):** general Add-MCU-onto-canvas UX; deleting Mechanism B (`ChipsProvider`/`McuStrip`/`ChipBridgeSync`) files.

---

## File Structure

- **Create** `packages/playground/src/board-resolve.ts` — pure, testable `resolveBoardForPart(part, primaryBoard, boards)` helper. One responsibility: map a diagram part to its `BoardConfig`, honoring `attrs.boardId` first. Extracted from App.tsx so it can be unit-tested without the React tree.
- **Create** `packages/playground/src/board-resolve.test.ts` — Vitest for the resolver (collision case is the headline test).
- **Modify** `packages/playground/src/App.tsx` — (a) call `resolveBoardForPart` from `mcuBoardForPart`; (b) replace the always-on analyzer mount with a Tools-menu toggle + resizable host.
- **Modify** `packages/playground/src/bundled-configs.ts` — add the `nrf52840-ble-lab` config with a two-MCU starter diagram, each MCU part carrying `attrs.boardId`.
- **Modify (git)** revert commit `32c8ae4` on the feature branch.

---

## Task 0: Branch setup + revert the dead junk-tile fix

**Files:** none (git only)

- [ ] **Step 1: Confirm a clean base and create the feature branch**

Run:
```bash
cd /home/andrii/Projects/labwired
git fetch origin
git switch main && git pull --ff-only
git switch -c feat/nrf-ble-lab-example
git log --oneline -3
```
Expected: tip is `32c8ae4 fix(playground): board pick replaces untouched default chip (no junk tile)` (or newer if main advanced — fine).

- [ ] **Step 2: Revert the broken junk-tile commit**

Run:
```bash
git revert --no-edit 32c8ae4
git log --oneline -2
```
Expected: a new `Revert "fix(playground): board pick replaces untouched default chip…"` commit on top. If the revert conflicts (main advanced and touched the same lines), resolve by keeping the pre-`32c8ae4` behavior of `ChipsProvider.addChip` (plain append-only / refocus-existing — no scratch-replace), then `git revert --continue`.

- [ ] **Step 3: Sanity build**

Run:
```bash
cd packages/playground && npx tsc --noEmit
```
Expected: exit 0 (no type errors introduced by the revert).

---

## Task 1: `resolveBoardForPart` — honor `attrs.boardId` (fixes the nRF collision)

**Files:**
- Create: `packages/playground/src/board-resolve.ts`
- Test: `packages/playground/src/board-resolve.test.ts`
- Modify: `packages/playground/src/App.tsx` (the `mcuBoardForPart` function, ~498–532)

- [ ] **Step 1: Confirm the `Part` and `BoardConfig` shapes**

Run:
```bash
cd /home/andrii/Projects/labwired/packages/playground/src
grep -rn "interface Part\b\|type Part =\|attrs" lib editor *.ts 2>/dev/null | grep -v node_modules | head
sed -n '55,118p' bundled-configs.ts   # BoardConfig interface
```
Confirm: `Part` has `id: string`, `type: string`, and an `attrs` record (string-keyed). `BoardConfig` has `boardId: string`, `mcuComponentType: string`. Note the exact import path for the `Part` type (used in the next step).

- [ ] **Step 2: Write the failing test**

Create `packages/playground/src/board-resolve.test.ts`:
```ts
import { describe, it, expect } from 'vitest';
import { resolveBoardForPart } from './board-resolve';
import type { BoardConfig } from './bundled-configs';

// Minimal fixtures — only the fields the resolver reads.
const dk = { boardId: 'nrf52840-dk', mcuComponentType: 'nrf52840-dk' } as BoardConfig;
const sensor = { boardId: 'nrf52840-ble-sensor', mcuComponentType: 'nrf52840-dk' } as BoardConfig;
const collector = { boardId: 'nrf52840-ble-collector', mcuComponentType: 'nrf52840-dk' } as BoardConfig;
const boards = [dk, sensor, collector];

// A part is whatever the diagram stores; the resolver only reads id/type/attrs.
const part = (id: string, type: string, attrs: Record<string, unknown> = {}) =>
  ({ id, type, attrs } as any);

describe('resolveBoardForPart', () => {
  it('prefers attrs.boardId over the mcuComponentType first-match (the nRF collision)', () => {
    // Both parts share type nrf52840-dk; without attrs.boardId they would both
    // resolve to `dk`. attrs.boardId must disambiguate.
    expect(resolveBoardForPart(part('mcu', 'nrf52840-dk', { boardId: 'nrf52840-ble-sensor' }), dk, boards))
      .toBe(sensor);
    expect(resolveBoardForPart(part('mcu2', 'nrf52840-dk', { boardId: 'nrf52840-ble-collector' }), dk, boards))
      .toBe(collector);
  });

  it('falls back to primaryBoard for the legacy id==="mcu" part with no boardId', () => {
    expect(resolveBoardForPart(part('mcu', 'nrf52840-dk'), dk, boards)).toBe(dk);
  });

  it('falls back to mcuComponentType match for a non-mcu part with no boardId', () => {
    expect(resolveBoardForPart(part('mcu2', 'nrf52840-dk'), dk, boards)).toBe(dk);
  });

  it('returns null when nothing matches', () => {
    expect(resolveBoardForPart(part('mcu2', 'no-such-type'), dk, boards)).toBeNull();
  });

  it('ignores an attrs.boardId that does not exist, falling through', () => {
    expect(resolveBoardForPart(part('mcu2', 'nrf52840-dk', { boardId: 'ghost' }), dk, boards)).toBe(dk);
  });
});
```

- [ ] **Step 3: Run the test to verify it fails**

Run:
```bash
cd /home/andrii/Projects/labwired/packages/playground && npx vitest run src/board-resolve.test.ts
```
Expected: FAIL — `Failed to resolve import "./board-resolve"` / `resolveBoardForPart is not a function`.

- [ ] **Step 4: Write the implementation**

Create `packages/playground/src/board-resolve.ts`:
```ts
import { type BoardConfig } from './bundled-configs';

/**
 * Resolve which BoardConfig a diagram part represents.
 *
 * Resolution order:
 *   1. `attrs.boardId` — explicit, set on MCU parts in multi-board labs so two
 *      parts that share an `mcuComponentType` (e.g. both 'nrf52840-dk') can run
 *      different firmware. This is what makes the BLE sensor + collector
 *      distinguishable on one canvas.
 *   2. The legacy `id === 'mcu'` part maps to the workspace's primary board.
 *   3. First board whose `mcuComponentType` matches the part's `type`.
 *   4. null — unknown part.
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

- [ ] **Step 5: Run the test to verify it passes**

Run:
```bash
cd /home/andrii/Projects/labwired/packages/playground && npx vitest run src/board-resolve.test.ts
```
Expected: PASS (5 tests).

- [ ] **Step 6: Wire App.tsx to the helper**

In `packages/playground/src/App.tsx`, add the import near the other local imports (e.g. beside the `bundled-configs` import):
```ts
import { resolveBoardForPart } from './board-resolve';
```
Then replace the body of `mcuBoardForPart` (~498–532) so it delegates — keep its existing signature/name and surrounding comments:
```ts
function mcuBoardForPart(part: Part): BoardConfig | null {
  return resolveBoardForPart(part, primaryBoard, BOARD_CONFIGS);
}
```
(If `mcuBoardForPart` is a `useCallback`, keep it a `useCallback` with `[primaryBoard]` deps and the same delegated body.)

- [ ] **Step 7: Typecheck + commit**

Run:
```bash
cd /home/andrii/Projects/labwired/packages/playground && npx tsc --noEmit && npx vitest run src/board-resolve.test.ts
git add packages/playground/src/board-resolve.ts packages/playground/src/board-resolve.test.ts packages/playground/src/App.tsx
git commit -m "feat(playground): resolve MCU part board via attrs.boardId (fix nRF collision)"
```
Expected: tsc exit 0; 5 tests pass.

---

## Task 2: `nrf52840-ble-lab` config with a two-MCU starter diagram

**Files:**
- Modify: `packages/playground/src/bundled-configs.ts`

- [ ] **Step 1: Learn the lab-config + starter-diagram authoring shape**

Run:
```bash
cd /home/andrii/Projects/labwired/packages/playground/src
sed -n '55,118p' bundled-configs.ts          # BoardConfig interface (find the diagram/parts field name)
sed -n '294,320p' bundled-configs.ts         # adxl345-sensor-lab — a kind:'lab' entry with peripherals
sed -n '382,445p' bundled-configs.ts         # the four nRF entries (sensor/collector exact fields)
grep -n "diagram\|parts\|id: 'mcu'\|x:\|y:\|rotate" bundled-configs.ts | head -40
```
From this, note: (a) the exact field a `BoardConfig` uses to carry its starter diagram (a `diagram`/`workspace`/`parts` field, OR whether `loadBoardWorkspace` synthesizes it — if a multi-part lab like adxl345 exists, the field exists; use the same one); (b) the `Part` literal shape used there (`{ id, type, x, y, rotate, attrs }`); (c) the exact `chipYaml`/`systemYaml`/`demoFirmwarePath`/`name` values on `nrf52840-ble-sensor` and `nrf52840-ble-collector`.

- [ ] **Step 2: Add the lab config**

In `bundled-configs.ts`, add a new entry to `BOARD_CONFIGS` (place it next to the other nRF entries). Mirror the field shape discovered in Step 1; the example below uses the conventional `parts` array — adapt field names to match the existing lab entries exactly:
```ts
{
  boardId: 'nrf52840-ble-lab',
  name: 'nRF52840 BLE Lab (2 boards)',
  // Two nRF52840s on one canvas. Each MCU part carries attrs.boardId so the
  // resolver (board-resolve.ts) gives each its own firmware: the sensor runs
  // the BLE-advertise/TX demo, the collector runs the scan/RX demo. Both use
  // the onboarding chip/system (RADIO + CLOCK present; the DK has neither).
  chipId: 'nrf52840',
  chipYaml: chipNrf52840Onboarding,
  systemYaml: systemNrf52840Onboarding,
  mcuComponentType: 'nrf52840-dk',
  kind: 'lab',
  // No single demoFirmwarePath — each MCU part loads its own via attrs.boardId.
  diagram: {
    parts: [
      // Keep one part id === 'mcu' so foregroundPartId's default ('mcu') has a
      // target on first load. attrs.boardId still steers its firmware.
      { id: 'mcu', type: 'nrf52840-dk', x: 180, y: 220, rotate: 0,
        attrs: { boardId: 'nrf52840-ble-sensor' } },
      { id: 'mcu-collector', type: 'nrf52840-dk', x: 620, y: 220, rotate: 0,
        attrs: { boardId: 'nrf52840-ble-collector' } },
    ],
    connections: [],
  },
},
```
NOTE: if the existing lab entries declare their diagram under a different key (e.g. `workspace`, or a top-level `parts`), use that key instead — match the surrounding entries verbatim. If `BoardConfig` lacks any diagram field and `loadBoardWorkspace` synthesizes from `mcuComponentType`, then instead extend `loadBoardWorkspace` to special-case `boardId === 'nrf52840-ble-lab'` and return the two-part diagram above (document this in the commit).

- [ ] **Step 3: Confirm each MCU part loads its own firmware**

Run:
```bash
cd /home/andrii/Projects/labwired/packages/playground/src
grep -n "demoFirmwarePath\|attrs.boardId\|mcuBoardForPart\|firmware" ../src/App.tsx | sed -n '1,40p'
```
Confirm the per-part sim setup (where `usePerChipSims`/launch resolves a part's board) uses `mcuBoardForPart` (now `resolveBoardForPart`) and loads that board's `demoFirmwarePath`. If the foreground/primary path loads firmware from `selectedBoard` rather than from the part's resolved board, adjust so a part with `attrs.boardId` loads the resolved board's `demoFirmwarePath` (otherwise both parts would run the lab's firmware, not sensor/collector firmware). Add a focused note in the commit if a code change here is needed.

- [ ] **Step 4: Typecheck + commit**

Run:
```bash
cd /home/andrii/Projects/labwired/packages/playground && npx tsc --noEmit
git add packages/playground/src/bundled-configs.ts packages/playground/src/App.tsx
git commit -m "feat(playground): nRF52840 BLE Lab — two-board starter diagram"
```
Expected: exit 0.

---

## Task 3: Move the analyzer into a Tools menu + make it resizable

**Files:**
- Modify: `packages/playground/src/App.tsx` (analyzer mount ~2452–2465; add toggle state + Tools control)

- [ ] **Step 1: Locate the analyzer mount and the toolbar/dock it should live in**

Run:
```bash
cd /home/andrii/Projects/labwired/packages/playground/src
sed -n '2445,2470p' App.tsx                              # current always-on mount
grep -n "SimDock\|toolbar\|Tools\|button\|DevDrawer\|WatchOverlay" App.tsx | head -30
grep -n "button\|onClick\|aria-label\|title=" studio/SimDock.tsx 2>/dev/null | head
```
Identify (a) the JSX block that currently renders `<BleAnalyzer bridge={bridge} running={running} />`, and (b) an existing toolbar/dock with toggle buttons (e.g. `SimDock`) where a "Packet Analyzer" / "Tools" toggle button fits, following the existing button pattern.

- [ ] **Step 2: Add visibility state**

Near the other `useState` hooks in the App component, add:
```ts
// Analyzer is an opt-in instrument now (was an always-on panel that froze the
// canvas). Toggled from the Tools control; hidden by default.
const [showAnalyzer, setShowAnalyzer] = useState(false);
```

- [ ] **Step 3: Add the Tools toggle control**

In the toolbar/dock identified in Step 1, add a toggle button matching the existing button style (replace `className`/icon to match siblings):
```tsx
<button
  type="button"
  className="<copy the className of a sibling dock/toolbar button>"
  aria-pressed={showAnalyzer}
  title="Packet Analyzer (BLE air)"
  onClick={() => setShowAnalyzer((v) => !v)}
>
  Analyzer
</button>
```
If there is no obvious toolbar, add it to the same row as the Run control in `SimDock`.

- [ ] **Step 4: Replace the always-on mount with a resizable, dismissable host**

Replace the current `<BleAnalyzer … />` mount block (~2452–2465) with a gated, resizable container. Native CSS `resize` needs `overflow` set on the same element; the analyzer fills it via its own `h-full`:
```tsx
{showAnalyzer && (
  <div
    className="absolute bottom-4 right-4 z-30 flex flex-col rounded-lg border border-border bg-bg-base shadow-xl overflow-hidden"
    style={{
      resize: 'both',
      width: 520,
      height: 320,
      minWidth: 320,
      minHeight: 160,
      maxWidth: '90vw',
      maxHeight: '80vh',
    }}
  >
    <div className="flex items-center justify-between px-2 py-1 border-b border-border cursor-default">
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
NOTE: `resize: 'both'` requires `overflow` not `visible` on the resizing element — the outer `overflow-hidden` satisfies that, and the inner wrapper's `min-h-0 overflow-hidden` lets `BleAnalyzer`'s internal `overflow-auto` table scroll. The element must NOT be inside a parent that clips it (`absolute` + high `z-30` keeps it above the canvas). If the surrounding container is not `position: relative`, place this block inside the nearest relatively-positioned canvas wrapper, or change `absolute` to `fixed`.

- [ ] **Step 5: Typecheck + commit**

Run:
```bash
cd /home/andrii/Projects/labwired/packages/playground && npx tsc --noEmit
git add packages/playground/src/App.tsx
git commit -m "fix(playground): analyzer is a toggled, resizable Tools instrument (no more frozen panel)"
```
Expected: exit 0.

---

## Task 4: Live verification — the proof gate (do NOT merge before this passes)

**Files:** none (manual + Playwright MCP)

- [ ] **Step 1: Start the dev server without auth**

Run (background):
```bash
cd /home/andrii/Projects/labwired/packages/playground
VITE_DISABLE_AUTH=true npx vite --port 5173 --strictPort
```
Run this with `run_in_background: true`. To stop it later use `fuser -k 5173/tcp` — **never** `pkill -f vite` (it matches the agent's own shell and exits 144).

- [ ] **Step 2: Open the lab and verify two boards render**

Using Playwright MCP: `browser_navigate` to `http://localhost:5173`, open the board picker / command palette, choose **"nRF52840 BLE Lab (2 boards)"**. `browser_snapshot` + `browser_take_screenshot`.
Expected: **two nRF52840 boards visible** on one canvas (sensor + collector), side by side.

- [ ] **Step 3: Open the analyzer from Tools and check it is usable**

Click the **Analyzer** toggle. Verify the panel appears, has a visible resize affordance (drag the corner via `browser_drag` a few px), and scrolls — it must NOT freeze the page.
Expected: panel toggles on/off, resizes, does not lock the UI.

- [ ] **Step 4: Run and confirm packets cross**

Press **Run**. Wait ~2–3s (`browser_wait_for`). `browser_take_screenshot` of the analyzer.
Expected: the analyzer frame counter climbs and rows appear with a de-whitened **Reading** column that **increments** (sensor's counter), and the **Address**/**Freq** columns are populated — proof the collector is hearing the sensor over shared air, not a single board echoing itself. Cross-check via console: `browser_console_messages` for any errors.

- [ ] **Step 5: Reload persistence check**

`browser_navigate` to the same URL again (reload). Re-open the lab if needed.
Expected: the lab still loads with two boards and Run still produces packets (no broken state, no junk tile).

- [ ] **Step 6: Record the evidence**

Save the key screenshot(s); note frame counts and a couple of incrementing Reading values in the PR description. If ANY step fails, stop — fix the relevant task, do not merge.

- [ ] **Step 7: Stop the dev server**

Run: `fuser -k 5173/tcp`

---

## Task 5: Cleanup + PR

**Files:** none (git + GitHub)

- [ ] **Step 1: Full local check**

Run:
```bash
cd /home/andrii/Projects/labwired/packages/playground && npx tsc --noEmit && npx vitest run
```
Expected: tsc exit 0; all tests pass (including `board-resolve.test.ts`).

- [ ] **Step 2: Abandon the dead scratch branches**

Run (only delete branches that are fully superseded and unpushed):
```bash
cd /home/andrii/Projects/labwired
git branch -D fix/multichip-junk-tile fix/multichip-scratch-replace 2>/dev/null || true
git branch --list 'fix/multichip-*'
```
Expected: those branches gone (or already absent).

- [ ] **Step 3: Push and open the PR**

Run:
```bash
git push -u origin feat/nrf-ble-lab-example
gh pr create --fill --title "nRF52840 BLE Lab — prebuilt two-board example + analyzer Tools fix"
```
In the PR body include: the proof-gate screenshots/notes from Task 4, that `32c8ae4` is reverted here, and that the BLE decode logic is unchanged. Do NOT add any AI/Claude attribution or Co-Authored-By line (project convention).

- [ ] **Step 4: Confirm another agent hasn't moved main under you**

Run:
```bash
git fetch origin && git log --oneline origin/main -3
```
If main advanced, rebase `feat/nrf-ble-lab-example` onto it and re-run Task 4's proof gate before merge.

---

## Self-review notes

- **Spec coverage:** prebuilt lab (Task 2) ✓; both-boards-visible via Mechanism A (Tasks 1–2, verified Task 4) ✓; part→board identity fix (Task 1) ✓; analyzer Tools-menu + resizable (Task 3) ✓; revert 32c8ae4 (Task 0) ✓; live proof gate (Task 4) ✓; vitest for resolver (Task 1) ✓; deferred Add-MCU / Mechanism-B deletion explicitly out of scope ✓.
- **Recon-first steps (1.1, 2.1, 3.1):** authored during a tool-output outage; they instruct the implementer to read the exact existing patterns (`BoardConfig` diagram field, lab-entry shape, toolbar button style) before writing code, because those structural shapes were not re-confirmable live at planning time. Each is followed by concrete code to write against the confirmed shape — not a "TBD".
- **Type consistency:** the resolver is named `resolveBoardForPart` throughout; `mcuBoardForPart` keeps its name and delegates; `attrs.boardId` is the single disambiguation key everywhere.
- **Risk:** the one genuine unknown is how a `BoardConfig` carries its starter diagram (a field vs `loadBoardWorkspace` synthesis). Step 2.1 resolves it before any code; Step 2.2 covers both branches.
