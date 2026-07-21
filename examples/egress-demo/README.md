# Egress Bridge Demo

Stream a **simulated** device's output to your **real** backend — MQTT, TCP, or
HTTP. Because LabWired boots the actual firmware on a register-accurate MCU, what
lands on your broker is the exact byte stream the physical device would send.
No hardware, no reflashing.

```
firmware TX  →  UART push_tx  →  EgressTap  →  EgressBus  →  worker thread  →  MQTT / TCP / HTTP
              (silicon-accurate)   (sim thread, enqueue-only)   (blocking I/O off the sim path)
```

The sim core only *enqueues*; a worker thread owns the socket. A slow or dead
endpoint back-pressures into a bounded drop-oldest buffer and **never stalls the
simulation**, so the *simulation run* stays deterministic and replayable. (What
reaches the wire is not itself replayable — a slow endpoint drops items, and the
bus logs a warning with the running drop count rather than losing data silently.)

## The manifest

See [`env.yaml`](./env.yaml). One `egress` interconnect taps a node's UART and
forwards it:

```yaml
interconnects:
  - type: "egress"
    nodes: ["sensor"]
    config:
      uart: "usart2"
      transport: "mqtt"                 # tcp | mqtt | http
      url: "mqtt://127.0.0.1:1883"
      topic: "labwired/sensor/uart"     # mqtt only
      encoding: "raw"                   # raw | ndjson-trace | frames-json
      buffer_max: 4096
```

`config` is a closed mapping: `uart`, `transport`, and `encoding` are optional
non-empty strings; `url` is a required non-empty string; `topic` is required
only for MQTT and invalid for TCP/HTTP; and `buffer_max` is a positive integer.
The only transport values are `tcp`, `mqtt`, and `http`; the only encodings are
`raw`, `ndjson-trace`, and `frames-json`. Unknown or mistyped keys are rejected
before the world starts.

## Point it at your own backend

| Transport | `url` | Notes |
|-----------|-------|-------|
| `tcp`  | `"127.0.0.1:9000"` | raw bytes to any TCP listener (virtual serial gateway) |
| `mqtt` | `"mqtt://broker:1883"` | MQTT 3.1.1 QoS 0 PUBLISH to `topic`; unauthenticated, no TLS/keep-alive — point at a broker you control. Prefer `ndjson-trace` (one framed message per reading) over `raw` |
| `http` | `"http://host:8080/ingest"` | HTTP/1.1 POST per flush |

Encodings: `raw` (bytes verbatim), `ndjson-trace` (one JSON line per byte/frame),
`frames-json` (CAN frames as a JSON array).

## Proof it works end-to-end

The runnable proof is a net-gated integration test that drives the real UART
`push_tx` fan-out and asserts the bytes arrive on a live localhost socket:

```bash
cargo test -p labwired-core --features net-tests --test egress_e2e
```

`crates/core/tests/egress_e2e.rs` writes `TEMP=21.5C\n` to a simulated UART's TX
register and asserts the identical bytes are received over a real `TcpStream` —
the same path a customer backend sees.

## Status / limits

- **Native path** (CLI + hosted builder) today. Browser/WASM egress needs a
  WebSocket relay (deferred — WASM has no raw sockets).
- Manifest wiring currently taps **UART** sources. CAN egress is supported and
  unit-tested at the bus layer (`EgressBus::add_can_source`); manifest CAN wiring
  is a follow-up.
