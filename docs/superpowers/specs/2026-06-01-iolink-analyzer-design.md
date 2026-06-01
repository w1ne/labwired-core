# IO-Link Analyzer — Design

**Date:** 2026-06-01
**Status:** Approved (design)
**Area:** `core` (IO-Link master trace tap), `packages/ui` (wasm bridge), `packages/playground` (instrument UI)
**Builds on:** `feat/iolink-playground-ui` (NOT `main` — see Dependency)

## Summary

A floating **IO-Link Analyzer** instrument in the playground Tools menu (sibling to the existing Air Tracer) that surfaces the live master↔device IO-Link protocol as **per-cycle transaction rows**, with a link-state phase strip, error highlighting, expandable raw frames, and copy-to-clipboard capture. It is built as the IO-Link decode layer over a frame tap on the simulated master — i.e. a focused, IO-Link-aware subset of a future generic UART analyzer.

The value: an IO-Link audience can *watch* the wake-up → startup → preoperate → operate handshake, read decoded PD/OD per cycle, and see CRC verdicts live — all from real firmware driving the modeled master, in the browser.

## Goals

- Show real IO-Link transactions live, decoded to protocol semantics (type, PD out/in, OD, CRC, link state).
- Mirror the proven Air Tracer pattern (core trace ring → wasm snapshot → polling instrument).
- Be a compelling, honest pitch artifact for the IO-Link team.

## Non-Goals (v1)

- **Fault injection.** No "corrupt a frame" control; CRC/validity columns reflect real verdicts only (clean in steady OPERATE; transient invalids may appear during startup).
- **Generic UART analyzer UI.** The core tap + decoder are structured so a generic UART view could be added later, but only the IO-Link instrument ships now.
- **Deep ISDU / parameter-service decode.** The on-request data (OD) byte is shown raw; full ISDU service dissection is future work.

## Decision Log

