# EK-RX261

The **Renesas EK-RX261** evaluation kit (RTK5EK2610S00001BE) with an
**R5F52618BGFP**: an RX261 MCU (RXv3 core, single-precision FPU, 64 MHz,
512 KiB code flash, 128 KiB RAM). It is LabWired's **first Renesas RX part**.

LabWired runs it on its RX core (the RXv3 instruction set with the ICU in the
core), with SCI6 as the console, CMT0-3 and the SYSTEM/FLASH start-up
handshakes. **Unmodified upstream Zephyr v4.3.0** images for `ek_rx261` start
at the fixed reset vector:

| Script | Firmware | Result |
| :--- | :--- | :--- |
| `zephyr-hello.yaml` | `samples/hello_world` | banner + `Hello World! ek_rx261/r5f52618bgfp` on SCI6 |
| `zephyr-sync.yaml` | `samples/synchronization` | thread_a / thread_b alternate on CMT0 tick interrupts |
| `zephyr-kernel-common.yaml` | `tests/kernel/common` | 12 suites, 61 pass + 1 skip (atomics, irq_offload, clock, …) |

!!! warning "Not claimed yet"
    PORT/MPC pin control (no LEDs or switches), SCI interrupts (the console
    is polled), DTC, group interrupts, IRQ pins, the RXv3 register banks,
    user-mode privilege checks and FPU exceptions. SIM-DERIVED: no bench
    part.

Evidence and the full claim table: `examples/ek-rx261/VALIDATION.md`.
Rebuild the fixtures: `crates/firmware-ek-rx261-zephyr/build.sh`.
