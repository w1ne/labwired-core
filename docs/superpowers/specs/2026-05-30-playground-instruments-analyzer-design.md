# Playground Instruments — Universal Logic/Protocol Analyzer Design

**Status:** Draft
**Date:** 2026-05-30
**Author:** Andrii Shylenko

## 1. Problem & Goals

LabWired can run firmware deterministically and (as of the nRF BLE work) route
radio traffic between simulated chips, but there is no way to *observe bus
traffic* the way a hardware engineer uses a Saleae logic analyzer. The original
ask — "drop two nRF sensors and watch them talk over Bluetooth" — is really a
special case of a missing general capability: a **universal analyzer** that
captures wire-level activity and decodes it into protocol transactions.

**Goal:** a reusable *instrument* — first member of a playground **toolset** —
that captures channels (physical pins and virtual transports) and decodes them
into a normalized transaction stream, usable from the CLI/API first and rendered
in the playground second.

**Non-goals (v1):** analog/scope capture; SPI/CAN/USB/1-Wire decoders (roadmap,
not v1); editing/injecting traffic (read-only sniffing); a generic floating-panel
window manager beyond what one Analyzer needs.

## 2. Background & Constraints

- **Positioning (memory `project_labwired_positioning`):** LabWired is the
  *grounding layer for AI agents*; the moat is **verified peripheral blocks**,
  not CPU speed. Decoders are exactly such blocks → they belong in the Rust core,
  deterministic and CI-testable, not in throwaway JS.
- **Clean architecture (memory `feedback_labwired_clean_architecture`):** no
  JS↔wasm callbacks per cycle; one execution path. Capture must be a passive
  in-engine ring polled by `sinceSeq`, never a per-cycle callback into JS.
- **CLI/API core (memory `project_labwired`):** the analyzer is an API/CLI
  capability first; the playground is one consumer of the same transaction
  stream. An AI agent running headless must get identical decoded output.
- **Existing trace:** `radio.rs` already exposes `virtual_air_trace_snapshot()`
  and `clear_virtual_air()` — proven end-to-end (sim + real nRF parity, this
  session). BLE enters the new model as a *virtual channel* reusing this.
