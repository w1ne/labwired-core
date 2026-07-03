---
title: "Stream your simulated firmware to a real backend"
description: "The egress bridge sends a simulated device's output to your own MQTT, TCP, or HTTP endpoint — from the exact firmware you'll flash, with no hardware."
draft: true
# Draft for the labwired.com Astro site. Review before publishing (neutral framing).
---

# Stream your simulated firmware to a real backend

"Test without hardware" usually means one of two things. Either you get a
*behavioral* fake — a script that pretends to be a Modbus slave or an MQTT node,
useful for exercising your cloud side but running none of your device code. Or you
get a register-accurate simulator that boots your real firmware but keeps its output
trapped inside the simulation, visible only in a console or a logic-analyzer view.

LabWired's egress bridge closes that gap. Your firmware boots on a register-accurate
MCU model, and its peripheral output streams out to *your* backend — an MQTT broker,
a TCP socket, an HTTP endpoint — as the exact bytes the physical device would send.
Point your existing dashboard at the simulation and watch real data arrive, before
a board exists.

## How it works

The data path is deliberately boring on the sim side and does the interesting work
off it:

```
firmware TX  →  UART push_tx  →  EgressTap  →  EgressBus  →  worker thread  →  MQTT / TCP / HTTP
              (silicon-accurate)   (enqueue only)   (blocking I/O, off the sim thread)
```

The simulated UART's transmit path already fans out to observers. The egress bridge
attaches one more: an `EgressTap` that forwards every transmitted byte onto an
in-process channel. An `EgressBus` — which the multi-node runtime ticks once per
simulation step — drains that channel, applies an encoding, and hands the payload to
a worker thread. Only the worker thread ever touches a real socket.

That separation is the whole point. Networks are non-deterministic; simulations must
not be. Because the simulation only ever *enqueues*, and a bounded drop-oldest buffer
absorbs a slow or dead endpoint, the run stays byte-for-byte reproducible no matter
what the network does. A live device on a real bus drops samples when the far side
can't keep up rather than freezing — the bridge behaves the same way, and reports how
many items it dropped.

## Declaring it

Egress is a manifest entry, not code. One `egress` interconnect taps a node's UART
and forwards it:

```yaml
interconnects:
  - type: "egress"
    nodes: ["sensor"]
    config:
      uart: "usart2"
      transport: "mqtt"                 # tcp | mqtt | http
      url: "mqtt://broker.example.io:1883"
      topic: "plant/line3/temp"
      encoding: "raw"                   # raw | ndjson-trace | frames-json
```

Three transports ship today: raw TCP (a virtual serial gateway any tool can read),
MQTT 3.1.1 publish, and HTTP POST. Three encodings: bytes verbatim, newline-delimited
JSON trace events, or CAN frames as a JSON array.

## Proving it end to end

Claims about "it reaches the wire" are easy to get wrong, so the demo is a test that
drives the real transmit path and asserts on a real socket:

```rust
let mut uart = Uart::new();
uart.attach_stream(Box::new(EgressTap::new(tx)));

for &b in b"TEMP=21.5C\n" {
    uart.write(TX_REG, b).unwrap();   // firmware writing its TX register
}
bus.tick().unwrap();                  // one sim step drains to the transport

assert_eq!(server.join().unwrap(), b"TEMP=21.5C\n");   // received over real TCP
```

No mocks in the middle: the bytes leave the simulated UART's transmit register and
arrive on a `TcpStream` a few microseconds later, exactly as a customer backend would
see them.

## Where it's going

Today the bridge runs on the native path — the CLI and the hosted builder, where real
sockets exist. Running it from the in-browser playground needs a small WebSocket relay,
since WebAssembly can't open raw sockets; that's next. CAN-frame egress works at the
bus layer already; wiring it through the manifest is a short follow-up.

The through-line: when the device under test is the real firmware on faithful silicon,
"stream it to your backend" stops being a demo prop and becomes something you can trust.