- **Row model:** per-cycle transaction (request paired with response), each row expandable to the raw `M→D` / `D→M` frame bytes. Chosen over a flat per-frame UART stream — the wire view lives *inside* the transaction view. (Rationale: speaks the IO-Link audience's language while retaining wire-level credibility.)
- **Scope:** pitch-polished — transaction table **plus** a live link-state phase strip, CRC/error row highlighting, and a Copy-capture button. Chosen over a bare table.
- **Errors:** natural verdicts only (no fault injection) — leaner and fully honest.
- **Decode location:** in Rust (the master already encodes/decodes), emitted as structured records. The UI never re-implements CRC6/framing. (Rationale: DRY; the protocol logic exists once.)
- **Dependency branch:** `feat/iolink-playground-ui`, the only branch carrying the runnable master + live bindings.

## Architecture

Four units with clean boundaries, mirroring Air Tracer (RADIO trace ring → `airTraceSnapshot()` → `BleAnalyzer`):

```
IolinkMaster (sim, ring) → iolinkTraceSnapshot() (wasm bridge) → iolinkDecode (format) → IoLinkAnalyzer (UI, Tools menu)
```

### Unit 1 — Core trace ring on `IolinkMaster`
**Where:** `core/crates/core/src/peripherals/components/iolink_master.rs` (at the IO-Link demo core revision, `core@3b203e7` lineage).

**What:** A bounded ring (cap ~256) on the master. The master already builds requests (`encode_type1_cycle`) and parses responses (`decode_operate` → `OperateResponse { pd_valid, checksum_ok }`). On each completed cycle it pushes one already-decoded record:

```
IolinkXfer {
  seq: u32, tick: u64,
  kind: WakeUp | Idle | Cyclic,
  com_speed: Com1 | Com2 | Com3,
  pd_out: Vec<u8>, pd_in: Vec<u8>, od: u8,
  ck_ok: bool, pd_valid: bool,
  link_state: <startup machine state>,
  raw_master: Vec<u8>, raw_device: Vec<u8>,
}
```

**API:** `trace_snapshot() -> Vec<IolinkXfer>` (clone of the ring, oldest→newest) and `trace_clear()`. No protocol logic is added — only recording of what the master already computes.

**Depends on:** existing `encode_type1_cycle` / `decode_operate` / `link_state` / `com_speed`.

### Unit 2 — Wasm bridge
**Where:** `packages/ui/src/wasm/simulator-bridge.ts` and the `core/crates/wasm` glue, alongside the existing `getIolinkMasterState()` (which already locates the master instance).

**What:** `iolinkTraceSnapshot(): IolinkXfer[]` and `iolinkTraceClear(): void`. Returns a typed JS array (a copy per call). Mirrors `airTraceSnapshot()`'s shape and lifecycle.

**Depends on:** Unit 1; the existing master-locator path used by `getIolinkMasterState()`.

### Unit 3 — Decode/format module
**Where:** `packages/playground/src/instruments/iolinkDecode.ts`.

**What:** Pure presentation helpers — `bytesToHex`, `kindLabel`, `linkStateToPhaseIndex`, `errorCount`, `filterErrorsOnly`, `toCsv`/`toHexDump` for Copy. No protocol/CRC logic. This is the seam where a generic `uartDecode` could later slot in.

**Depends on:** the `IolinkXfer` type from the bridge.

### Unit 4 — Instrument UI
**Where:** `packages/playground/src/instruments/IoLinkAnalyzer.tsx`, registered in `packages/playground/src/studio/ToolsMenu.tsx`, opened as a `ChipWindow`. Cloned from `BleAnalyzer.tsx`.

**What:** Polls `iolinkTraceSnapshot()` every ~200 ms while the sim runs. Renders:
- **Phase strip:** WAKE-UP / STARTUP / PREOPERATE / OPERATE, current phase highlighted, derived from the latest record's `link_state`.
- **Toolbar:** frame count, COM rate, error count, "errors only" filter, Copy-capture button.
- **Transaction table:** `# | t(ms) | Type | PD out | PD in | OD | CK | Link`, invalid rows highlighted, each row expandable to raw `M→D` / `D→M` hex.

**Depends on:** Units 2–3; `ToolsMenu`, `ChipWindow`.

## Data Flow

Sim steps → master pushes one decoded `IolinkXfer` per completed cycle into its ring. The analyzer's poll loop (~200 ms, only while running) calls `iolinkTraceSnapshot()` → fresh `IolinkXfer[]` copy → `iolinkDecode` formats → React renders the table, derives the phase strip from the latest `link_state`, and counts errors. **Copy** serializes the current snapshot to hex/CSV onto the clipboard; **Clear** calls `iolinkTraceClear()`.

## Error Handling

- No IO-Link master in the circuit / sim idle → `iolinkTraceSnapshot()` returns `[]`; analyzer shows an empty state ("Add an IO-Link master and press Run").
- Ring bounded (~256); oldest dropped; toolbar notes "last N".
- Genuinely invalid frames (`ck_ok=false` / `pd_valid=false`, mostly transient during startup) render as highlighted rows — real verdicts, never fabricated.
- Each poll returns a copy across the wasm boundary; no shared mutable state.

## Testing

- **Rust:** ring behavior (push / cap at 256 / clear); a stepped master+device reaches OPERATE and the ring fills with `Cyclic` xfers carrying correct `pd_in` + `ck_ok`. Reuses existing encode/decode.
- **TS:** `iolinkDecode` pure-function unit tests (`node --test`) — hex formatting, kind→label, link→phase index, error-count and errors-only filter, CSV/hex export.
- **UI:** manual in-browser via `serve`; if the automated browser backend is unavailable, verify the data path headlessly and provide a serve one-liner.

## Dependency / Sequencing

The runnable IO-Link demo (firmware ELF, live bridge bindings, example entry) currently lives **only on `feat/iolink-playground-ui`** (firmware source + docs on `feat/iolink-dido-device-demo`); `main` carries only the static `iolink-master.tsx` component shell. Therefore:

1. This analyzer **branches from `feat/iolink-playground-ui`**, not `main`.
2. To demo live on app.labwired.com, the IO-Link demo must land on `main` and deploy **first**; the analyzer rides on top. Merging the demo to main is a **separate prerequisite track**, out of scope for this spec. The analyzer can be built and reviewed against the feature branch in parallel.
