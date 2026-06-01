# IO-Link Analyzer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an "IO-Link Analyzer" instrument to the playground Tools menu that taps the simulated IO-Link master and renders the live master↔device protocol as per-cycle transaction rows (kind, PD, CRC, link-state), with a phase strip, error highlighting, expandable raw frames, and copy-to-clipboard.

**Architecture:** Clone the proven Air Tracer pattern — a bounded trace ring on the core `IolinkMaster` (decoded where the master already encodes/parses frames) → a `iolink_trace_snapshot()` wasm export walking the UART→`IolinkMaster` path → a typed `iolinkTraceSnapshot()` bridge method → a pure `iolinkDecode.ts` formatter → an `IoLinkAnalyzer.tsx` React instrument in a `ChipWindow`.

**Tech Stack:** Rust (labwired-core, serde), `wasm-bindgen` + `serde_wasm_bindgen`, TypeScript/React (`@labwired/ui` bridge), vitest (playground tests), `node`/`cargo` test runners, `wasm-pack` for the engine rebuild.

**Build base & branches:** This builds on the PARENT branch `feat/iolink-playground-ui` (the only branch with the runnable IO-Link master + live bindings), which pins the CORE submodule at `3b203e7`. Work happens in TWO isolated worktrees:
- **core** worktree off `core@3b203e7` (its own repo, labwired-core) — Tasks 1–3 (Rust + wasm).
- **parent** worktree off `feat/iolink-playground-ui` (labwired) — Tasks 4–7 (bridge TS, decoder, UI), plus the rebuilt wasm artifact committed here.

The core (`core`) is a git submodule; commit Rust/wasm changes inside the core worktree, commit TS/UI + the rebuilt `.wasm`/`.js` inside the parent worktree. Do NOT push or merge without the operator's say-so.

**Deviations from the design mockup (grounded in the actual model, intentional):**
1. The ordinal column is a per-frame **`seq`** (the master has no wall clock), not `t (ms)`.
2. `ck_ok` and `pd_valid` are `Option<bool>` — `null`/"—" for non-cyclic startup frames (which have no decodable OPERATE response), `true`/`false` only for cyclic frames. This avoids painting every startup row with a false "✗".
3. For this DI demo `pd_out` and `od` are empty/idle, so those columns will mostly render "—"; the columns still exist (data-model parity with the spec) and would populate for a PD-carrying device.

---

## File Structure

- **Modify** `core/crates/core/src/peripherals/components/iolink_master.rs` — add `IolinkFrameKind`, `IolinkXfer`, a bounded trace ring + `trace_snapshot()`/`trace_clear()`; capture each frame's raw master bytes and accumulated device response. (Tasks 1–2)
- **Modify** `core/crates/core/src/peripherals/components/mod.rs` — re-export the new public types (if the module uses an explicit re-export list).
- **Modify** `core/crates/wasm/src/lib.rs` — add `iolink_trace_snapshot()` (and `iolink_trace_clear()`) exports walking the UART→`IolinkMaster` path. (Task 3)
- **Modify** `packages/ui/src/wasm/simulator-bridge.ts` — add the `IolinkXfer` interface + `iolinkTraceSnapshot()`/`iolinkTraceClear()` methods + the raw `WasmSimulatorInstance` decls. (Task 4)
- **Create** `packages/playground/src/instruments/iolinkDecode.ts` — pure formatters (hex, kind label, link→phase index, error count, errors-only filter, CSV/hex export). (Task 5)
- **Create** `packages/playground/src/instruments/iolinkDecode.test.ts` — vitest unit tests for the formatters. (Task 5)
- **Create** `packages/playground/src/instruments/IoLinkAnalyzer.tsx` — the instrument (phase strip + toolbar + table + expandable rows). (Task 6)
- **Modify** `packages/playground/src/App.tsx` — register the tool in `ToolsMenu` + mount a `ChipWindow`. (Task 6)
- **Modify** `packages/playground/public/wasm/labwired_wasm_bg.wasm` + `labwired_wasm.js` — rebuilt engine carrying the new export. (Task 7)

**Data contract (shared across all tasks):**

Rust (`iolink_master.rs`), serialized to JS as plain objects:
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IolinkFrameKind { WakeUp, Idle, OperateReq, Cyclic }

#[derive(Debug, Clone, serde::Serialize)]
pub struct IolinkXfer {
    pub seq: u32,
    pub kind: IolinkFrameKind,
    pub com: IolinkComSpeed,
    pub pd_out: Vec<u8>,
    pub pd_in: Vec<u8>,
    pub od: u8,
    pub ck_ok: Option<bool>,
    pub pd_valid: Option<bool>,
    pub link_state: IolinkLinkState,
    pub raw_master: Vec<u8>,
    pub raw_device: Vec<u8>,
}
```

TypeScript (`simulator-bridge.ts`):
```ts
export interface IolinkXfer {
  seq: number;
  kind: 'wake_up' | 'idle' | 'operate_req' | 'cyclic';
  com: 'com1' | 'com2' | 'com3';
  pd_out: number[];
  pd_in: number[];
  od: number;
  ck_ok: boolean | null;
  pd_valid: boolean | null;
  link_state: 'startup' | 'operate';
  raw_master: number[];
  raw_device: number[];
}
```

---

## Task 1: Core — trace record types + per-frame builder

**Files:**
- Modify: `core/crates/core/src/peripherals/components/iolink_master.rs`

Add the new types and a pure builder that turns a finalized in-flight frame into an `IolinkXfer`. TDD: test the builder against the existing IO-Link CRC vectors before wiring it into the device.

- [ ] **Step 1: Add the types and a `PendingXfer` + builder**

Insert after the `IolinkLinkState` enum (around `iolink_master.rs:88`):

```rust
/// Which frame in the startup/cyclic schedule a trace record came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IolinkFrameKind {
    WakeUp,
    Idle,
    OperateReq,
    Cyclic,
}

