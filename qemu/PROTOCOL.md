# LabWired MMIO bridge protocol, version 2

QEMU's `labwired-mmio-bridge` device (`patches/0001-labwired-mmio-bridge.patch`)
connects as a client to a unix stream socket. The LabWired side
(`crates/qemu-bridge`) listens on it and owns the peripheral model.

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

- One MMIO window and one IRQ line per socket.
- No DMA.
- No snapshot/migration of the model state.