- **Multi-chip:** `ChipsProvider` runs N `WasmSimulator` bridges sharing the
  process-static virtual air. Packets cross chips, so the Analyzer instrument is
  **chip-independent** (not bound to the active chip's drawer).

## 3. Design Overview

Three layers with strict separation:

```
ENGINE (Rust)
  1. CAPTURE — ProbeBus: bounded ring of Sample{ t_cycles, channel, level }
       • physical pin  → push on GPIO level change (only if subscribed)
       • virtual xport → BLE air frame pushed as a channel event
  2. DECODE  — pure fn(channel samples) -> Transaction{ bus, fields, payload, ok }
       v1 decoders: BLE, UART, I2C   (deterministic, unit-tested)
EXPOSE (WASM + CLI)
       probe_samples(sinceSeq)        -> raw edges (waveform)
       probe_transactions(sinceSeq)   -> decoded rows
       CLI: `labwired sniff` streams NDJSON transactions
PLAYGROUND (React) — one CONSUMER
  3. INSTRUMENT HOST — dockable, chip-independent panels
       └ ANALYZER  ▸ Waveform lanes (pins)  ▸ Protocol rows (txns)
                   ▸ bus/channel filters    ▸ channel→decoder binding
```

**Channel model — the key abstraction.** A channel is
`PhysicalPin(chip, pin) | VirtualTransport(kind)`. Pins yield digital edges and
render as waveforms; BLE is radio, not a wire, so it enters as a virtual channel
and renders as a frame lane (rows only, no waveform). This keeps "universal"
honest: anything on a wire — *including bit-banged protocols* — is capturable,
while radio still fits the same transaction schema.

## 4. Detailed Design

### 4.1 Capture — `ProbeBus` (core)

```rust
pub struct Sample { pub t_cycles: u64, pub channel: ChannelId, pub level: u8 }
pub enum ChannelId { Pin { chip: u16, pin: u16 }, Virtual(VirtualKind) }
pub enum VirtualKind { BleAir }

pub struct ProbeBus {
    ring: Vec<Sample>,   // preallocated, drop-oldest
    head: usize, seq: u64,
    subscribed: ChannelMask,   // capture nothing unless a channel is armed
    dropped: u64,              // count of overwritten-before-read samples
}
```

- **Opt-in:** with no subscribed channels, GPIO write paths take a single
  `mask.is_empty()` branch → zero cost when the Analyzer is closed.
- **Edge-only:** record a pin sample only when its level changes.
- **Bounded:** drop-oldest ring; `dropped` surfaced to the UI as an explicit
  "N samples dropped" marker — never silent truncation.
- **Time in cycles** (not wallclock) → reproducible, CI-stable captures.
- BLE: `radio.rs` pushes one `Sample`/frame into `VirtualKind::BleAir` carrying a
  frame handle; the air-frame payload stays in the existing air trace, indexed by
  seq, so we don't duplicate buffers.

### 4.2 Decode — decoders (core)

```rust
pub struct Transaction {
    pub t_cycles: u64, pub bus: Bus,
    pub fields: Vec<(String, String)>,  // e.g. addr=0x68, rw=W, ack=true
    pub payload: Vec<u8>, pub ok: bool, // crc/ack/framing valid
}
pub trait Decoder { fn feed(&mut self, s: &Sample) -> Option<Transaction>; }
```

- **BLE** (v1, trivial): air frame → `addr`, `len`, `crc_ok`, payload. Reuses the
  proven path; this is the vertical-slice decoder.
- **UART** (week 3): one data line + configured baud → start/data/stop framing →
  byte; `fields: parity/framing ok`. Two channels (TX, RX) = two decoder
  instances.
- **I2C** (week 3): SDA+SCL → START/STOP detection, 7-bit addr + R/W, per-byte
  ACK/NACK, data. Classic edge-driven state machine.
- Decoders are pure over their input samples → unit-testable against captured
  fixtures with zero hardware.

### 4.3 Expose — WASM + CLI

- WASM: `probe_subscribe(channels)`, `probe_samples(sinceSeq) -> Sample[]`,
  `probe_transactions(sinceSeq) -> Transaction[]`. Incremental by `seq`.
- CLI (API-first surface): `labwired sniff --bus ble|uart|i2c [--sda P0.26
  --scl P0.27 | --line PA9 --baud 115200] firmware.elf` → NDJSON:
  ```json
  {"t":12840,"bus":"uart","dir":"tx","payload":"48656c6c6f","ascii":"Hello"}
  {"t":20100,"bus":"i2c","addr":"0x68","rw":"W","payload":"75","ok":true}
  {"t":31002,"bus":"ble","addr":"0xBExxBABE","len":4,"payload":"03","ok":true}
  ```
  The playground Analyzer renders this exact stream — no second decode path.

### 4.4 Instrument Host + Analyzer (playground — secondary)

- **Host:** a registry of instruments rendered as dockable panels, independent of
  the per-chip properties drawer (extends the floating-window direction in memory
  `project_labwired_playground_windows`). v1 ships exactly one instrument.
- **Analyzer panel:** Waveform lanes (digital pins, time = cycles) + Protocol
  rows (decoded txns) + bus/channel filter + a channel→decoder binding UI.
- **Demo wiring:** add two *distinct* nRF board entries (`nrf52840-ble-sensor`,
  `nrf52840-ble-collector`) in `bundled-configs.ts` pointing at the two ELFs;
  drop both (no change to the duplicate-board rule); Analyzer BLE bus shows
  frames flowing sensor→collector live. ELFs built+copied via `build-firmware.sh`.

## 5. Testing

- **Decoders:** Rust unit tests over hand-authored `Sample` fixtures (known
  I2C/UART/BLE captures → expected `Transaction`s). This is the moat — highest
  coverage here.
- **Capture:** ring bounds, drop-oldest + `dropped` counter, opt-in no-op when
  unsubscribed (assert zero samples), edge-only dedup.
- **CLI:** golden NDJSON for the two-nRF firmware (deterministic by cycle).
- **Playground:** e2e — drop two nRFs, open Analyzer, assert BLE rows advance and
  payload increments (mirrors the hardware watchpoint result from this session).

## 6. Rollout & Risks

**Build order — BLE slice first (chosen):**
1. *Week 1 (visible):* Instrument Host + Analyzer + BLE decoder over existing
   air-trace + two-nRF demo wiring + `labwired sniff --bus ble`.
2. *Week 2:* `ProbeBus` pin-edge capture + WASM `probe_samples` + waveform lanes.
3. *Week 3:* UART + I2C decoders + CLI flags + channel-binding UI.

**Protocol roadmap (popularity × onboarding cost over our edge substrate):**

| Protocol | Ubiquity | Lines/source | Decoder cost | Phase |
|---|---|---|---|---|
| BLE | high | radio (air-trace exists) | trivial (done path) | v1 |
| UART | very high | TX/RX + baud | low | v1 |
| I2C | very high (sensors) | SDA/SCL | low–med | v1 |
| SPI | high (display/flash) | CLK/MOSI/MISO/CS + CPOL/CPHA | low–med | v2 |
| 1-Wire | med | single line, timing | med | v2 |
| CAN | med (auto/industrial) | bit-stuff + arb + CRC | med–high | v3 |
| Modbus/RS-485 | med (industrial) | UART + framing layer | med (reuses UART) | v3 |
| USB | high but complex | NRZI/diff/enumeration | high | later |

**Risks:** (a) capture perf — mitigated by opt-in mask + edge-only + no per-cycle
JS callback; (b) memory — bounded ring with explicit drop marker; (c) scope
creep across instruments — v1 ships one instrument only; (d) determinism — all
timestamps in cycles.

## 7. Open Questions

- Decode location: Rust-only (chosen, for CI/agent parity) — confirm we never
  need a JS fast-path for huge captures (likely fine; rows are sparse).
- Should `labwired sniff` attach to a *running* sim session or only spawn one
  from an ELF? (Lean: spawn-from-ELF in v1, attach later.)
- Waveform time axis: raw cycles vs. derived µs (needs per-chip clock) — default
  cycles in v1.