/// One captured master↔device exchange, decoded where the master already
/// builds requests and parses responses. Serialized to JS as a plain object
/// (tagged struct via serde_wasm_bindgen).
#[derive(Debug, Clone, serde::Serialize)]
pub struct IolinkXfer {
    pub seq: u32,
    pub kind: IolinkFrameKind,
    pub com: IolinkComSpeed,
    pub pd_out: Vec<u8>,
    pub pd_in: Vec<u8>,
    pub od: u8,
    /// `None` for non-cyclic frames (no decodable OPERATE response).
    pub ck_ok: Option<bool>,
    pub pd_valid: Option<bool>,
    pub link_state: IolinkLinkState,
    pub raw_master: Vec<u8>,
    pub raw_device: Vec<u8>,
}

/// In-flight frame: its request bytes are known at queue time; the device
/// response accumulates until the next frame is queued, then it's finalized.
#[derive(Debug, Clone)]
struct PendingXfer {
    seq: u32,
    kind: IolinkFrameKind,
    pd_out: Vec<u8>,
    link_state: IolinkLinkState,
    raw_master: Vec<u8>,
    raw_device: Vec<u8>,
}

/// Max trace records retained (oldest dropped). Mirrors the air-trace ring size.
const TRACE_CAP: usize = 256;
```

- [ ] **Step 2: Add a pure finalizer + write its failing test**

Add this associated function on `IolinkMaster` (inside `impl IolinkMaster`, after `operate_response_len`):

```rust
    /// Turn a completed in-flight frame into a trace record, decoding the
    /// device response only for cyclic (OPERATE) frames.
    fn finalize_xfer(&self, p: PendingXfer) -> IolinkXfer {
        let (pd_in, ck_ok, pd_valid) = if matches!(p.kind, IolinkFrameKind::Cyclic) {
            let n = self.operate_response_len();
            if p.raw_device.len() >= n {
                let r = decode_operate(&p.raw_device[..n], self.pd_in_len, self.od_len);
                (r.pd, Some(r.checksum_ok), Some(r.pd_valid))
            } else {
                (Vec::new(), Some(false), Some(false))
            }
        } else {
            (Vec::new(), None, None)
        };
        IolinkXfer {
            seq: p.seq,
            kind: p.kind,
            com: self.com,
            pd_out: p.pd_out,
            pd_in,
            od: 0x00,
            ck_ok,
            pd_valid,
            link_state: p.link_state,
            raw_master: p.raw_master,
            raw_device: p.raw_device,
        }
    }
```

Add this test inside the existing `#[cfg(test)] mod tests` block (after `decodes_operate_response_and_extracts_pd`):

```rust
    #[test]
    fn finalize_cyclic_decodes_response_and_marks_ck() {
        let m = IolinkMaster::new(1, 1, IolinkComSpeed::Com2);
        // A valid cyclic OPERATE response: [status=0x20, PD_in=0xA5, OD=0x00, CK].
        let resp = [0x20u8, 0xA5, 0x00, crc6(&[0x20, 0xA5, 0x00])];
        let p = PendingXfer {
            seq: 7,
            kind: IolinkFrameKind::Cyclic,
            pd_out: vec![],
            link_state: IolinkLinkState::Operate,
            raw_master: encode_type1_cycle(&[]),
            raw_device: resp.to_vec(),
        };
        let x = m.finalize_xfer(p);
        assert_eq!(x.seq, 7);
        assert_eq!(x.kind, IolinkFrameKind::Cyclic);
        assert_eq!(x.pd_in, vec![0xA5]);
        assert_eq!(x.ck_ok, Some(true));
        assert_eq!(x.pd_valid, Some(true));
    }

    #[test]
    fn finalize_startup_frame_has_no_crc_verdict() {
        let m = IolinkMaster::new(1, 1, IolinkComSpeed::Com2);
        let p = PendingXfer {
            seq: 0,
            kind: IolinkFrameKind::WakeUp,
            pd_out: vec![],
            link_state: IolinkLinkState::Startup,
            raw_master: vec![0x55],
            raw_device: vec![],
        };
        let x = m.finalize_xfer(p);
        assert_eq!(x.ck_ok, None);
        assert_eq!(x.pd_valid, None);
        assert!(x.pd_in.is_empty());
    }
```

- [ ] **Step 3: Run the tests to verify they fail**

Run (inside the core worktree):
```bash
cargo test -p labwired-core --lib iolink_master 2>&1 | tail -20
```
Expected: compile error or FAIL — `PendingXfer`/`finalize_xfer`/`IolinkXfer` referenced before the device wiring exists. (At this point the types + finalizer exist but `current`/`trace` fields aren't on the struct yet; the test only uses `finalize_xfer`, which compiles. Expected result: the two new tests PASS, the rest still pass — if so, this step's "fail" is satisfied by first confirming a clean compile. If it does not compile, fix the type definitions until it does.)

- [ ] **Step 4: Confirm the finalizer tests pass**

Run:
```bash
cargo test -p labwired-core --lib iolink_master 2>&1 | tail -20
```
Expected: PASS, including `finalize_cyclic_decodes_response_and_marks_ck` and `finalize_startup_frame_has_no_crc_verdict`.

- [ ] **Step 5: Commit**

```bash
cd <core-worktree>
git add crates/core/src/peripherals/components/iolink_master.rs
git commit -m "feat(iolink): trace record types + per-frame finalizer"
```

---

## Task 2: Core — wire the ring into the master

**Files:**
- Modify: `core/crates/core/src/peripherals/components/iolink_master.rs`

