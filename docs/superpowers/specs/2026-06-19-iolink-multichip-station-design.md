# IO-Link Multi-Chip Station — Design

**Status:** approved design (brainstorm 2026-06-19), pre-plan.

## Goal

Simulate a real IO-Link station the way the hardware actually exists: a **master
chip running real `iolinki-master` firmware** wired point-to-point to **N sensor
chips, each running real `iolinki` device firmware**. No host-linked stacks, no
device-stack reentrancy — each device is its own simulated MCU with its own
address space, so the device stack's singleton globals are a non-issue.

This supersedes the host-linked native-bridge approach (`feat/iolink-real-stack`)
for the *multi-sensor* use case. That branch remains valid as a host-side
single-master proof.

## Why this architecture

IO-Link is point-to-point: exactly one device per master port. N sensors = N
ports = N device chips. The simulator already models multiple UART peripherals
per chip and runs the real device stack as firmware (the `al2205-iolink-dido`
example proves it on an STM32L476). The only missing pieces are the inter-chip
*wire* and the multi-chip *loader* — both already declared in-tree:

- `crates/core/src/world.rs` — `World`, the documented multi-node orchestrator
  (`machines` + `interconnects`, `step_all()` lockstep). `CanBus` and
  `WirelessBus` interconnects are fully implemented and tested.
  `World::from_manifest()` is a stub (`bail!`).
- `crates/core/src/network/mod.rs` — `UartCrossLink` is a named stub
  (`tick()` is empty).
- `crates/core/src/peripherals/uart.rs` — `UartStreamDevice` (`poll()` injects
  RX, `on_tx_byte()` observes TX) + `attach_stream()` is the **proven** seam the
  al2205 master peer already uses.

So the architecturally correct solution is to **finish the declared multi-node
architecture**: a first-class `UartCrossLink` interconnect built on
`UartStreamDevice` endpoints, loaded declaratively from an `EnvironmentManifest`.
The master is a peer node, not "a device attached to the sensor's UART."

## Architecture

```
        EnvironmentManifest (env.yaml)
        ├── node: master   (system + master.elf)
        ├── node: sensor1  (system + device.elf)
        ├── node: sensor2  (system + device.elf)
        └── interconnects:
            ├── uart_cross_link: master.usart1 <-> sensor1.usart2
            └── uart_cross_link: master.usart2 <-> sensor2.usart2

   World::from_manifest()  builds N Machines + wires interconnects
        step_all():  step every Machine, then tick every interconnect
```

### Components

1. **`UartCrossLink` interconnect (real).** Owns two `UartStreamDevice`
   endpoints, one attached to each node's named UART. On `tick()` it shuttles
   bytes both directions: bytes a node's firmware transmitted (captured via
   `on_tx_byte`) are delivered to the peer's RX (via the peer endpoint's
   `poll`). Point-to-point, full-duplex, byte-accurate. Baud pacing respected
   via `poll(elapsed_us)`.

2. **`World::from_manifest()` (real).** Parses the `EnvironmentManifest`: for
   each node, load its `SystemManifest` + ELF, build the correct `Machine`
   (CPU + bus + peripherals + external_devices), register it; for each
   interconnect, construct it and attach its endpoints to the named UARTs.
   Needs a `MachineTrait` seam to reach a named UART peripheral for stream
   attachment (today `MachineTrait` exposes only `step`/`read_u8`/`write_u8`).

3. **Master firmware crate (new).** Mirrors `al2205-iolink-dido/firmware`:
   `Makefile` pulling `IOLINKI_MASTER_DIR` (+ device-stack `frame.c`/`crc.c`),
   `phy_labwired.c` adapting `iolinki-master`'s PHY to the chip UART(s),
   `main.c` running the master cycle (wake-up → startup → operate, read PD).
   Phase 1: one port. Phase 2: N ports across N UARTs via the controller API
   (`iolink_master_controller_*`).

4. **Device firmware (reuse).** The `al2205-iolink-dido` device firmware already
   runs the real device stack and publishes PD. Reuse it per sensor node;
   per-sensor PD stimulus via `config_overrides` / distinct input presets.

5. **Environment manifest + example.** `core/examples/iolink-station/env.yaml`
   plus the per-node `system.yaml`s, and a README framing it as simulation
   evidence (not electrical conformance).

### Data flow (one port, per cycle)

```
master FW writes frame to USART TX
  -> UART model on_tx_byte -> master endpoint buffer
  -> UartCrossLink.tick(): master buffer -> sensor RX buffer
  -> sensor endpoint.poll() -> sensor USART RX -> device FW iolink_process()
  -> device FW writes response to its USART TX
  -> sensor endpoint on_tx_byte -> UartCrossLink.tick(): -> master RX
  -> master FW reads PD; reaches OPERATE
```

## Phasing

- **Phase 1 — 2-node proof.** Implement `UartCrossLink::tick()` +
  `World::from_manifest()` for 2 nodes + a one-port master FW. Prove
  master chip ↔ one sensor chip reach OPERATE and exchange real PD over the
  wire. Hardest unknowns (wire, loader, master FW) validated at 1× complexity.
- **Phase 2 — 4-port station.** Scale the master FW to 4 UART ports via the
  controller API; env manifest with 4 sensor nodes; assert all four reach
  OPERATE with their distinct PD. No new wire/loader work — pure scale-up.

## Testing

- Rust integration test driving `World::from_manifest()` on the 2-node env,
  stepping until the master node's firmware reports OPERATE + correct PD
  (observed via UART debug or a memory probe through `MachineTrait::read_u8`).
- Phase 2: same harness, 4 nodes, assert per-port PD.
- Existing `CanBus`/`WirelessBus` world tests must stay green (no regression to
  the interconnect framework).
- Firmware builds gated on `arm-none-eabi-gcc`; ELFs are build artifacts.

## Risks / open issues

- **ARM toolchain prerequisite.** Firmware ELFs need `arm-none-eabi-gcc`
  (now installed). CI without it can only run the wire/loader unit tests against
  prebuilt or checked-in ELFs.
- **`MachineTrait` UART seam.** Attaching a stream endpoint to a named UART
  inside a type-erased `Machine` needs a new trait method; design it minimally.
- **Lockstep timing.** `step_all()` is naive lockstep; IO-Link response timing
  must tolerate it (the device stack defaults timing-enforcement OFF, which
  helps). Baud pacing handled in endpoint `poll`.
- **Device-stack `frame.c`.** Lives in unpushed commit `aec4803`, not on the
  submodule's pinned `develop` (see reference memory). Firmware build must point
  at a device-stack checkout that has it.

## Non-goals

- Device-stack reentrancy / host-side multi-device (explicitly avoided — the
  chip-per-device model makes it unnecessary).
- A browser-runnable playground catalog lab (needs prebuilt multi-node support
  in the wasm/playground layer; out of scope here).
- IO-Link electrical/PHY-timing conformance. This is functional simulation.
