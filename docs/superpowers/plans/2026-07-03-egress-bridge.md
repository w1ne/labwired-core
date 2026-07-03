# Egress Bridge Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stream a simulated peripheral's output (UART TX bytes, CAN frames) out of the deterministic sim to a real MQTT/TCP/HTTP backend, declared in the environment manifest.

**Architecture:** The deterministic sim core only *enqueues* items over an in-process channel; it never does real network I/O. An `EgressBus` (a `network::Interconnect`, ticked for free by `World::step_all`) drains those channels each step, applies an encoding, and hands payloads to an `EgressTransport` worker thread that owns the blocking socket. A bounded drop-oldest buffer means a slow/dead endpoint never stalls `Machine::run`, preserving replay determinism.

**Tech Stack:** Rust, `std::sync::mpsc` (channels), `std::thread` (transport worker), `serde_json` (encodings), `std::net::TcpStream` (TCP), existing `network/mqtt.rs` packet helpers (MQTT). No new crate dependencies beyond `serde_json` (already in the tree).

## Global Constraints

- Language/toolchain: Rust; validate formatting in the CI Docker image (clang-format is C-only; Rust uses `cargo fmt`). Run `cargo fmt --all` before each commit.
- **The sim thread must never block on network I/O.** All blocking writes happen on the transport worker thread. `EgressBus::tick` and `EgressTap::on_tx_byte` must only do non-blocking channel operations.
- No new third-party crate dependencies. `serde`, `serde_json`, `serde_yaml`, `anyhow` are already available.
- Source-file license header: every new `.rs` file starts with the 5-line MIT header used across the crate (copy verbatim from `crates/core/src/network/mod.rs:1-5`).
- Base branch: `feat/egress-bridge` off `origin/main`. Work in worktree `~/projects/labwired-egress`.
- Determinism: identical firmware + identical stimuli must produce identical *enqueued* item sequences regardless of transport behavior. Tests assert on enqueued items, not on wire timing.
- Test commands run from `~/projects/labwired-egress`: `cargo test -p labwired-core <name>`. Net-backed integration tests are gated behind the `net-tests` cargo feature and are NOT part of the default gate.

## Scope of THIS plan

- **In:** UART TX egress fully wired through the manifest; `EgressBus` + `EgressTap` + encodings + `MemoryTransport`/`TcpSink`/`MqttPublisher`/`HttpPoster`; CAN-frame egress supported and unit-tested at the `EgressBus` layer.
- **Deferred (documented, not built here):** manifest wiring for CAN sources (needs a named `CanBus` lifecycle in `from_manifest`, out of scope); browser/WASM egress via WS relay; ingress/bidirectional; the E2E demo lab + blog post (tracked as a follow-up task at the end).

## File Structure

- Create `crates/core/src/network/egress/mod.rs` — module root; `EgressItem`, `EncodingKind`, `BufferPolicy`, re-exports.
- Create `crates/core/src/network/egress/encoding.rs` — pure encode functions (`raw`, `ndjson-trace`, `frames-json`).
- Create `crates/core/src/network/egress/tap.rs` — `EgressTap` (`UartStreamDevice`).
- Create `crates/core/src/network/egress/bus.rs` — `EgressBus` (`Interconnect`) + worker-thread plumbing.
- Create `crates/core/src/network/egress/transport/mod.rs` — `EgressTransport` trait + `MemoryTransport`.
- Create `crates/core/src/network/egress/transport/tcp.rs` — `TcpSink`.
- Create `crates/core/src/network/egress/transport/mqtt.rs` — `MqttPublisher`.
- Create `crates/core/src/network/egress/transport/http.rs` — `HttpPoster`.
- Modify `crates/core/src/network/mod.rs` — add `pub mod egress;`.
- Modify `crates/core/src/world.rs` — add `"egress"` arm to the `from_manifest` interconnect match (UART source).
- Modify `crates/core/Cargo.toml` — add the `net-tests` feature.

---

### Task 1: Egress module scaffolding — `EgressItem`, `EncodingKind`, `EgressTransport` trait, `MemoryTransport`

**Files:**
- Create: `crates/core/src/network/egress/mod.rs`
- Create: `crates/core/src/network/egress/transport/mod.rs`
- Modify: `crates/core/src/network/mod.rs` (add `pub mod egress;` after line 12)

**Interfaces:**
- Produces:
  - `enum EgressItem { Byte(u8), Frame(crate::network::CanFrame) }` (derives `Debug, Clone, PartialEq`)
  - `enum EncodingKind { Raw, NdjsonTrace, FramesJson }` (derives `Debug, Clone, Copy, PartialEq`)
  - `struct BufferPolicy { pub max: usize }` with `Default` → `max: 4096`
  - `trait EgressTransport: Send { fn send(&mut self, payload: &[u8]) -> anyhow::Result<()>; }`
  - `struct MemoryTransport { sink: std::sync::Arc<std::sync::Mutex<Vec<Vec<u8>>>> }` with `MemoryTransport::new() -> Self` and `fn handle(&self) -> std::sync::Arc<std::sync::Mutex<Vec<Vec<u8>>>>` (clone of the Arc so tests can inspect what the worker received).

- [ ] **Step 1: Write the failing test**