Capture each outbound frame's raw bytes, accumulate the device response, finalize-on-next-queue into the ring, and expose `trace_snapshot()`/`trace_clear()`.

- [ ] **Step 1: Add ring + bookkeeping fields to `IolinkMaster`**

In the `IolinkMaster` struct (after the `pub pd_valid: bool` field), add:

```rust
    /// Bounded ring of completed transactions (oldest→newest), for the analyzer.
    #[serde(skip)]
    trace: VecDeque<IolinkXfer>,
    /// The frame currently in flight (request sent, response accumulating).
    #[serde(skip)]
    current: Option<PendingXfer>,
    /// Monotonic per-frame sequence number.
    #[serde(skip)]
    frame_seq: u32,
```

In `IolinkMaster::new`, initialize them in the struct literal (before `m.queue_next_frame()`):

```rust
            trace: VecDeque::new(),
            current: None,
            frame_seq: 0,
```

- [ ] **Step 2: Refactor `queue_next_frame` to record frames + finalize the previous one**

Replace the entire `queue_next_frame` body with this version (it now builds each frame into a `Vec` so the raw bytes can be recorded, finalizes the previously in-flight frame into the ring, and starts a new `PendingXfer`):

```rust
    /// Queue the next frame in the startup/cyclic schedule and advance `step`.
    /// Also finalizes the previous in-flight frame into the trace ring.
    fn queue_next_frame(&mut self) {
        // Finalize the previous frame (its response accumulated during the gap).
        if let Some(p) = self.current.take() {
            let x = self.finalize_xfer(p);
            if self.trace.len() >= TRACE_CAP {
                self.trace.pop_front();
            }
            self.trace.push_back(x);
        }
        self.rx_accum.clear();

        let idle_end = 1 + IDLE_FRAMES; // steps [1..=IDLE_FRAMES] are IDLE
        let (frame, kind): (Vec<u8>, IolinkFrameKind) = if self.step == 0 {
            (vec![0x55], IolinkFrameKind::WakeUp) // wake-up pulse (once)
        } else if self.step < idle_end {
            (encode_type0(0x00), IolinkFrameKind::Idle) // Type 0 IDLE → PREOPERATE
        } else if self.step == idle_end {
            (encode_type0(0x0F), IolinkFrameKind::OperateReq) // OPERATE transition
        } else {
            self.link_state = IolinkLinkState::Operate;
            (encode_type1_cycle(&[]), IolinkFrameKind::Cyclic) // cyclic Type 1
        };

        let pd_out: Vec<u8> = Vec::new(); // DI device: master sends no PD out
        for &b in &frame {
            self.tx_queue.push_back(b);
        }
        self.current = Some(PendingXfer {
            seq: self.frame_seq,
            kind,
            pd_out,
            link_state: self.link_state,
            raw_master: frame,
            raw_device: Vec::new(),
        });
        self.frame_seq = self.frame_seq.wrapping_add(1);

        // Hold `step` at the first cyclic index so it keeps repeating Type 1.
        if self.step <= idle_end {
            self.step += 1;
        }
    }
```

- [ ] **Step 3: Accumulate the device response into the in-flight frame**

In `on_tx_byte`, after the existing `if self.rx_accum.len() < 64 { self.rx_accum.push(byte); }` line, add:

```rust
        if let Some(p) = self.current.as_mut() {
            if p.raw_device.len() < 64 {
                p.raw_device.push(byte);
            }
        }
```

- [ ] **Step 4: Add the public snapshot/clear accessors**

Add to `impl IolinkMaster` (after `com_speed`):

```rust
    /// Snapshot of captured transactions, oldest→newest. Cloned for the UI.
    pub fn trace_snapshot(&self) -> Vec<IolinkXfer> {
        self.trace.iter().cloned().collect()
    }

    /// Clear the trace ring (the analyzer's "Clear" control).
    pub fn trace_clear(&mut self) {
        self.trace.clear();
    }
```

- [ ] **Step 5: Write the integration test (ring fills with the startup→cyclic sequence)**

Add to the `#[cfg(test)] mod tests` block:

```rust
    #[test]
    fn trace_ring_captures_startup_then_cyclic() {
        let mut m = IolinkMaster::new(1, 1, IolinkComSpeed::Com2);
        // Pump enough ticks to queue wake-up → IDLEs → OPERATE req → several
        // cyclic frames. Each frame queue happens after FRAME_GAP_TICKS idle polls.
        for _ in 0..(FRAME_GAP_TICKS as u64 * 10 + 64) {
            let _ = m.poll(1000);
        }
        let trace = m.trace_snapshot();
        assert!(trace.len() >= 5, "expected several frames, got {}", trace.len());
        // First captured frame is the wake-up.
        assert_eq!(trace[0].kind, IolinkFrameKind::WakeUp);
        // The schedule reaches cyclic frames in OPERATE.
        assert!(
            trace.iter().any(|x| x.kind == IolinkFrameKind::Cyclic
                && x.link_state == IolinkLinkState::Operate),
            "expected a cyclic OPERATE frame in the trace"
        );
        // Seqs are monotonic.
        for w in trace.windows(2) {
            assert!(w[1].seq > w[0].seq);
        }
    }

    #[test]
    fn trace_clear_empties_ring() {
        let mut m = IolinkMaster::new(1, 1, IolinkComSpeed::Com2);
        for _ in 0..(FRAME_GAP_TICKS as u64 * 3 + 16) {
            let _ = m.poll(1000);
        }
        assert!(!m.trace_snapshot().is_empty());
        m.trace_clear();
        assert!(m.trace_snapshot().is_empty());
    }
```

- [ ] **Step 6: Run the tests**

