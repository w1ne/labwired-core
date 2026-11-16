# LabWired MMIO bridge protocol

QEMU's `labwired-mmio-bridge` device (`patches/0001-labwired-mmio-bridge.patch`)
connects as a client to a unix stream socket. The LabWired side
(`crates/qemu-bridge`) listens on it and owns the peripheral model.

## Message

Every message is 24 bytes, little-endian, no padding:

| bytes | field    | meaning                                    |
|-------|----------|--------------------------------------------|
| 0..4  | `type`   | message type (table below)                 |
| 4..8  | `size`   | access size in bytes (1, 2, 4, 8)          |
| 8..16 | `offset` | byte offset inside the bridged MMIO window |
| 16..24| `value`  | data (write value, read result, IRQ level) |

| type | name  | direction      | fields used                  |
|------|-------|----------------|------------------------------|
| 1    | READ  | QEMU → LabWired| size, offset                 |
| 2    | WRITE | QEMU → LabWired| size, offset, value          |
| 3    | REPLY | LabWired → QEMU| value (read result)          |
| 4    | ACK   | LabWired → QEMU| none                         |
| 5    | IRQ   | LabWired → QEMU| value: 0 = low, else high    |

## Rules

- One access in flight. A guest load sends READ and blocks the vCPU until
  REPLY arrives; a guest store sends WRITE and blocks until ACK.
- IRQ messages may arrive at any time: before the REPLY/ACK of an access
  (the QEMU side applies them in order while it waits), or between accesses
  (a main-loop fd handler applies them). IRQ is a level, not an edge.
- The LabWired side sends IRQ only when the level changes. In this spike the
  BSC model changes its level only as a result of an access, so IRQ always
  precedes the REPLY/ACK of that access.
- QEMU applies IRQ by driving GIC SPI input `irq` (device property) of the
  single `arm_gic` in the machine.
- If the socket closes, QEMU exits with an error.

## Not in this version

- TODO: time sync. The model has no notion of guest time. Every transfer
  completes inside the access that starts it; DIV/DEL/CLKT are stored only.
  A real co-simulation needs a clock message (QEMU virtual time → model) and
  model-scheduled events (model → QEMU timer).
- No DMA, no multiple windows per socket, no versioning/handshake.
