# Egress Bridge — Design Spec

**Date:** 2026-07-03
**Branch:** `feat/egress-bridge` (off `origin/main`)
**Status:** Approved design, ready for implementation plan

## Motivation

CloudIotLab lets a *virtual device* stream out to the user's own MQTT/TCP/HTTP
backend ("route my virtual device to my dashboard"). LabWired's differentiator is
that our device is **silicon-accurate and boots the real firmware** — something a
protocol-script "virtual device" structurally cannot claim. The egress bridge lets
the exact firmware a user will flash stream real peripheral output to their real
backend: *"firmware verifiably runs — and streams to your real network."*

This extends the `SimInput`/stimuli line (PR #451): stimuli inject **in**; egress
taps **out**. Same manifest style, opposite direction.

## Scope

**In scope (MVP):**
- Egress-only (device → backend). No ingress.
- Native path only: CLI + the Europa hosted builder (where `std::net` works).
- Sources: UART/USART TX bytes, CAN/CAN-FD transmitted frames.
- Transports: TCP, MQTT (3.1.1), HTTP POST.
- Encodings: `raw`, `ndjson-trace`, `frames-json`.
- Manifest-declared endpoints (no per-lab code).

**Explicitly deferred:**
- Browser/WASM egress — WASM has no raw sockets; needs a WebSocket relay (phase 2).
- Ingress / full bidirectional bridge (external → sim via stimuli).
- Interactive raw-bus tunneling (virtual COM/CAN gateway that a tool talks *to*).

## Core principle: the determinism boundary

The deterministic sim core **never does real I/O**. It only *enqueues*; a separate
host thread drains the queue and performs the blocking socket write. Replay
determinism (the faithful-replay guarantee) is preserved because the sim thread's
behavior is independent of network timing. A slow/dead endpoint back-pressures into
a bounded buffer (drop-oldest + dropped-count metric), never stalls `Machine::run`.

## Architecture

Data flow:

```
firmware TX
  → push_tx fan-out (peripherals/uart.rs:920)         [existing, unchanged]
  → EgressTap.on_tx_byte  →  mpsc::Sender               [new, sim thread]
  → EgressBus.tick() drains (world.rs:100, via step_all) [new, sim thread]
  → mpsc → EgressTransport worker thread                [new, host thread]
  → real MQTT / TCP / HTTP
```

### Components

**1. `EgressTap`** — a `UartStreamDevice` (peripherals/uart.rs:28).
- `on_tx_byte(byte)` (contract at uart.rs:36; already invoked by the `push_tx`
  fan-out at uart.rs:920) pushes the byte into an `mpsc::Sender<u8>` (or a small
  framed message).
- Attaches through the existing `attach_uart_stream_by_id` path (world.rs:31 →
  bus/mod.rs:1746). **No core hot-path changes.**
- For CAN there is no tap: `CanBus::attach()` (network/mod.rs:69) already returns a
  `Receiver<CanFrame>` of every transmitted frame. `EgressBus` holds that receiver.

**2. `EgressBus`** — a new `Interconnect` implementor (network/mod.rs:15), sibling
to `CanBus`.
- Owns the receiver ends of all taps (UART byte channels and/or CAN frame
  receivers) plus their `EgressTransport` handles.
- `tick()` (driven for free by `World::step_all`, world.rs:100) non-blockingly
  drains all pending items, applies the configured encoding, and forwards to the
  transport's channel. Enqueue-side only; never blocks.

**3. `EgressTransport`** — a trait; each impl owns a host-side worker thread doing
the blocking writes. The sim↔transport channel is bounded.
- `TcpSink` — raw/encoded bytes to a `TcpStream`.
- `MqttPublisher` — publish to a topic (crib the 3.1.1 packet format from the
  existing in-sim broker at network/mqtt.rs).
- `HttpPoster` — batch and POST JSON on a flush interval.
- `MemoryTransport` (test fake) — collects into a `Vec` for deterministic assertions.

### Encoding

- `raw` — bytes verbatim (TCP payload or MQTT binary payload). No interpretation.
- `ndjson-trace` — one JSON line per event using the serde-ready
  `UartTraceEvent { seq, direction, byte }` (uart.rs:19).
- `frames-json` — CAN: serialize `CanFrame { id, data, extended, fd, ... }`
  (network/mod.rs:19), already `Serialize`.

### Buffering / back-pressure

Bounded channel (default cap 4096). On full: drop oldest, increment a
`dropped: u64` counter exposed as a metric. Never blocks the sim thread. Rationale:
a live device on a real network drops rather than stalls; determinism must not
depend on the endpoint keeping up.

## Manifest schema

New interconnect kind, parsed in `World::from_manifest` (world.rs:124) alongside
`uart_cross_link`:

```yaml
interconnects:
  - kind: egress
    source: { uart: usart2 }        # or { can: fdcan1 }
    transport:
      type: mqtt                     # tcp | mqtt | http
      url: mqtt://broker.acme.io:1883
      topic: plant/line3/sensor      # mqtt only
      # tcp:  host, port
      # http: url, method, flush_interval_ms
    encoding: raw                    # raw | ndjson-trace | frames-json
    buffer: { max: 4096, on_full: drop_oldest }
```

`from_manifest` builds the tap (or `CanBus::attach()`), constructs the matching
transport, spawns its worker thread, and registers the `EgressBus` via
`add_interconnect` (world.rs:86).

## Testing

- **Core/sim side (deterministic, no network):** unit-test `EgressTap` + `EgressBus`
  against `MemoryTransport`. Assert exact bytes/frames arrive in order; buffer-full
  drops oldest and bumps the counter; `tick()` never blocks. Runs in the normal
  Rust suite.
- **Transport side (real sockets, isolated):** integration tests — `TcpSink` →
  localhost listener; `MqttPublisher` → loopback broker; `HttpPoster` → local test
  server; asserting bytes-on-wire. Gated behind a `net-tests` feature/job so they
  don't flake the main gate.
- **E2E proof (demo + blog):** one real example (IO-Link thermal-fingerprint sensor
  or CANmod.gps) running in the CLI / Europa builder, egressing live to a real MQTT
  broker, with a subscriber asserting a decoded value arrives. Per convention, this
  gets a blog post.

## Key types to build against

- `UartStreamDevice` (peripherals/uart.rs:28), `on_tx_byte` (uart.rs:36)
- `attach_uart_stream_by_id` (bus/mod.rs:1746), `attach_uart_stream` (world.rs:31)
- `Interconnect` (network/mod.rs:15), `add_interconnect` (world.rs:86),
  ticked in `World::step_all` (world.rs:100)
- `CanBus::attach()` (network/mod.rs:69), `CanFrame` (network/mod.rs:19)
- `UartTraceEvent` (serde-ready, uart.rs:19)
- `World::from_manifest` (world.rs:124) for manifest wiring
- MQTT 3.1.1 packet format reference: network/mqtt.rs