Run:
```bash
cargo test -p labwired-core --lib iolink_master 2>&1 | tail -25
```
Expected: PASS — all prior tests plus `trace_ring_captures_startup_then_cyclic` and `trace_clear_empties_ring`. If `trace.len()` is short, increase the poll-loop iteration count (the device is paced by `FRAME_GAP_TICKS`); do not change `FRAME_GAP_TICKS`.

- [ ] **Step 7: Ensure the new types are exported from the components module**

Check `core/crates/core/src/peripherals/components/mod.rs`. If it re-exports `IolinkMaster`/`IolinkComSpeed`/`IolinkLinkState` via an explicit `pub use`, add `IolinkXfer` and `IolinkFrameKind` to that list. Verify:
```bash
grep -n "Iolink" crates/core/src/peripherals/components/mod.rs
cargo build -p labwired-core 2>&1 | tail -5
```
Expected: builds clean; `IolinkXfer` and `IolinkFrameKind` are reachable as `labwired_core::peripherals::components::{IolinkXfer, IolinkFrameKind}` (add the `pub use` if not).

- [ ] **Step 8: Commit**

```bash
git add crates/core/src/peripherals/components/iolink_master.rs crates/core/src/peripherals/components/mod.rs
git commit -m "feat(iolink): trace ring on the master + snapshot/clear accessors"
```

---

## Task 3: Wasm — `iolink_trace_snapshot` export

**Files:**
- Modify: `core/crates/wasm/src/lib.rs`

Mirror `air_trace_snapshot` (serialize a `Vec` of tagged structs directly → plain JS objects), but locate the master via the UART path used by `get_iolink_master_state`.

- [ ] **Step 1: Add the exports**

Next to `get_iolink_master_state` in `crates/wasm/src/lib.rs`, add:

```rust
    /// Snapshot of the IO-Link master's captured transactions (oldest→newest),
    /// for the IO-Link Analyzer instrument. Empty array if no master is wired.
    #[wasm_bindgen]
    pub fn iolink_trace_snapshot(&self) -> JsValue {
        use labwired_core::peripherals::components::IolinkMaster;
        let Some(machine) = self.machine.as_ref() else {
            return serde_wasm_bindgen::to_value(&Vec::<u8>::new()).unwrap_or(JsValue::NULL);
        };
        for p in &machine.bus.peripherals {
            let Some(any) = p.dev.as_any() else { continue };
            let Some(uart) =
                any.downcast_ref::<labwired_core::peripherals::uart::Uart>()
            else {
                continue;
            };
            for stream in &uart.attached_streams {
                if let Some(m) = stream.as_any().and_then(|a| a.downcast_ref::<IolinkMaster>()) {
                    let trace = m.trace_snapshot();
                    return serde_wasm_bindgen::to_value(&trace).unwrap_or(JsValue::NULL);
                }
            }
        }
        serde_wasm_bindgen::to_value(&Vec::<IolinkXferUnit>::new()).unwrap_or(JsValue::NULL)
    }

    /// Clear the IO-Link master's trace ring.
    #[wasm_bindgen]
    pub fn iolink_trace_clear(&mut self) {
        use labwired_core::peripherals::components::IolinkMaster;
        let Some(machine) = self.machine.as_mut() else { return };
        for p in &mut machine.bus.peripherals {
            let Some(any) = p.dev.as_any_mut() else { continue };
            let Some(uart) =
                any.downcast_mut::<labwired_core::peripherals::uart::Uart>()
            else {
                continue;
            };
            for stream in &mut uart.attached_streams {
                if let Some(m) = stream.as_any_mut().and_then(|a| a.downcast_mut::<IolinkMaster>()) {
                    m.trace_clear();
                    return;
                }
            }
        }
    }
```

Note: the empty-array fallback above references `IolinkXferUnit` only as a type hint — replace both empty-vec fallbacks with `Vec::<labwired_core::peripherals::components::IolinkXfer>::new()` so the element type is unambiguous. Final form for the two fallback lines:
```rust
        return serde_wasm_bindgen::to_value(
            &Vec::<labwired_core::peripherals::components::IolinkXfer>::new(),
        ).unwrap_or(JsValue::NULL);
```
(Use that exact expression in both the no-machine branch and the no-master tail.)

Verify the mutable accessor path exists: `Uart::as_any_mut` and `UartStreamDevice::as_any_mut` are already implemented (the master implements `as_any_mut` per its `UartStreamDevice` impl). If `machine.bus.peripherals` is not iterable mutably or `p.dev.as_any_mut()` is unavailable, drop `iolink_trace_clear` from this task and implement Clear UI-side as a "hide older than now" filter instead — but first confirm by grep:
```bash
grep -n "fn as_any_mut" crates/core/src/peripherals/uart.rs crates/core/src/peripherals/components/iolink_master.rs
```

- [ ] **Step 2: Build the wasm crate (typecheck only, native target)**

Run (inside the core worktree):
```bash
cargo build -p labwired-wasm --target wasm32-unknown-unknown 2>&1 | tail -15 || cargo check -p labwired-wasm 2>&1 | tail -15
```
Expected: compiles. Fix any borrow/lookup errors against the verbatim `get_iolink_master_state` pattern (immutable) and `as_any_mut` for the clear path.

- [ ] **Step 3: Commit**

```bash
git add crates/wasm/src/lib.rs
git commit -m "feat(wasm): iolink_trace_snapshot + iolink_trace_clear exports"
```

(The actual `wasm-pack` artifact rebuild + commit-into-playground happens in Task 7, after the UI consumes it.)

---

## Task 4: Bridge — typed `iolinkTraceSnapshot()`

**Files:**
- Modify: `packages/ui/src/wasm/simulator-bridge.ts`

- [ ] **Step 1: Add the `IolinkXfer` interface**

