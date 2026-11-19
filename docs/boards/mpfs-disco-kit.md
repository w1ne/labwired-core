# PolarFire SoC Discovery Kit

The **Microchip PolarFire SoC Discovery Kit** (MPFS-DISCO-KIT) with an
**MPFS095T**: a PolarFire FPGA with a hardened RISC-V microprocessor subsystem
(MSS) of one E51 monitor core (RV64IMAC) and four U54 application cores
(RV64GC). It is LabWired's **first 64-bit RISC-V part**.

LabWired runs **one U54 hart** (`mhartid` 1) on its RV64 core, with the CLINT
inside the core and the five MMUARTs as 16550s. **Unmodified upstream Zephyr
v4.1.0** images for `mpfs_icicle/polarfire/u54` (the same MSS) boot from the
DDR window at `0x8000_0000`:

| Script | Firmware | Result |
| :--- | :--- | :--- |
| `zephyr-hello.yaml` | `samples/hello_world` | banner + `Hello World! mpfs_icicle/polarfire/u54` on MMUART1 |
| `zephyr-sync.yaml` | `samples/synchronization` | thread_a / thread_b alternate on CLINT timer interrupts |
| `zephyr-kernel-common.yaml` | `tests/kernel/common` | 12 suites, 63/63 cases (atomics, irq_offload, clock, …) |
| `mss-peripherals-smoke.yaml` | bare-metal `mpfs-gpio-smoke.S` | MSS GPIO_0..2 readback, PLIC source 51 claim/complete, I2C0 START/SLA+W NACK/STOP, SPI1 frame + PLIC 55 (13/13) |

!!! warning "Not claimed yet"
    F/D, S/U modes and the Sv39 MMU (so no OpenSBI or Linux), the E51 and
    U54 2-4, PMP enforcement, I2C/SPI slave mode and bit timing, SPI0, QSPI,
    the per-pin direct GPIO interrupts, the HSS boot flow and DDR training.
    `mvendorid`/`marchid`/`mimpid` read 0. SIM-DERIVED: no bench part.

Evidence and the full claim table: `examples/mpfs-disco-kit/VALIDATION.md`.
Rebuild the fixtures: `crates/firmware-mpfs-zephyr/build.sh`.