In `crates/core/src/network/egress/transport/mod.rs`, at the bottom:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_transport_records_payloads() {
        let mut t = MemoryTransport::new();
        let handle = t.handle();
        t.send(b"hello").unwrap();
        t.send(b"world").unwrap();
        let got = handle.lock().unwrap();
        assert_eq!(&*got, &[b"hello".to_vec(), b"world".to_vec()]);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p labwired-core memory_transport_records_payloads`
Expected: FAIL — module `egress` does not exist / `MemoryTransport` not found.

- [ ] **Step 3: Write minimal implementation**

`crates/core/src/network/mod.rs` — add after line 12 (`pub mod virtual_uart_wire;`):

```rust
pub mod egress;
```

`crates/core/src/network/egress/mod.rs`:

```rust
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Egress bridge: stream simulated peripheral output out to a real backend.
//!
//! The deterministic sim core only enqueues [`EgressItem`]s; a worker thread
//! owned by the transport performs the blocking network write. See
//! `docs/superpowers/specs/2026-07-03-egress-bridge-design.md`.

pub mod encoding;
pub mod transport;

use crate::network::CanFrame;

/// One unit of output captured from a simulated peripheral.
#[derive(Debug, Clone, PartialEq)]
pub enum EgressItem {
    /// A single byte transmitted on a UART TX line.
    Byte(u8),
    /// A CAN/CAN-FD frame transmitted by the firmware.
    Frame(CanFrame),
}

/// How buffered [`EgressItem`]s become an on-wire payload.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EncodingKind {
    /// Bytes verbatim; frames as their raw data field.
    Raw,
    /// One JSON object per item, newline-delimited.
    NdjsonTrace,
    /// One JSON object per CAN frame.
    FramesJson,
}

/// Bounded-buffer policy. On overflow the oldest item is dropped.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BufferPolicy {
    pub max: usize,
}

impl Default for BufferPolicy {
    fn default() -> Self {
        Self { max: 4096 }
    }
}
```

`crates/core/src/network/egress/transport/mod.rs`:

```rust
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Egress transports. Each `send` runs on the worker thread and may block.

use std::sync::{Arc, Mutex};

/// A destination for encoded egress payloads. `send` runs on the transport
/// worker thread, so blocking network I/O is allowed here (never on the sim
/// thread).
pub trait EgressTransport: Send {
    fn send(&mut self, payload: &[u8]) -> anyhow::Result<()>;
}