Add near the existing `IolinkMasterState` interface (~`simulator-bridge.ts:82`):

```ts
/** One captured IO-Link master↔device transaction (see core IolinkXfer). */
export interface IolinkXfer {
  seq: number;
  kind: 'wake_up' | 'idle' | 'operate_req' | 'cyclic';
  com: 'com1' | 'com2' | 'com3';
  pd_out: number[];
  pd_in: number[];
  od: number;
  ck_ok: boolean | null;
  pd_valid: boolean | null;
  link_state: 'startup' | 'operate';
  raw_master: number[];
  raw_device: number[];
}
```

- [ ] **Step 2: Declare the raw wasm methods**

In `interface WasmSimulatorInstance` (near `get_iolink_master_state(): IolinkMasterState | null;`), add:

```ts
  iolink_trace_snapshot(): unknown;
  iolink_trace_clear(): void;
```

- [ ] **Step 3: Add the bridge methods**

Add near `getIolinkMasterState()` (~`simulator-bridge.ts:509`). The Rust side serializes a tagged `Vec<struct>`, which `serde_wasm_bindgen` returns as plain objects (like `air_trace_snapshot`), so `asPlainObject` is applied defensively:

```ts
  /** Snapshot of the IO-Link master's captured transactions (oldest→newest). */
  iolinkTraceSnapshot(): IolinkXfer[] {
    const raw = this.sim.iolink_trace_snapshot() as unknown[] | null;
    return (raw ?? []).map((x) => asPlainObject<IolinkXfer>(x));
  }

  /** Clear the IO-Link master's trace ring. */
  iolinkTraceClear(): void {
    this.sim.iolink_trace_clear();
  }
```

- [ ] **Step 4: Typecheck the ui package**

Run (inside the parent worktree):
```bash
npm run -w packages/ui typecheck 2>&1 | tail -15 || npx -w packages/ui tsc --noEmit 2>&1 | tail -15
```
Expected: no type errors. (The methods call real wasm exports that exist once Task 7 rebuilds the engine; typecheck only needs the `WasmSimulatorInstance` decls from Step 2.)

- [ ] **Step 5: Commit**

```bash
cd <parent-worktree>
git add packages/ui/src/wasm/simulator-bridge.ts
git commit -m "feat(ui): iolinkTraceSnapshot/iolinkTraceClear bridge methods"
```

---

## Task 5: Decoder/format module + tests

**Files:**
- Create: `packages/playground/src/instruments/iolinkDecode.ts`
- Create: `packages/playground/src/instruments/iolinkDecode.test.ts`

Pure presentation helpers (no protocol logic — decode already happened in Rust). TDD with vitest.

- [ ] **Step 1: Write the failing test**

Create `packages/playground/src/instruments/iolinkDecode.test.ts`:

```ts
import { describe, it, expect } from 'vitest';
import {
  toHex,
  kindLabel,
  linkPhaseIndex,
  PHASES,
  errorCount,
  filterErrorsOnly,
  toCsv,
} from './iolinkDecode';
import type { IolinkXfer } from '@labwired/ui';

function xfer(over: Partial<IolinkXfer>): IolinkXfer {
  return {
    seq: 0,
    kind: 'cyclic',
    com: 'com2',
    pd_out: [],
    pd_in: [0xa5],
    od: 0,
    ck_ok: true,
    pd_valid: true,
    link_state: 'operate',
    raw_master: [0x00, 0x00, 0x00, 0x1b],
    raw_device: [0x20, 0xa5, 0x00, 0x0d],
    ...over,
  };
}

describe('iolinkDecode', () => {
  it('formats bytes as spaced uppercase hex, with a dash for empty', () => {
    expect(toHex([0x0a, 0xff, 0x00])).toBe('0A FF 00');
    expect(toHex([])).toBe('—');
  });

  it('labels frame kinds', () => {
    expect(kindLabel('wake_up')).toBe('WAKE-UP');
    expect(kindLabel('idle')).toBe('IDLE');
    expect(kindLabel('operate_req')).toBe('OPERATE');
    expect(kindLabel('cyclic')).toBe('CYCLIC');
  });

  it('maps link state to a phase-strip index', () => {
    expect(PHASES).toEqual(['WAKE-UP', 'STARTUP', 'PREOPERATE', 'OPERATE']);
    expect(linkPhaseIndex('startup')).toBe(1);
    expect(linkPhaseIndex('operate')).toBe(3);
  });

  it('counts only frames with a false CRC (null verdicts are not errors)', () => {
    const rows = [
      xfer({ ck_ok: true }),
      xfer({ ck_ok: false }),
      xfer({ ck_ok: null, kind: 'wake_up' }),
    ];
    expect(errorCount(rows)).toBe(1);
    expect(filterErrorsOnly(rows)).toHaveLength(1);
    expect(filterErrorsOnly(rows)[0].ck_ok).toBe(false);
  });

  it('exports CSV with a header and one row per xfer', () => {
    const csv = toCsv([xfer({ seq: 3 })]);
    const lines = csv.trim().split('\n');
    expect(lines[0]).toBe('seq,kind,link_state,pd_out,pd_in,ck_ok,raw_master,raw_device');
    expect(lines[1]).toContain('3,cyclic,operate');
    expect(lines[1]).toContain('A5'); // pd_in hex
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run (inside parent worktree):
```bash
npx vitest run src/instruments/iolinkDecode.test.ts --config vitest.config.ts --root packages/playground 2>&1 | tail -15
```
Expected: FAIL — cannot resolve `./iolinkDecode`.

- [ ] **Step 3: Write the module**

Create `packages/playground/src/instruments/iolinkDecode.ts`:

```ts
// Pure presentation helpers for the IO-Link Analyzer. The protocol decode
// (CRC6, M-sequence framing, PD/OD extraction) already happens in the Rust
// master and arrives as IolinkXfer records; these are display formatters only,
// kept pure so they unit-test in isolation and can back a CLI exporter later.
import type { IolinkXfer } from '@labwired/ui';

