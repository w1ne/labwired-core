# LabWired MMIO bridge protocol, version 2

QEMU's `labwired-mmio-bridge` device (`patches/0001-labwired-mmio-bridge.patch`)
connects as a client to a unix stream socket. The LabWired side
(`crates/qemu-bridge`) listens on it and owns the peripheral model.

## Instances and sockets

One QEMU device per modeled peripheral, one socket per device. There is no
instance id in the protocol: the socket is the instance. `qemu-bridge plan
--dtb FILE --socket-dir DIR` prints one `-device labwired-mmio-bridge,...`
per modeled DT node, with `socket=DIR/<label>.sock` (label = the node's
/aliases name, e.g. `i2c1`). `qemu-bridge serve --dtb FILE --socket-dir DIR`
listens on the same paths, serves each socket on its own thread, and writes
one transcript per instance (`--transcript-dir`). Each instance keeps its own
model time; QEMU serializes all accesses under the BQL, so per-instance
transcripts are deterministic under `-icount`.

## Shared interrupt inputs

Several devices may name the same GIC input: every BSC on BCM2711 uses
SPI 117. The QEMU side drives that input with the OR of the levels of all
bridge devices on it (device property `irq-or`, default `on`).
`irq-or=off` (`plan --no-irq-or`) makes each device write the input with its
own level, last writer wins; that exists only as a negative control.

QEMU's own BCM2838 model also drives SPI 117: its three built-in BSCs
(BSC0/1/2 at 0xfe205000/0xfe804000/0xfe805000) feed an or-irq gate and a
split-irq into the same GIC input. The OR above does not include that gate.
It stays quiet because those built-in models only change their output when
their registers are accessed: BSC1 is shadowed by the bridge window, and
BSC0/BSC2 are disabled in the Pi 4 DT, so the guest never touches them. A DT
that enables a QEMU built-in BSC next to a bridged one would have two
writers on SPI 117 again.

## Time

QEMU is the only clock. Every request carries `t_ns`, the
`QEMU_CLOCK_VIRTUAL` time at which QEMU sends it. Before LabWired handles a
request it advances its model to `t_ns`; it never runs ahead of the time it
was given. `t_ns` never decreases; LabWired closes the connection if it does.

Run QEMU with `-icount shift=N,sleep=off,align=off`. The virtual clock is
then a function of the guest instruction count, and a run is repeatable to
the nanosecond. Without `-icount` the protocol still works, but virtual time
follows the host clock and two runs differ.

## Exchanges

QEMU starts every exchange. Each request gets exactly one reply. LabWired
never sends anything unsolicited.

Request, QEMU → LabWired, 32 bytes, little-endian:

| bytes  | field    | meaning                                          |
|--------|----------|--------------------------------------------------|
| 0..4   | `type`   | 0 HELLO, 1 READ, 2 WRITE, 3 SYNC                 |
| 4..8   | `size`   | access size in bytes (1, 2, 4, 8); 0 otherwise   |
| 8..16  | `t_ns`   | QEMU_CLOCK_VIRTUAL time, ns                      |
| 16..24 | `offset` | byte offset in the bridged MMIO window           |
| 24..32 | `value`  | WRITE: data. HELLO: protocol version (2)         |

Reply, LabWired → QEMU, 24 bytes, little-endian:

| bytes  | field              | meaning                                       |
|--------|--------------------|-----------------------------------------------|
| 0..8   | `value`            | READ: register value. HELLO: version. Else 0  |
| 8..12  | `irq_level`        | level of the interrupt line after the request |
| 12..16 | reserved           | 0                                             |
| 16..24 | `next_deadline_ns` | next model state change; `u64::MAX` = none    |

| type  | when QEMU sends it                              |
|-------|-------------------------------------------------|
| HELLO | once, when the device realizes. Version check. |
| READ  | guest load in the window. The vCPU waits.       |
| WRITE | guest store in the window. The vCPU waits.      |
| SYNC  | the deadline timer fired                        |

After every reply except HELLO, QEMU:

1. drives the GIC SPI input `irq` (device property) to `irq_level`;
2. arms its single `QEMU_CLOCK_VIRTUAL` timer at `next_deadline_ns`, or
   deletes it if the value is `u64::MAX`.

When the timer fires, QEMU sends SYNC at the current virtual time and
applies its reply. This is how a model event that no guest access observes
(for example DONE at the end of an I²C transfer) raises the interrupt at the
right virtual time. Under `-icount` the timer fires exactly at the deadline.

## Transcript

`qemu-bridge --transcript FILE` writes one line per request:

```
<t_ns> <HELLO|READ|WRITE|SYNC> <offset> <value> <irq_level> <next_deadline_ns|none>
```

`value` is the read result for READ and the written data for WRITE.

## Changes from version 1

- Every request carries `t_ns`; SYNC is new.
- Replies carry `irq_level` and `next_deadline_ns`.
- The asynchronous IRQ message and the main-loop fd handler are gone.
- HELLO checks the version.

## Not in this version

- One MMIO window and one IRQ line per socket (and per device).
- No DMA.
- No snapshot/migration of the model state.
