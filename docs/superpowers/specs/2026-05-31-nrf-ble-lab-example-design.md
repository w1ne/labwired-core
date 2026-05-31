# nRF52840 BLE Lab — prebuilt two-board example (design)

Date: 2026-05-31
Status: APPROVED (design) — ready for writing-plans

## Problem

The goal is a LabWired playground demo where **two nRF52840 BLE boards on one
canvas communicate** (sensor transmits, collector receives) with packets visible
in the BLE analyzer. The engine already does this honestly: two `SimulatorBridge`
instances share a Rust-side process-global virtual-air static
(`OnceLock<Mutex<VirtualAir>>`), frames cross, CRC is verified, readings
de-whitened. The UI path is what's broken.

### Root cause of the four failed attempts
The playground has **two parallel multi-chip systems**:
- **Mechanism A — `usePerChipSims`** (App.tsx): keyed by diagram **part id**.
  Built for "both chips live on one canvas" — every MCU *part* on one shared
  diagram gets its own bridge; the selected part is foreground (main loop), the
  rest are background-ticked. No diagram swap. Already wired into App.tsx
  (foregroundPartId @1135, hook @1140, per-part render `isFg` @2352, BLE detect
  @2457).
- **Mechanism B — `ChipsProvider` + `ChipBridgeSync` + `McuStrip`**: keyed by
  **chipId**, a registry of separate *workspaces*; switching a tab swaps the
  whole diagram (only one board visible). `ChipBridgeSync` mirrors
  `App.selectedBoard` onto the active chip every render — this clobber defeated
  all four add-vs-replace fixes.

All four failed fixes targeted **Mechanism B**, the wrong subsystem for "two
boards on one canvas." Mechanism B is self-contained: `useChips`/`McuStrip`/
`ChipBridgeSync` are consumed only by App.tsx + the `multi-mcu/*` files + their
own tests + `useCommandPaletteItems.ts`. Nothing else depends on it.

## Decision: ship nRF as a prebuilt EXAMPLE (not Add-MCU assembly)

Per user ("for the nrf we can just have an example!"): instead of building the
general Add-MCU-onto-canvas UX, ship the nRF two-board case as a ready-made lab
the user just opens. Minimal scope; sidesteps the entire add-vs-replace/junk-tile
problem class. Uses **Mechanism A only**; never touches Mechanism B.

## Design

1. **The example artifact.** A new bundled lab config (`nrf52840-ble-lab`,
   `kind:'lab'`) whose starter **diagram contains two MCU parts** — sensor +
   collector — pre-placed side by side, plus the BLE analyzer panel. Fixed
   content, not user-assembled.

2. **Part→board identity (the one real code fix).** `mcuBoardForPart`
   (App.tsx:532) currently resolves a part by `mcuComponentType` first-match —
   and all nRF boards share `mcuComponentType: 'nrf52840-dk'`, so both parts
   collapse to the wrong (RADIO-less DK) config. Fix: each MCU part carries
   **`attrs.boardId`** (`nrf52840-ble-sensor` / `nrf52840-ble-collector`), and
   `mcuBoardForPart` resolves `attrs.boardId` first, falling back to the existing
   type match. This lets two same-package parts run *different firmware* (sensor
   TX vs collector RX) on the RADIO-equipped onboarding chip/system.
   - Existing BLE board entries (`nrf52840-ble-sensor`, `nrf52840-ble-collector`)
     already use `chipYaml: chipNrf52840Onboarding` + `systemYaml:
     systemNrf52840Onboarding` (have RADIO + CLOCK; the DK does NOT) and carry
     `demoFirmwarePath`. Each part runs its board's demo firmware.

3. **Both run, both talk — Mechanism A only.** `usePerChipSims` gives each MCU
   part its own bridge: selected part on the main loop, the other background-
   ticked (200k cyc/16ms). Both share the virtual-air static → frames cross.
   No ChipsProvider/McuStrip/addChip involvement.

4. **Packets + analyzer UX fix.** `BleAnalyzer` already snapshots the air-trace
   (`airTraceSnapshot()` exposes `AirFrameTrace` with `whitening_iv`),
   de-whitens `bytes[..len-3]` (CRC appended post-whitening), and renders the
   decode correctly. BUT the panel itself is currently **frozen, unresizable,
   and useless** as a floating widget. Required UX rework:
   - **Move it into a Tools/Instruments menu** (toggle on/off from there) instead
     of an always-on floating panel that blocks the canvas.
   - **Make it resizable and scrollable** — the packet list must grow/shrink and
     scroll; the panel must not freeze the UI or pin to an unusable fixed size.
   - It should open empty and fill live as packets cross the air during Run.
   - Decode logic (de-whitening/CRC) stays as-is — this is purely the panel's
     mount point, sizing, and visibility control.

5. **Proof before any merge (live, not typecheck):** load "nRF52840 BLE Lab" →
   **two boards visible** → open the analyzer from the Tools menu → Run →
   **analyzer fills with packets**, is **resizable + scrollable**, does not
   freeze → reload → still works. Plus a vitest for `mcuBoardForPart` attrs
   resolution.

## Scope explicitly deferred (YAGNI)
- The general Add-MCU-onto-canvas assembly UX.
- Deletion of Mechanism B's files (another agent is active; don't churn shared
  files we don't need to).

## Resolved decisions
1. **Visibility: BOTH boards visible on one canvas** (Mechanism A). The selected
   part is foreground (main loop); the other background-ticks. Matches the
   original "two boards on one canvas" intent.
2. **Revert `32c8ae4` on the feature branch** so main is clean after merge — the
   broken junk-tile fix targeted the Add-MCU-replace path we are no longer using
   for nRF, so it is dead weight.

## Implementation notes (for writing-plans, after sign-off)
- Base: clean off main @32c8ae4 (compiles; the "duplicate imports" I worried
  about mid-investigation was a Read-tool artifact — `git show` confirms one of
  each import). Abandon scratch branches `fix/multichip-junk-tile`,
  `fix/multichip-scratch-replace`.
- Files likely touched: `bundled-configs.ts` (new lab entry + 2-MCU starter
  diagram with `attrs.boardId` on each part), `App.tsx` `mcuBoardForPart`
  (attrs.boardId-first resolution) + analyzer mount/visibility, the Tools/
  Instruments menu component, `instruments/BleAnalyzer.tsx` (resizable +
  scrollable container; decode logic untouched).
- Dev server: `VITE_DISABLE_AUTH=true npx vite --port 5173 --strictPort`
  (run_in_background). Never `pkill -f vite` (kills own shell); use
  `fuser -k 5173/tcp`. Verify in browser via Playwright MCP.