/** The IO-Link startup→operate phases, in order, for the phase strip. */
export const PHASES = ['WAKE-UP', 'STARTUP', 'PREOPERATE', 'OPERATE'] as const;

/** Format a byte array as spaced uppercase hex; '—' when empty. */
export function toHex(bytes: number[]): string {
  if (!bytes || bytes.length === 0) return '—';
  return bytes.map((b) => (b & 0xff).toString(16).padStart(2, '0').toUpperCase()).join(' ');
}

/** Short label for a frame kind. */
export function kindLabel(kind: IolinkXfer['kind']): string {
  switch (kind) {
    case 'wake_up':
      return 'WAKE-UP';
    case 'idle':
      return 'IDLE';
    case 'operate_req':
      return 'OPERATE';
    case 'cyclic':
      return 'CYCLIC';
    default:
      return kind;
  }
}

/**
 * Map the latest record's link_state to a PHASES index for the phase strip.
 * The core master models a binary startup→operate machine; we present the
 * canonical IO-Link phases, marking everything up to the current one as done.
 */
export function linkPhaseIndex(linkState: IolinkXfer['link_state']): number {
  return linkState === 'operate' ? 3 : 1;
}

/** A frame is an error only if it has an explicit failed CRC (null = no verdict). */
export function isError(x: IolinkXfer): boolean {
  return x.ck_ok === false;
}

/** Count of frames with a failed CRC. */
export function errorCount(rows: IolinkXfer[]): number {
  return rows.reduce((n, x) => (isError(x) ? n + 1 : n), 0);
}

/** Keep only failed-CRC frames. */
export function filterErrorsOnly(rows: IolinkXfer[]): IolinkXfer[] {
  return rows.filter(isError);
}

/** A CK cell value: 'ok' | 'bad' | 'na'. */
export function ckState(x: IolinkXfer): 'ok' | 'bad' | 'na' {
  if (x.ck_ok === null) return 'na';
  return x.ck_ok ? 'ok' : 'bad';
}

