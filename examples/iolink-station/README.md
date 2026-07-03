# IO-Link Multi-Chip Station

A real IO-Link station, simulated the way the hardware exists: a **master chip
running the real `iolinki-master` firmware** wired point-to-point to **sensor
chips, each running the real `iolinki` device firmware**. IO-Link is
point-to-point — exactly one device per master port — so N sensors are N
separate chips, each its own simulated MCU with its own address space. There is
no host-side stack and no shared device state.

This is the multi-chip counterpart to [`iolink-dido`](../iolink-dido),
which runs one device stack as firmware against a host-side master model.

## Topology

```
  master chip (master-fw, STM32L476)        sensor chips (iolink-dido device FW)
  ┌───────────────────────────┐             ┌─────────────────────────┐
  │ iolinki-master controller │  USART2 <═══╪═> USART2  iolinki device │ sensor1 (PD 0xA5)
  │                           │  USART3 <═══╪═> USART2  iolinki device │ sensor2 (PD 0x3C)
  │                           │  UART4  <═══╪═> USART2  iolinki device │ sensor3 (PD 0xC3)
  │                           │  UART5  <═══╪═> USART2  iolinki device │ sensor4 (PD 0x5A)
  └───────────────────────────┘             └─────────────────────────┘
        <═══>  = UartCrossLink interconnect (the simulated C/Q wire)
        orchestrated by labwired_core::world::World
```

Each `<═══>` is a `UartCrossLink` (`crates/core/src/network/mod.rs`) built on the
`UartStreamDevice` seam; `World::from_manifest` (`crates/core/src/world.rs`)
builds every node from its `system.yaml` + firmware ELF and wires the links from
the environment manifest. `World::step_all()` advances all chips in lockstep and
ticks the wires.

## Build the firmware (needs `arm-none-eabi-gcc`)

```bash
make -C ../iolink-dido/firmware   # device ELF (iolink_dido.elf)
make -C master-fw                        # one-port master ELF (master.elf)
make -C master-fw-4port                  # four-port master ELF (master.elf)
make -C device-fw-svc                    # service-rich device ELF (device.elf)
make -C master-fw-svc                    # service-script master ELF (master.elf)
```

ELFs are build artifacts (git-ignored). The `svc` pair needs `STM32CUBE_L4_DIR`
set (CMSIS from STM32CubeL4); `ci/build.sh` builds all of them.

## Run the proofs

```bash
# from the core crate root:
cargo test --release -p labwired-core --test world_multichip -- --nocapture
```

- `master_chip_reaches_operate_with_real_sensor_chip` — Phase 1: one master chip
  drives one real sensor chip to OPERATE and reads its process data (0xA5).
- `four_port_station_all_sensors_operate_with_distinct_pd` — Phase 2: four ports,
  four sensor chips, each preset to a distinct **palindrome** PD byte
  (`0xA5, 0x3C, 0xC3, 0x5A`, bit-order-invariant) so every port is verified to
  read its *own* sensor — proving four independent links with no cross-talk.

The tests read the master's `g_master_state` / `g_master_pd` globals over the bus
to observe progress; they skip with a message if the ELFs are not built.

## Full service coverage (`svc` pair)

`world_station_services` drives the **whole** `iolinki-master` service surface on
the wire, not just startup + cyclic PD. A `master-fw-svc` chip runs a phased
script against a service-rich `device-fw-svc` chip and proves each service after
OPERATE:

```bash
STM32CUBE_L4_DIR=$HOME/projects/STM32CubeL4 bash ci/build.sh
cargo test --release -p labwired-core --test world_station_services -- --nocapture
```

- **ISDU read** — vendor-name index `0x0010` reads back `"LABWIRED"`.
- **Cyclic PD output** — the master's PD-out byte is echoed back as the device's
  PD-in (actuator mirror).
- **Events** — a device WARNING event (`0x8CA0`) is triggered and read back via
  `read_event_details` (ISDU `0x001C`).
- **Data Storage** — an Application-Tag record is written and read back
  byte-for-byte (ISDU `0x0003`).

This test also asserts the run is **fidelity-clean**: zero undecoded
instructions and zero unmapped MMIO (`labwired_core::fidelity`). That guard
caught nothing here only because the underlying simulator gap it targets — the
Cortex-M4 `UADD8`/`SEL` SIMD instructions newlib's optimised `strlen` relies on —
was fixed first; before that fix, string ISDU reads silently returned garbage.
Set `LABWIRED_STRICT_FIDELITY=1` to make the first such gap a hard failure.

## Manifests

- `env.yaml` — 2-node (master + sensor1).
- `env4.yaml` — 5-node (master + 4 sensors), 4 `uart_cross_link`s.
- `master/system.yaml`, `sensor*/system.yaml` — per-node chip + devices.

## Scope

This is **functional** simulation evidence: real master and device firmware
exchanging real IO-Link process data over a modeled C/Q wire. It is **not**
IO-Link electrical/PHY-timing conformance. Line speed is irrelevant in the
cycle-stepped model.