/// In-memory transport for deterministic tests; records every payload.
pub struct MemoryTransport {
    sink: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl MemoryTransport {
    pub fn new() -> Self {
        Self {
            sink: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// A clone of the shared record, so a test can inspect received payloads.
    pub fn handle(&self) -> Arc<Mutex<Vec<Vec<u8>>>> {
        Arc::clone(&self.sink)
    }
}

impl Default for MemoryTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl EgressTransport for MemoryTransport {
    fn send(&mut self, payload: &[u8]) -> anyhow::Result<()> {
        self.sink.lock().unwrap().push(payload.to_vec());
        Ok(())
    }
}
```

Create a placeholder `crates/core/src/network/egress/encoding.rs` so the module compiles:

```rust
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Encoding functions are implemented in Task 3.
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p labwired-core memory_transport_records_payloads`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/core/src/network/mod.rs crates/core/src/network/egress
git commit -m "feat(egress): module scaffolding, EgressTransport trait, MemoryTransport"
```

---

### Task 2: `EgressTap` — a UartStreamDevice that forwards TX bytes

**Files:**
- Create: `crates/core/src/network/egress/tap.rs`
- Modify: `crates/core/src/network/egress/mod.rs` (add `pub mod tap;`)

**Interfaces:**
- Consumes: `EgressItem` (Task 1), `crate::peripherals::uart::UartStreamDevice` (peripherals/uart.rs:28) — `poll(&mut self, u32) -> Option<u8>`, `on_tx_byte(&mut self, u8)`.
- Produces: `struct EgressTap { tx: std::sync::mpsc::Sender<EgressItem> }` and `EgressTap::new(tx: Sender<EgressItem>) -> Self`.

- [ ] **Step 1: Write the failing test**

In `crates/core/src/network/egress/tap.rs`, at the bottom:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::uart::UartStreamDevice;
    use std::sync::mpsc::channel;

    #[test]
    fn tap_forwards_tx_bytes_and_never_emits_rx() {
        let (tx, rx) = channel();
        let mut tap = EgressTap::new(tx);
        // Tap is TX-observe-only: poll must never inject RX bytes.
        assert_eq!(tap.poll(1000), None);
        tap.on_tx_byte(0x41);
        tap.on_tx_byte(0x42);
        assert_eq!(rx.recv().unwrap(), EgressItem::Byte(0x41));
        assert_eq!(rx.recv().unwrap(), EgressItem::Byte(0x42));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p labwired-core tap_forwards_tx_bytes`
Expected: FAIL — `EgressTap` not found.

- [ ] **Step 3: Write minimal implementation**

Add `pub mod tap;` to `crates/core/src/network/egress/mod.rs` (below `pub mod transport;`).

`crates/core/src/network/egress/tap.rs`:

```rust
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! `EgressTap` observes UART TX bytes and forwards them to the egress channel.

use crate::network::egress::EgressItem;
use crate::peripherals::uart::UartStreamDevice;
use std::sync::mpsc::Sender;

/// A UART stream device that only observes TX (never drives RX). Each byte the
/// firmware transmits is forwarded to the egress channel. Sending on an
/// unbounded `mpsc::Sender` never blocks, so the sim thread stays deterministic.
pub struct EgressTap {
    tx: Sender<EgressItem>,
}

impl EgressTap {
    pub fn new(tx: Sender<EgressItem>) -> Self {
        Self { tx }
    }
}

impl UartStreamDevice for EgressTap {
    fn poll(&mut self, _elapsed_us: u32) -> Option<u8> {
        None
    }

    fn on_tx_byte(&mut self, byte: u8) {
        // Ignore send errors: a dropped receiver means egress was torn down.
        let _ = self.tx.send(EgressItem::Byte(byte));
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p labwired-core tap_forwards_tx_bytes`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/core/src/network/egress
git commit -m "feat(egress): EgressTap UartStreamDevice forwards TX bytes"
```

---

### Task 3: Encoding functions (`raw`, `ndjson-trace`, `frames-json`)

**Files:**
- Modify: `crates/core/src/network/egress/encoding.rs` (replace the Task 1 placeholder)

**Interfaces:**
- Consumes: `EgressItem`, `EncodingKind` (Task 1); `CanFrame` (network/mod.rs:19).
- Produces: `pub fn encode(kind: EncodingKind, items: &[EgressItem]) -> Vec<u8>`. Returns one payload for a batch of items. Empty input → empty `Vec`.
  - `Raw`: concatenate `Byte` values; for `Frame`, append its `data` bytes.
  - `NdjsonTrace`: one JSON line per item. `Byte` → `{"kind":"byte","byte":N}`; `Frame` → `{"kind":"frame","id":N,"data":[...],"extended":bool,"fd":bool}`. Trailing `\n` after each line.
  - `FramesJson`: JSON array of frame objects (same shape as the frame line above); `Byte` items are skipped.

- [ ] **Step 1: Write the failing test**

Replace the placeholder body of `encoding.rs` header comment area with the header + tests (implementation added in Step 3). For now add just the test module at the bottom of the file:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::egress::{EgressItemAlias as _, EgressItem};
    use crate::network::CanFrame;
    use crate::network::egress::EncodingKind;

    #[test]
    fn raw_concatenates_bytes() {
        let items = vec![EgressItem::Byte(b'h'), EgressItem::Byte(b'i')];
        assert_eq!(encode(EncodingKind::Raw, &items), b"hi".to_vec());
    }

    #[test]
    fn ndjson_emits_one_line_per_item() {
        let items = vec![EgressItem::Byte(0x41)];
        let out = String::from_utf8(encode(EncodingKind::NdjsonTrace, &items)).unwrap();
        assert_eq!(out, "{\"kind\":\"byte\",\"byte\":65}\n");
    }

    #[test]
    fn frames_json_is_array_and_skips_bytes() {
        let items = vec![
            EgressItem::Byte(0x00),
            EgressItem::Frame(CanFrame::classic(0x123, vec![1, 2])),
        ];
        let out = String::from_utf8(encode(EncodingKind::FramesJson, &items)).unwrap();
        assert_eq!(
            out,
            "[{\"id\":291,\"data\":[1,2],\"extended\":false,\"fd\":false}]"
        );
    }

    #[test]
    fn empty_input_is_empty() {
        assert!(encode(EncodingKind::Raw, &[]).is_empty());
    }
}
```

> Note: delete the bogus `use crate::network::egress::{EgressItemAlias as _, EgressItem};` line — it is only here to force you to write the real import. The correct imports are `use super::*;`, `use crate::network::egress::{EgressItem, EncodingKind};`, and `use crate::network::CanFrame;`. Use exactly those.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p labwired-core network::egress::encoding`
Expected: FAIL — `encode` not defined.

- [ ] **Step 3: Write minimal implementation**

Replace `crates/core/src/network/egress/encoding.rs` with (keep the license header):

```rust
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Encode a batch of [`EgressItem`]s into a single on-wire payload.

use crate::network::egress::{EgressItem, EncodingKind};
use crate::network::CanFrame;

/// Encode `items` into one payload according to `kind`. Empty in → empty out.
pub fn encode(kind: EncodingKind, items: &[EgressItem]) -> Vec<u8> {
    match kind {
        EncodingKind::Raw => {
            let mut out = Vec::new();
            for item in items {
                match item {
                    EgressItem::Byte(b) => out.push(*b),
                    EgressItem::Frame(f) => out.extend_from_slice(&f.data),
                }
            }
            out
        }
        EncodingKind::NdjsonTrace => {
            let mut out = String::new();
            for item in items {
                match item {
                    EgressItem::Byte(b) => {
                        out.push_str(&format!("{{\"kind\":\"byte\",\"byte\":{b}}}\n"));
                    }
                    EgressItem::Frame(f) => {
                        out.push_str(&format!("{{\"kind\":\"frame\",{}}}\n", frame_fields(f)));
                    }
                }
            }
            out.into_bytes()
        }
        EncodingKind::FramesJson => {
            let objs: Vec<String> = items
                .iter()
                .filter_map(|item| match item {
                    EgressItem::Frame(f) => Some(format!("{{{}}}", frame_fields(f))),
                    EgressItem::Byte(_) => None,
                })
                .collect();
            if objs.is_empty() {
                Vec::new()
            } else {
                format!("[{}]", objs.join(",")).into_bytes()
            }
        }
    }
}

/// Shared `id,data,extended,fd` field list for a CAN frame (no braces).
fn frame_fields(f: &CanFrame) -> String {
    let data: Vec<String> = f.data.iter().map(|b| b.to_string()).collect();
    format!(
        "\"id\":{},\"data\":[{}],\"extended\":{},\"fd\":{}",
        f.id,
        data.join(","),
        f.extended,
        f.fd
    )
}
```

Fix the test imports per the Step-1 note (remove the bogus alias line).

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p labwired-core network::egress::encoding`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/core/src/network/egress/encoding.rs
git commit -m "feat(egress): raw / ndjson-trace / frames-json encodings"
```

---

### Task 4: `EgressBus` — drain channels, bounded drop-oldest buffer, worker thread

**Files:**
- Create: `crates/core/src/network/egress/bus.rs`
- Modify: `crates/core/src/network/egress/mod.rs` (add `pub mod bus;`)

**Interfaces:**
- Consumes: `EgressItem`, `EncodingKind`, `BufferPolicy` (Task 1); `encode` (Task 3); `EgressTransport` (Task 1); `Interconnect` trait (network/mod.rs:15) — `fn tick(&mut self) -> SimResult<()>`; `crate::SimResult`.
- Produces:
  - `struct EgressBus`
  - `EgressBus::new(rx: std::sync::mpsc::Receiver<EgressItem>, encoding: EncodingKind, policy: BufferPolicy, transport: Box<dyn EgressTransport>) -> Self` — spawns the worker thread internally.
  - `EgressBus::dropped(&self) -> u64` — count of items dropped by the bounded buffer.
  - `impl Interconnect for EgressBus`.
  - Also `EgressBus::add_can_source(&mut self, rx: std::sync::mpsc::Receiver<CanFrame>)` — register a CAN frame receiver (from `CanBus::attach`) as an additional source.

**Design notes for the implementer:**
- Two channels. (1) `rx: Receiver<EgressItem>` from taps (unbounded std mpsc — tap sends never block). (2) an internal `SyncSender<Vec<u8>>`/`Receiver<Vec<u8>>` bounded pair to the worker thread.
- `tick()`: drain both the `EgressItem` receiver and every CAN receiver into an internal `VecDeque<EgressItem>` buffer. While `buffer.len() > policy.max`, `pop_front()` and increment `dropped`. Then encode the whole drained buffer into one payload and `try_send` it to the worker. If the worker channel is full (`TrySendError::Full`), leave the payload's items to be retried — i.e. re-buffer by NOT clearing on failure. Simplest correct form: encode only when we successfully reserve a slot; see reference impl below. Never call blocking `send`.
- Worker thread: `while let Ok(payload) = worker_rx.recv() { let _ = transport.send(&payload); }`. Exits when the bus is dropped (sender closed).

- [ ] **Step 1: Write the failing test**

In `crates/core/src/network/egress/bus.rs`, at the bottom:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::egress::transport::MemoryTransport;
    use crate::network::egress::{BufferPolicy, EgressItem, EncodingKind};
    use crate::network::Interconnect;
    use std::sync::mpsc::channel;
    use std::time::{Duration, Instant};

    fn wait_for<F: Fn() -> bool>(f: F) {
        let start = Instant::now();
        while !f() && start.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn bus_forwards_encoded_bytes_to_transport() {
        let (tx, rx) = channel();
        let transport = MemoryTransport::new();
        let handle = transport.handle();
        let mut bus = EgressBus::new(
            rx,
            EncodingKind::Raw,
            BufferPolicy::default(),
            Box::new(transport),
        );
        tx.send(EgressItem::Byte(b'h')).unwrap();
        tx.send(EgressItem::Byte(b'i')).unwrap();
        bus.tick().unwrap();
        wait_for(|| !handle.lock().unwrap().is_empty());
        let got = handle.lock().unwrap();
        assert_eq!(got.concat(), b"hi".to_vec());
    }

    #[test]
    fn bounded_buffer_drops_oldest_and_counts() {
        // Never tick, so nothing is consumed: overflow must drop oldest.
        let (tx, rx) = channel();
        let transport = MemoryTransport::new();
        let mut bus = EgressBus::new(
            rx,
            EncodingKind::Raw,
            BufferPolicy { max: 2 },
            Box::new(transport),
        );
        for b in 0..5u8 {
            tx.send(EgressItem::Byte(b)).unwrap();
        }
        bus.tick().unwrap();
        // 5 items in, buffer cap 2 → 3 dropped.
        assert_eq!(bus.dropped(), 3);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p labwired-core network::egress::bus`
Expected: FAIL — `EgressBus` not found.

- [ ] **Step 3: Write minimal implementation**

Add `pub mod bus;` to `crates/core/src/network/egress/mod.rs`.

`crates/core/src/network/egress/bus.rs`:

```rust
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! `EgressBus`: an `Interconnect` that drains tapped items each sim step and
//! hands encoded payloads to a transport worker thread. Enqueue-only on the sim
//! side; all blocking network I/O happens on the worker thread.

use crate::network::egress::encoding::encode;
use crate::network::egress::transport::EgressTransport;
use crate::network::egress::{BufferPolicy, EgressItem, EncodingKind};
use crate::network::{CanFrame, Interconnect};
use crate::SimResult;
use std::collections::VecDeque;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::thread::JoinHandle;

/// Bound on the worker payload queue. Independent of the item BufferPolicy.
const WORKER_QUEUE_DEPTH: usize = 64;

pub struct EgressBus {
    rx: Receiver<EgressItem>,
    can_sources: Vec<Receiver<CanFrame>>,
    buffer: VecDeque<EgressItem>,
    policy: BufferPolicy,
    encoding: EncodingKind,
    dropped: u64,
    worker_tx: Option<SyncSender<Vec<u8>>>,
    worker: Option<JoinHandle<()>>,
}

impl EgressBus {
    pub fn new(
        rx: Receiver<EgressItem>,
        encoding: EncodingKind,
        policy: BufferPolicy,
        mut transport: Box<dyn EgressTransport>,
    ) -> Self {
        let (worker_tx, worker_rx) = sync_channel::<Vec<u8>>(WORKER_QUEUE_DEPTH);
        let worker = std::thread::spawn(move || {
            while let Ok(payload) = worker_rx.recv() {
                let _ = transport.send(&payload);
            }
        });
        Self {
            rx,
            can_sources: Vec::new(),
            buffer: VecDeque::new(),
            policy,
            encoding,
            dropped: 0,
            worker_tx: Some(worker_tx),
            worker: Some(worker),
        }
    }

    /// Register a CAN frame receiver (e.g. from `CanBus::attach`).
    pub fn add_can_source(&mut self, rx: Receiver<CanFrame>) {
        self.can_sources.push(rx);
    }

    /// Items discarded by the bounded buffer since construction.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    fn drain_sources(&mut self) {
        while let Ok(item) = self.rx.try_recv() {
            self.buffer.push_back(item);
        }
        for src in &self.can_sources {
            while let Ok(frame) = src.try_recv() {
                self.buffer.push_back(EgressItem::Frame(frame));
            }
        }
        while self.buffer.len() > self.policy.max {
            self.buffer.pop_front();
            self.dropped += 1;
        }
    }
}

impl Interconnect for EgressBus {
    fn tick(&mut self) -> SimResult<()> {
        self.drain_sources();
        if self.buffer.is_empty() {
            return Ok(());
        }
        let items: Vec<EgressItem> = self.buffer.iter().cloned().collect();
        let payload = encode(self.encoding, &items);
        if payload.is_empty() {
            self.buffer.clear();
            return Ok(());
        }
        if let Some(tx) = &self.worker_tx {
            match tx.try_send(payload) {
                Ok(()) => self.buffer.clear(),
                Err(TrySendError::Full(_)) => { /* keep buffered, retry next tick */ }
                Err(TrySendError::Disconnected(_)) => self.buffer.clear(),
            }
        }
        Ok(())
    }
}

impl Drop for EgressBus {
    fn drop(&mut self) {
        // Close the channel so the worker loop exits, then join.
        self.worker_tx.take();
        if let Some(h) = self.worker.take() {
            let _ = h.join();
        }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p labwired-core network::egress::bus`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/core/src/network/egress
git commit -m "feat(egress): EgressBus interconnect with bounded buffer and worker thread"
```

---

### Task 5: Real transports — `TcpSink`, `MqttPublisher`, `HttpPoster` (net-tests gated)

**Files:**
- Create: `crates/core/src/network/egress/transport/tcp.rs`
- Create: `crates/core/src/network/egress/transport/mqtt.rs`
- Create: `crates/core/src/network/egress/transport/http.rs`
- Modify: `crates/core/src/network/egress/transport/mod.rs` (add the three `pub mod`s + re-exports)
- Modify: `crates/core/Cargo.toml` (add `net-tests` feature)

**Interfaces:**
- Consumes: `EgressTransport` trait (Task 1).
- Produces:
  - `TcpSink::connect(addr: &str) -> anyhow::Result<TcpSink>` (addr like `"127.0.0.1:9000"`); `impl EgressTransport` writes the payload bytes to the stream.
  - `MqttPublisher::connect(host: &str, port: u16, topic: String) -> anyhow::Result<MqttPublisher>`; on connect sends an MQTT 3.1.1 CONNECT and reads CONNACK; each `send` emits a PUBLISH (QoS 0) with `topic` and the payload.
  - `HttpPoster::new(url: String, flush_interval_ms: u64) -> HttpPoster`; each `send` POSTs the payload as the request body to `url`.

**Design notes:**
- Crib the MQTT 3.1.1 CONNECT/PUBLISH byte layout from `crates/core/src/network/mqtt.rs` (the in-sim broker parses these packets — reuse the field layout for encoding).
- `HttpPoster` builds a minimal HTTP/1.1 request by hand over a `TcpStream` (`POST <path> HTTP/1.1\r\nHost: ..\r\nContent-Length: N\r\nConnection: close\r\n\r\n<body>`); no `reqwest` dependency. Parse `url` into host/port/path. `flush_interval_ms` is stored for future batching; MVP posts per `send`.
- All three do blocking I/O — correct, because they run on the worker thread.

- [ ] **Step 1: Write the failing test**

Add to `crates/core/src/network/egress/transport/tcp.rs`:

```rust
#[cfg(all(test, feature = "net-tests"))]
mod tests {
    use super::*;
    use crate::network::egress::transport::EgressTransport;
    use std::io::Read;
    use std::net::TcpListener;

    #[test]
    fn tcp_sink_writes_payload_to_listener() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let handle = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = [0u8; 5];
            sock.read_exact(&mut buf).unwrap();
            buf
        });
        let mut sink = TcpSink::connect(&addr).unwrap();
        sink.send(b"hello").unwrap();
        assert_eq!(&handle.join().unwrap(), b"hello");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p labwired-core --features net-tests tcp_sink_writes_payload`
Expected: FAIL — `TcpSink` not found.

- [ ] **Step 3: Write minimal implementation**

`crates/core/Cargo.toml` — add under `[features]` (create the section if absent):

```toml
[features]
net-tests = []
```

`crates/core/src/network/egress/transport/mod.rs` — add below the existing content:

```rust
pub mod http;
pub mod mqtt;
pub mod tcp;

pub use http::HttpPoster;
pub use mqtt::MqttPublisher;
pub use tcp::TcpSink;
```

`crates/core/src/network/egress/transport/tcp.rs`:

```rust
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Raw-TCP egress transport.

use crate::network::egress::transport::EgressTransport;
use std::io::Write;
use std::net::TcpStream;

pub struct TcpSink {
    stream: TcpStream,
}

impl TcpSink {
    pub fn connect(addr: &str) -> anyhow::Result<Self> {
        let stream = TcpStream::connect(addr)?;
        Ok(Self { stream })
    }
}

impl EgressTransport for TcpSink {
    fn send(&mut self, payload: &[u8]) -> anyhow::Result<()> {
        self.stream.write_all(payload)?;
        self.stream.flush()?;
        Ok(())
    }
}
```

`crates/core/src/network/egress/transport/mqtt.rs`:

```rust
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! MQTT 3.1.1 publish transport (QoS 0). Packet layout mirrors the in-sim
//! broker in `network/mqtt.rs`.

use crate::network::egress::transport::EgressTransport;
use std::io::{Read, Write};
use std::net::TcpStream;

pub struct MqttPublisher {
    stream: TcpStream,
    topic: String,
}

impl MqttPublisher {
    pub fn connect(host: &str, port: u16, topic: String) -> anyhow::Result<Self> {
        let mut stream = TcpStream::connect((host, port))?;
        stream.write_all(&connect_packet("labwired-egress"))?;
        let mut connack = [0u8; 4];
        stream.read_exact(&mut connack)?;
        anyhow::ensure!(connack[0] == 0x20, "unexpected MQTT CONNACK");
        Ok(Self { stream, topic })
    }
}

impl EgressTransport for MqttPublisher {
    fn send(&mut self, payload: &[u8]) -> anyhow::Result<()> {
        self.stream.write_all(&publish_packet(&self.topic, payload))?;
        self.stream.flush()?;
        Ok(())
    }
}

/// MQTT 3.1.1 CONNECT with clean session, keep-alive 60s, no will/auth.
fn connect_packet(client_id: &str) -> Vec<u8> {
    let mut var = Vec::new();
    var.extend_from_slice(&[0x00, 0x04]); // "MQTT" length
    var.extend_from_slice(b"MQTT");
    var.push(0x04); // protocol level 4 (3.1.1)
    var.push(0x02); // connect flags: clean session
    var.extend_from_slice(&[0x00, 0x3C]); // keep-alive 60s
    let id = client_id.as_bytes();
    var.extend_from_slice(&(id.len() as u16).to_be_bytes());
    var.extend_from_slice(id);
    let mut pkt = vec![0x10];
    pkt.extend_from_slice(&remaining_length(var.len()));
    pkt.extend_from_slice(&var);
    pkt
}

/// MQTT PUBLISH, QoS 0 (no packet id).
fn publish_packet(topic: &str, payload: &[u8]) -> Vec<u8> {
    let mut var = Vec::new();
    let t = topic.as_bytes();
    var.extend_from_slice(&(t.len() as u16).to_be_bytes());
    var.extend_from_slice(t);
    var.extend_from_slice(payload);
    let mut pkt = vec![0x30];
    pkt.extend_from_slice(&remaining_length(var.len()));
    pkt.extend_from_slice(&var);
    pkt
}

/// MQTT variable-length "remaining length" encoding.
fn remaining_length(mut len: usize) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let mut byte = (len % 128) as u8;
        len /= 128;
        if len > 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if len == 0 {
            break;
        }
    }
    out
}
```

`crates/core/src/network/egress/transport/http.rs`:

```rust
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Minimal HTTP/1.1 POST egress transport (no external HTTP crate).

use crate::network::egress::transport::EgressTransport;
use std::io::{Read, Write};
use std::net::TcpStream;

pub struct HttpPoster {
    host: String,
    port: u16,
    path: String,
    #[allow(dead_code)]
    flush_interval_ms: u64,
}

impl HttpPoster {
    /// `url` like `http://host:8080/ingest`. Only plain HTTP is supported.
    pub fn new(url: String, flush_interval_ms: u64) -> anyhow::Result<Self> {
        let rest = url
            .strip_prefix("http://")
            .ok_or_else(|| anyhow::anyhow!("only http:// URLs supported: {url}"))?;
        let (authority, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, "/"),
        };
        let (host, port) = match authority.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), p.parse()?),
            None => (authority.to_string(), 80u16),
        };
        Ok(Self {
            host,
            port,
            path: path.to_string(),
            flush_interval_ms,
        })
    }
}

impl EgressTransport for HttpPoster {
    fn send(&mut self, payload: &[u8]) -> anyhow::Result<()> {
        let mut stream = TcpStream::connect((self.host.as_str(), self.port))?;
        let head = format!(
            "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            self.path,
            self.host,
            payload.len()
        );
        stream.write_all(head.as_bytes())?;
        stream.write_all(payload)?;
        stream.flush()?;
        let mut resp = Vec::new();
        stream.read_to_end(&mut resp)?; // drain so the server sees a clean close
        Ok(())
    }
}
```

Add matching `#[cfg(all(test, feature = "net-tests"))]` tests to `mqtt.rs` (publish to a `TcpListener` that reads and asserts the first byte is `0x10` CONNECT, replies CONNACK `[0x20,0x02,0x00,0x00]`, then reads the PUBLISH and asserts byte `0x30`) and to `http.rs` (a `TcpListener` reading the request line and asserting it starts with `POST /ingest`). Keep each test self-contained like the TCP test.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p labwired-core --features net-tests network::egress::transport`
Expected: PASS (tcp + mqtt + http).
Also run the default gate to confirm net tests are excluded: `cargo test -p labwired-core network::egress` → PASS, no net tests run.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/core/src/network/egress/transport crates/core/Cargo.toml
git commit -m "feat(egress): TcpSink, MqttPublisher, HttpPoster transports (net-tests gated)"
```

---

### Task 6: Manifest wiring — `"egress"` interconnect with a UART source

**Files:**
- Modify: `crates/core/src/world.rs` (add an `"egress"` arm to the `from_manifest` match at ~world.rs:171)

**Interfaces:**
- Consumes: `InterconnectConfig { r#type, nodes: Vec<String>, config: HashMap<String, serde_yaml::Value> }` (labwired-config lib.rs:267); `MachineTrait::attach_uart_stream` (world.rs:31); `World::add_interconnect` (world.rs:86); `EgressTap`, `EgressBus`, `EncodingKind`, `BufferPolicy`, transports (Tasks 1–5).
- Produces: manifest support for:

```yaml
interconnects:
  - type: egress
    nodes: [sensor_node]        # exactly one node id
    config:
      uart: usart2              # UART peripheral id on that node
      transport: mqtt           # tcp | mqtt | http
      url: "mqtt://broker.acme.io:1883"   # tcp: "host:port"; http: "http://host:port/path"
      topic: "plant/line3/sensor"          # mqtt only
      encoding: raw             # raw | ndjson-trace | frames-json
      buffer_max: 4096          # optional, default 4096
```

- [ ] **Step 1: Write the failing test**

Add to `crates/core/src/world.rs` (bottom, in a `#[cfg(test)] mod egress_manifest_tests`). This is a config-parse test that does not require firmware — it drives a small helper that maps an `InterconnectConfig` to an `EgressBus`, so factor the arm's body into a testable function `build_egress(cfg: &InterconnectConfig) -> anyhow::Result<(String, String, EgressBus)>` returning `(node_id, uart_id, bus)`:

```rust
#[cfg(test)]
mod egress_manifest_tests {
    use super::*;
    use labwired_config::InterconnectConfig;
    use std::collections::HashMap;

    fn cfg(pairs: &[(&str, &str)]) -> InterconnectConfig {
        let mut config = HashMap::new();
        for (k, v) in pairs {
            config.insert(k.to_string(), serde_yaml::Value::String(v.to_string()));
        }
        InterconnectConfig {
            r#type: "egress".to_string(),
            nodes: vec!["sensor_node".to_string()],
            config,
        }
    }

    #[test]
    fn parses_tcp_egress_config() {
        let c = cfg(&[
            ("uart", "usart2"),
            ("transport", "tcp"),
            ("url", "127.0.0.1:9"),
            ("encoding", "raw"),
        ]);
        let (node, uart, _bus) = build_egress(&c).unwrap();
        assert_eq!(node, "sensor_node");
        assert_eq!(uart, "usart2");
    }

    #[test]
    fn rejects_unknown_transport() {
        let c = cfg(&[("uart", "usart2"), ("transport", "carrier-pigeon"), ("url", "x")]);
        assert!(build_egress(&c).is_err());
    }
}
```

> `build_egress` must build the tap channel and the `EgressBus` but does NOT need a live network peer for `tcp` (connect is lazy only if you make it so). To keep the parse test hermetic, construct the transport but tolerate connect failure by using `MemoryTransport` when `cfg` has `transport: memory` — however, do NOT add a public "memory" transport to the manifest surface. Instead: in `build_egress`, for `tcp`/`http` use a lazy connect (store addr, connect on first `send`) so construction never touches the network. Implement `TcpSink`/`HttpPoster` connect lazily OR wrap: simplest is to have `build_egress` accept the transport already built by a small `make_transport(kind, cfg)` and unit-test `make_transport` separately. Choose the lazy-connect refactor: change `TcpSink` to store `addr: String` and connect inside `send` on first use. Update Task 5's `TcpSink` accordingly if not already lazy.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p labwired-core egress_manifest`
Expected: FAIL — `build_egress` not defined.

- [ ] **Step 3: Write minimal implementation**

In `world.rs`, add the helper (top-level `fn`, not inside `impl`) and the match arm. Add the arm before the `other =>` bail at world.rs:199:

```rust
"egress" => {
    let (node, uart, bus) = build_egress(ic)?;
    world
        .machines
        .get_mut(&node)
        .with_context(|| format!("egress: unknown node '{node}'"))?
        .attach_uart_stream(&uart, Box::new(crate::network::egress::tap::EgressTap::new(
            world_egress_sender(&bus),
        )))?;
    world.add_interconnect(Box::new(bus));
}
```

> The tap needs the `Sender<EgressItem>` whose `Receiver` the bus owns. Cleanest: `build_egress` creates the channel, keeps the `Receiver` in the `EgressBus`, and returns the `Sender` too. Change the helper signature to return `(String, String, Sender<EgressItem>, EgressBus)` and drop `world_egress_sender`. Update the test's destructuring to `let (node, uart, _tx, _bus) = ...`.

Helper:

```rust
fn build_egress(
    ic: &labwired_config::InterconnectConfig,
) -> anyhow::Result<(
    String,
    String,
    std::sync::mpsc::Sender<crate::network::egress::EgressItem>,
    crate::network::egress::bus::EgressBus,
)> {
    use crate::network::egress::transport::{HttpPoster, MqttPublisher, TcpSink};
    use crate::network::egress::{bus::EgressBus, BufferPolicy, EncodingKind, EgressItem};
    use anyhow::Context;

    let node = ic.nodes.first().context("egress needs exactly one node")?.clone();
    let get = |k: &str| ic.config.get(k).and_then(|v| v.as_str());
    let uart = get("uart").unwrap_or("usart2").to_string();
    let encoding = match get("encoding").unwrap_or("raw") {
        "raw" => EncodingKind::Raw,
        "ndjson-trace" => EncodingKind::NdjsonTrace,
        "frames-json" => EncodingKind::FramesJson,
        other => anyhow::bail!("egress: unknown encoding '{other}'"),
    };
    let url = get("url").context("egress: missing 'url'")?.to_string();
    let transport: Box<dyn crate::network::egress::transport::EgressTransport> =
        match get("transport").unwrap_or("tcp") {
            "tcp" => Box::new(TcpSink::new(url)),
            "mqtt" => {
                let (host, port) = parse_mqtt_url(&url)?;
                let topic = get("topic").context("egress: mqtt needs 'topic'")?.to_string();
                Box::new(MqttPublisher::lazy(host, port, topic))
            }
            "http" => Box::new(HttpPoster::new(url, 0)?),
            other => anyhow::bail!("egress: unknown transport '{other}'"),
        };
    let policy = BufferPolicy {
        max: ic
            .config
            .get("buffer_max")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(BufferPolicy::default().max),
    };
    let (tx, rx) = std::sync::mpsc::channel::<EgressItem>();
    let bus = EgressBus::new(rx, encoding, policy, transport);
    Ok((node, uart, tx, bus))
}

/// Parse `mqtt://host:port` → (host, port).
fn parse_mqtt_url(url: &str) -> anyhow::Result<(String, u16)> {
    let rest = url.strip_prefix("mqtt://").unwrap_or(url);
    let (host, port) = rest
        .rsplit_once(':')
        .ok_or_else(|| anyhow::anyhow!("mqtt url needs host:port: {url}"))?;
    Ok((host.to_string(), port.parse()?))
}
```

> This introduces lazy constructors so manifest parsing never blocks on the network: change `TcpSink::connect(addr)` → keep it, but ADD `TcpSink::new(addr: String)` storing the addr and connecting on first `send` (guard with `Option<TcpStream>`); add `MqttPublisher::lazy(host, port, topic)` that defers connect+CONNACK to the first `send`. Update Task 5's impls to include these lazy constructors and the connect-on-first-send guard. The eager `connect` constructors remain for the net-tests.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p labwired-core egress_manifest`
Expected: PASS (2 tests).
Then full default gate: `cargo test -p labwired-core network::egress` → PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/core/src/world.rs crates/core/src/network/egress
git commit -m "feat(egress): manifest 'egress' interconnect wiring for UART sources"
```

---

### Task 7: End-to-end proof + docs (follow-up, may be split to its own session)

**Files:**
- Create: an environment manifest + a small subscriber under `examples/` (path to match the repo's existing example layout — inspect a sibling example first).
- Create: a blog post draft (per the repo convention that every new example gets an article — see the LabWired blog editor tooling).

**Interfaces:**
- Consumes: everything above, plus an existing runnable example (IO-Link thermal-fingerprint sensor or CANmod.gps).

- [ ] **Step 1: Inspect an existing multi-node example manifest** to copy its exact directory/format (`grep -rl "interconnects" examples/`).
- [ ] **Step 2: Add an `egress` interconnect** to a copy of that example pointing at a local broker.
- [ ] **Step 3: Write a subscriber** (a few lines: connect to the broker/socket, assert one decoded value arrives) and run it against the CLI-driven sim.
- [ ] **Step 4: Capture the run output** and confirm a real value crossed the wire.
- [ ] **Step 5: Draft the blog post** describing the demo; get review before publish (neutral framing).
- [ ] **Step 6: Commit** the example + draft.

> This task is intentionally coarse: it is a demo/integration task, not a unit-tested code path. Right-size it in its own session once Tasks 1–6 are green.

---

## Self-Review

**Spec coverage:**
- Determinism boundary (enqueue-only core, worker thread) → Tasks 2, 4. ✓
- UART source → Tasks 2, 6. ✓ CAN source → Task 4 (`add_can_source`, unit-tested); manifest CAN wiring explicitly deferred in "Scope of THIS plan". ✓
- Transports TCP/MQTT/HTTP → Task 5. ✓ Encodings raw/ndjson/frames → Task 3. ✓
- Bounded drop-oldest buffer + dropped metric → Task 4. ✓
- Manifest-declared → Task 6. ✓
- Testing split (deterministic core vs net-gated) → Tasks 1–4 default, Task 5 `net-tests`. ✓
- E2E demo + blog → Task 7. ✓
- Browser/WASM deferral → stated in Scope; not a task. ✓

**Placeholder scan:** No "TBD"/"handle edge cases" left; the two intentional "wrong" lines (Task 3 bogus import, Task 6 `world_egress_sender`) are explicitly corrected inline in their own notes. ✓

**Type consistency:** `EgressItem`, `EncodingKind`, `BufferPolicy`, `EgressTransport`, `encode`, `EgressBus::new(rx, encoding, policy, transport)`, `EgressTap::new(tx)` are used consistently across tasks. `TcpSink`/`MqttPublisher` gain lazy constructors (`TcpSink::new`, `MqttPublisher::lazy`) introduced in Task 6 and back-annotated to Task 5. ✓