/** Serialize a capture to CSV for the Copy button. */
export function toCsv(rows: IolinkXfer[]): string {
  const header = 'seq,kind,link_state,pd_out,pd_in,ck_ok,raw_master,raw_device';
  const body = rows
    .map((x) =>
      [
        x.seq,
        x.kind,
        x.link_state,
        toHex(x.pd_out).replace(/ /g, ''),
        toHex(x.pd_in).replace(/ /g, ''),
        x.ck_ok === null ? '' : x.ck_ok,
        toHex(x.raw_master).replace(/ /g, ''),
        toHex(x.raw_device).replace(/ /g, ''),
      ].join(','),
    )
    .join('\n');
  return `${header}\n${body}\n`;
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run:
```bash
npx vitest run src/instruments/iolinkDecode.test.ts --config vitest.config.ts --root packages/playground 2>&1 | tail -15
```
Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
git add packages/playground/src/instruments/iolinkDecode.ts packages/playground/src/instruments/iolinkDecode.test.ts
git commit -m "feat(playground): iolinkDecode formatters + tests"
```

---

## Task 6: Instrument UI + Tools-menu registration

**Files:**
- Create: `packages/playground/src/instruments/IoLinkAnalyzer.tsx`
- Modify: `packages/playground/src/App.tsx`

- [ ] **Step 1: Write the analyzer component**

Create `packages/playground/src/instruments/IoLinkAnalyzer.tsx` (cloned structure from `BleAnalyzer.tsx`; adds the phase strip, toolbar with Copy + errors-only filter, and expandable raw rows):

```tsx
// IO-Link Analyzer — taps the simulated IO-Link master and renders the live
// master↔device protocol as per-cycle transaction rows. Polls the same way as
// the Air Tracer (BleAnalyzer): a useRef'd bridge + interval while running. The
// protocol decode already happened in Rust (IolinkXfer); this only formats.
import { useEffect, useMemo, useRef, useState } from 'react';
import type { IolinkXfer, SimulatorBridge } from '@labwired/ui';
import {
  PHASES,
  ckState,
  errorCount,
  filterErrorsOnly,
  kindLabel,
  linkPhaseIndex,
  toCsv,
  toHex,
} from './iolinkDecode';

export interface IoLinkAnalyzerProps {
  bridge: SimulatorBridge | null;
  running: boolean;
  pollMs?: number;
}

export function IoLinkAnalyzer({ bridge, running, pollMs = 200 }: IoLinkAnalyzerProps) {
  const [rows, setRows] = useState<IolinkXfer[]>([]);
  const [errorsOnly, setErrorsOnly] = useState(false);
  const [expanded, setExpanded] = useState<number | null>(null);
  const bridgeRef = useRef(bridge);
  bridgeRef.current = bridge;

  useEffect(() => {
    let cancelled = false;
    const poll = () => {
      const b = bridgeRef.current;
      if (!b) return;
      try {
        const trace = b.iolinkTraceSnapshot();
        if (!cancelled) setRows(trace);
      } catch {
        /* bridge may be mid-teardown between Run/Stop; ignore one tick */
      }
    };
    poll();
    if (!running) return;
    const id = window.setInterval(poll, pollMs);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, [running, pollMs, bridge]);

  const phaseIdx = rows.length ? linkPhaseIndex(rows[rows.length - 1].link_state) : -1;
  const errs = errorCount(rows);
  const com = rows.length ? rows[rows.length - 1].com.toUpperCase() : '—';
  const view = useMemo(
    () => (errorsOnly ? filterErrorsOnly(rows) : rows),
    [rows, errorsOnly],
  );

  const copy = () => {
    try {
      void navigator.clipboard?.writeText(toCsv(rows));
    } catch {
      /* clipboard unavailable; no-op */
    }
  };

  return (
    <div className="flex flex-col h-full min-h-0 text-fg-primary text-[12px]">
      {/* Phase strip */}
      <div className="flex gap-1 px-3 py-2 border-b border-border">
        {PHASES.map((p, i) => (
          <div
            key={p}
            className={`flex-1 text-center rounded px-1 py-0.5 font-mono text-[10px] ${
              phaseIdx < 0
                ? 'bg-bg-canvas text-fg-tertiary'
                : i < phaseIdx
                  ? 'bg-green-900/40 text-green-400'
                  : i === phaseIdx
                    ? 'bg-green-500 text-black font-bold'
                    : 'bg-bg-canvas text-fg-tertiary'
            }`}
          >
            {p}
          </div>
        ))}
      </div>

      {/* Toolbar */}
      <div className="flex items-center justify-between px-3 py-1.5 border-b border-border font-mono text-[11px]">
        <span className="text-fg-tertiary">
          {rows.length} frame{rows.length === 1 ? '' : 's'} · {com}
          {errs > 0 && <span className="text-red-500"> · {errs} CRC err</span>}
        </span>
        <span className="flex gap-2">
          <button
            type="button"
            className={`px-2 py-0.5 rounded border border-border ${errorsOnly ? 'text-red-500' : 'text-fg-secondary'}`}
            onClick={() => setErrorsOnly((v) => !v)}
          >
            errors only
          </button>
          <button
            type="button"
            className="px-2 py-0.5 rounded border border-border text-fg-secondary"
            onClick={copy}
          >
            ⧉ Copy
          </button>
        </span>
      </div>

      {view.length === 0 ? (
        <div className="flex-1 flex items-center justify-center px-4 text-center text-fg-tertiary text-[12px]">
          {running
            ? 'Waiting for IO-Link traffic… ensure an IO-Link master is wired and running.'
            : 'No transactions yet. Add an IO-Link master and press Run.'}
        </div>
      ) : (
        <div className="flex-1 min-h-0 overflow-auto">
          <table className="w-full border-collapse font-mono text-[11px]">
            <thead className="sticky top-0 bg-bg-base text-fg-secondary">
              <tr className="text-left">
                <th className="px-3 py-1.5 font-medium">#</th>
                <th className="px-3 py-1.5 font-medium">Type</th>
                <th className="px-3 py-1.5 font-medium">PD out</th>
                <th className="px-3 py-1.5 font-medium">PD in</th>
                <th className="px-3 py-1.5 font-medium">CK</th>
                <th className="px-3 py-1.5 font-medium">Link</th>
              </tr>
            </thead>
            <tbody>
              {view
                .slice()
                .reverse()
                .map((r) => {
                  const ck = ckState(r);
                  const isExpanded = expanded === r.seq;
                  return (
                    <FragmentRow
                      key={r.seq}
                      r={r}
                      ck={ck}
                      expanded={isExpanded}
                      onToggle={() => setExpanded(isExpanded ? null : r.seq)}
                    />
                  );
                })}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}

function FragmentRow({
  r,
  ck,
  expanded,
  onToggle,
}: {
  r: IolinkXfer;
  ck: 'ok' | 'bad' | 'na';
  expanded: boolean;
  onToggle: () => void;
}) {
  return (
    <>
      <tr
        className={`border-t border-border/60 hover:bg-bg-canvas cursor-pointer ${ck === 'bad' ? 'bg-red-500/10' : ''}`}
        onClick={onToggle}
      >
        <td className="px-3 py-1 text-fg-tertiary">{r.seq}</td>
        <td className="px-3 py-1">{kindLabel(r.kind)}</td>
        <td className="px-3 py-1 text-fg-secondary">{toHex(r.pd_out)}</td>
        <td className="px-3 py-1 text-fg-primary font-semibold">{toHex(r.pd_in)}</td>
        <td className="px-3 py-1 font-semibold">
          {ck === 'na' ? (
            <span className="text-fg-tertiary">—</span>
          ) : ck === 'ok' ? (
            <span className="text-green-500">✓</span>
          ) : (
            <span className="text-red-500">✗</span>
          )}
        </td>
        <td className="px-3 py-1">{r.link_state === 'operate' ? 'OPERATE' : 'STARTUP'}</td>
      </tr>
      {expanded && (
        <tr className="bg-bg-canvas/60">
          <td colSpan={6} className="px-3 py-1.5 text-fg-secondary">
            <div>M→D: {toHex(r.raw_master)}</div>
            <div>D→M: {toHex(r.raw_device)}</div>
          </td>
        </tr>
      )}
    </>
  );
}
```

- [ ] **Step 2: Register the tool + window in `App.tsx`**

Add the import near the `BleAnalyzer` import (~`App.tsx:56`):
```tsx
import { IoLinkAnalyzer } from './instruments/IoLinkAnalyzer';
```

Add a state flag near `showAnalyzer` (~`App.tsx:623`):
```tsx
  const [showIolink, setShowIolink] = useState(false);
```

Add a second entry to the `ToolsMenu` `tools` array (~`App.tsx:2178`, after the `air-tracer` object):
```tsx
                {
                  id: 'iolink-analyzer',
                  label: 'IO-Link Analyzer',
                  description: 'Master↔device frames, CRC, link state',
                  active: showIolink,
                  onToggle: () => setShowIolink((v) => !v),
                },
```

Add a `ChipWindow` mount next to the Air Tracer one (~`App.tsx:2543`, after the existing `{!isMobile && showAnalyzer && (...)}` block):
```tsx
    {!isMobile && showIolink && (
      <ChipWindow
        initial={{ x: 900, y: 120 }}
        width={540}
        height={360}
        zIndex={95}
        onClose={() => setShowIolink(false)}
        title={
          <span className="truncate text-xs font-semibold text-fg-primary">
            IO-Link Analyzer · master↔device
          </span>
        }
      >
        <IoLinkAnalyzer bridge={bridge} running={running} />
      </ChipWindow>
    )}
```

- [ ] **Step 3: Typecheck + build the playground**

Run (inside parent worktree):
```bash
npm run -w packages/playground build 2>&1 | tail -20
```
Expected: type-checks and builds. Fix any prop/type mismatches against the `IolinkXfer` interface and the `bridge`/`running` variables already in scope at the Air Tracer mount.

- [ ] **Step 4: Commit**

```bash
git add packages/playground/src/instruments/IoLinkAnalyzer.tsx packages/playground/src/App.tsx
git commit -m "feat(playground): IO-Link Analyzer instrument in Tools menu"
```

---

## Task 7: Rebuild wasm + verify end-to-end

**Files:**
- Modify: `packages/playground/public/wasm/labwired_wasm_bg.wasm`, `packages/playground/public/wasm/labwired_wasm.js`

The playground ships a prebuilt wasm engine; it must be rebuilt to carry `iolink_trace_snapshot`. (See the stale-wasm playbook: the deploy does NOT rebuild it.)

- [ ] **Step 1: Rebuild the engine from the core worktree's wasm crate**

Run (the core worktree holds the Task 1–3 changes):
```bash
cd <core-worktree>/crates/wasm
wasm-pack build --release --target web --out-dir <parent-worktree>/packages/playground/public/wasm
```
Expected: emits `labwired_wasm_bg.wasm` + `labwired_wasm.js` into the playground. `wasm-pack` may rewrite `package.json`/`.gitignore` in the out-dir with trailing-newline noise — revert those two files; stage only the `.wasm` + `.js`.

- [ ] **Step 2: Confirm the new export is present**

Run:
```bash
grep -a -c "iolink_trace_snapshot" <parent-worktree>/packages/playground/public/wasm/labwired_wasm.js
```
Expected: ≥ 1.

- [ ] **Step 3: Run the full instrument test suites**

Run (inside parent worktree):
```bash
npx vitest run src/instruments/ --config vitest.config.ts --root packages/playground 2>&1 | tail -20
```
Expected: PASS (iolinkDecode + any existing instrument tests).

- [ ] **Step 4: Headless data-path check (browser backend may be unavailable)**

Serve the playground and confirm the bridge method returns frames once a sim with an IO-Link master runs. If a browser is available, open the playground, load the IO-Link DI example, press Run, open Tools → IO-Link Analyzer, and confirm: the phase strip advances to OPERATE, CYCLIC rows accumulate with `PD in` populated and `CK ✓`, a row expands to show `M→D`/`D→M` hex, and Copy writes CSV.

If no browser backend is available, verify headlessly: from the parent worktree, load the rebuilt `labwired_wasm.js` in a Node harness (or reuse an existing wasm-bridge test in `packages/ui/src/wasm/simulator-bridge.test.ts`) that instantiates the engine with the IO-Link DI manifest, steps it past startup, and asserts `iolink_trace_snapshot()` returns cyclic xfers with non-empty `pd_in`. Document which path was used.

- [ ] **Step 5: Commit the rebuilt engine**

```bash
cd <parent-worktree>
git add packages/playground/public/wasm/labwired_wasm_bg.wasm packages/playground/public/wasm/labwired_wasm.js
git commit -m "build(playground): rebuild wasm engine with iolink_trace_snapshot"
```

---

## Self-Review

**Spec coverage:**
- Unit 1 (core trace ring) → Tasks 1–2 ✓ (`IolinkXfer`, ring, `trace_snapshot`/`trace_clear`).
- Unit 2 (wasm bridge) → Task 3 (wasm export) + Task 4 (TS `iolinkTraceSnapshot`/`iolinkTraceClear`) ✓.
- Unit 3 (decode/format) → Task 5 (`iolinkDecode.ts` + tests) ✓.
- Unit 4 (instrument UI) → Task 6 (`IoLinkAnalyzer.tsx`, ToolsMenu + ChipWindow) ✓.
- Phase strip ✓ (Task 6, `linkPhaseIndex`/`PHASES`). Error highlighting ✓ (`ckState`/`isError`, row bg). Copy-capture ✓ (`toCsv`). Expandable raw frames ✓ (`FragmentRow`). Empty/error states ✓. Ring cap 256 ✓. No fault injection ✓ (not present). Dependency/branch base documented ✓.

**Placeholder scan:** No TBD/TODO; every code step shows complete code. The one conditional ("if `as_any_mut` unavailable, drop clear") is a guarded fallback with an exact grep check, not a placeholder.

**Type/name consistency:** `IolinkXfer` fields identical across Rust (`seq,kind,com,pd_out,pd_in,od,ck_ok,pd_valid,link_state,raw_master,raw_device`), the TS interface, the decoder, and the UI. serde `rename_all="snake_case"` makes Rust enums serialize to the exact TS string unions (`wake_up`/`idle`/`operate_req`/`cyclic`, `com1..3`, `startup`/`operate`). `iolink_trace_snapshot`/`iolink_trace_clear` (wasm) ↔ `iolinkTraceSnapshot`/`iolinkTraceClear` (bridge) consistent. `PHASES`/`linkPhaseIndex`/`ckState`/`errorCount`/`filterErrorsOnly`/`toCsv`/`toHex`/`kindLabel` used identically in module, tests, and UI.
